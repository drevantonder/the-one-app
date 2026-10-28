// Package bucket keeps every device's log in S3-compatible storage, encrypted
// on the device before upload, so a node can be rebuilt from the bucket alone
// and devices that are never online together still exchange entries.
//
// Layout: logs/<device>/<first>-<last>.seg holds that device's entries
// first..last (gzipped JSON lines, then XChaCha20-Poly1305). Segments may
// overlap; readers skip what they already have.
package bucket

import (
	"bytes"
	"compress/gzip"
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"golang.org/x/crypto/chacha20poly1305"

	"github.com/drevantonder/the-one-app/internal/core"
	"github.com/drevantonder/the-one-app/internal/store"
	"github.com/drevantonder/the-one-app/protocol"
)

// Config locates the bucket. It is device-local and shared between a user's
// devices over the tailnet, never through the log.
type Config struct {
	Endpoint        string `json:"endpoint" jsonschema:"S3 endpoint URL, e.g. https://<account>.r2.cloudflarestorage.com"`
	Bucket          string `json:"bucket" jsonschema:"bucket name"`
	Region          string `json:"region,omitempty" jsonschema:"region; auto for R2"`
	AccessKeyID     string `json:"accessKeyId" jsonschema:"S3 access key id"`
	SecretAccessKey string `json:"secretAccessKey" jsonschema:"S3 secret access key"`
}

// Kit is everything a device needs to use the bucket: where it is and the
// key. Keep a copy somewhere safe (e.g. a password manager) to recover if
// every device is lost.
type Kit struct {
	Config Config `json:"config"`
	Key    string `json:"key"` // base64, 32 bytes
}

const (
	metaConfig = "bucket_config"
	metaKey    = "bucket_key"
	checkKey   = "key.check"
	checkText  = "the-one-app key check"
)

type Bucket struct {
	node  *core.Node
	store *store.Store
	log   *slog.Logger

	mu        sync.Mutex
	cfg       *Config
	key       []byte
	client    *s3.Client
	remoteMax map[string]int64 // highest seq per device known to be in the bucket
	lastErr   string
	lastUp    time.Time
	lastDown  time.Time
	wake      chan struct{}
	leases    leases
}

func New(node *core.Node, st *store.Store, log *slog.Logger) *Bucket {
	return &Bucket{node: node, store: st, log: log, remoteMax: map[string]int64{}, wake: make(chan struct{}, 1)}
}

// Start loads any saved configuration, registers the bucket operations and
// runs the upload and download loops.
func (b *Bucket) Start(ctx context.Context) error {
	if raw, _ := b.store.Meta(metaConfig); raw != "" {
		var k Kit
		if err := json.Unmarshal([]byte(raw), &k.Config); err != nil {
			return err
		}
		k.Key, _ = b.store.Meta(metaKey)
		if err := b.use(k); err != nil {
			b.log.Error("bucket config unusable", "err", err)
		}
	}
	b.node.AddCoreOp("bucket_status", "Show whether this device backs up to a bucket, and when it last synced.", core.SchemaFor[struct{}](), true,
		func(context.Context, json.RawMessage, protocol.Caller) (protocol.Result, error) {
			return b.statusResult(), nil
		})
	b.node.AddCoreOp("bucket_configure", "Back this device up to an S3-compatible bucket. A new key is made unless the bucket already has one, in which case use bucket_import with the kit instead.", core.SchemaFor[Config](), false,
		func(ctx context.Context, raw json.RawMessage, _ protocol.Caller) (protocol.Result, error) {
			var c Config
			if err := json.Unmarshal(raw, &c); err != nil {
				return protocol.Result{}, err
			}
			return b.configure(ctx, c)
		})
	b.node.AddCoreOp("bucket_kit", "Print the recovery kit: the bucket location and its key. Store it in a password manager; anyone with it can read every device's data.", core.SchemaFor[struct{}](), true,
		func(context.Context, json.RawMessage, protocol.Caller) (protocol.Result, error) {
			k, ok := b.Kit()
			if !ok {
				return protocol.Result{}, errors.New("no bucket configured")
			}
			raw, _ := json.Marshal(k)
			return protocol.Result{Text: base64.RawURLEncoding.EncodeToString(raw)}, nil
		})
	type importIn struct {
		Kit string `json:"kit" jsonschema:"a recovery kit printed by bucket_kit"`
	}
	b.node.AddCoreOp("bucket_import", "Use a bucket from a recovery kit, e.g. to rebuild a device from scratch.", core.SchemaFor[importIn](), false,
		func(ctx context.Context, raw json.RawMessage, _ protocol.Caller) (protocol.Result, error) {
			var in importIn
			if err := json.Unmarshal(raw, &in); err != nil {
				return protocol.Result{}, err
			}
			k, err := DecodeKit(in.Kit)
			if err != nil {
				return protocol.Result{}, err
			}
			if err := b.Adopt(ctx, k); err != nil {
				return protocol.Result{}, err
			}
			return protocol.Result{Text: "Using bucket " + k.Config.Bucket + ". Downloading every device's log now."}, nil
		})
	go b.loop(ctx)
	return nil
}

// Routes serves this device's kit to the user's other devices (the mesh
// listener only admits the same Tailscale user).
func (b *Bucket) Routes(mux *http.ServeMux) {
	mux.HandleFunc("GET /v1/sync/kit", func(w http.ResponseWriter, _ *http.Request) {
		k, ok := b.Kit()
		if !ok {
			http.Error(w, "no bucket", http.StatusNotFound)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(k)
	})
}

// AdoptFromPeer takes the bucket kit from a peer if this device has none.
func (b *Bucket) AdoptFromPeer(ctx context.Context, base string) {
	if b.Configured() {
		return
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+"/v1/sync/kit", nil)
	if err != nil {
		return
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return
	}
	var k Kit
	if json.NewDecoder(resp.Body).Decode(&k) != nil {
		return
	}
	if err := b.Adopt(ctx, k); err != nil {
		b.log.Warn("could not adopt a peer's bucket", "err", err)
		return
	}
	b.log.Info("adopted bucket from peer", "peer", base)
}

func DecodeKit(s string) (Kit, error) {
	var k Kit
	raw, err := base64.RawURLEncoding.DecodeString(strings.TrimSpace(s))
	if err != nil {
		return k, errors.New("not a recovery kit")
	}
	if err := json.Unmarshal(raw, &k); err != nil {
		return k, errors.New("not a recovery kit")
	}
	return k, nil
}

// Kit returns the current configuration and key, if any.
func (b *Bucket) Kit() (Kit, bool) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.cfg == nil {
		return Kit{}, false
	}
	return Kit{Config: *b.cfg, Key: base64.StdEncoding.EncodeToString(b.key)}, true
}

// Configured reports whether this device uses a bucket.
func (b *Bucket) Configured() bool {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.cfg != nil
}

func (b *Bucket) configure(ctx context.Context, c Config) (protocol.Result, error) {
	key := make([]byte, chacha20poly1305.KeySize)
	if _, err := rand.Read(key); err != nil {
		return protocol.Result{}, err
	}
	if existing, ok := b.Kit(); ok && existing.Config.Bucket == c.Bucket && existing.Config.Endpoint == c.Endpoint {
		key, _ = base64.StdEncoding.DecodeString(existing.Key) // reconfiguring the same bucket keeps the key
	}
	if err := b.Adopt(ctx, Kit{Config: c, Key: base64.StdEncoding.EncodeToString(key)}); err != nil {
		return protocol.Result{}, err
	}
	return protocol.Result{Text: fmt.Sprintf("Backing up to %s. Save the recovery kit (one bucket kit) in a password manager: without it, the bucket can't be read if every device is lost.", c.Bucket)}, nil
}

// Adopt switches to a kit after checking the key matches the bucket's.
func (b *Bucket) Adopt(ctx context.Context, k Kit) error {
	if err := b.use(k); err != nil {
		return err
	}
	if err := b.verifyKey(ctx); err != nil {
		b.mu.Lock()
		b.cfg, b.client = nil, nil
		b.mu.Unlock()
		return err
	}
	raw, _ := json.Marshal(k.Config)
	if err := b.store.SetMeta(metaConfig, string(raw)); err != nil {
		return err
	}
	if err := b.store.SetMeta(metaKey, k.Key); err != nil {
		return err
	}
	b.log.Info("bucket configured", "bucket", k.Config.Bucket)
	b.poke()
	return nil
}

func (b *Bucket) use(k Kit) error {
	key, err := base64.StdEncoding.DecodeString(k.Key)
	if err != nil || len(key) != chacha20poly1305.KeySize {
		return errors.New("bad key in kit")
	}
	c := k.Config
	if c.Endpoint == "" || c.Bucket == "" || c.AccessKeyID == "" || c.SecretAccessKey == "" {
		return errors.New("endpoint, bucket, accessKeyId and secretAccessKey are required")
	}
	if c.Region == "" {
		c.Region = "auto"
	}
	client := s3.New(s3.Options{
		BaseEndpoint: aws.String(c.Endpoint),
		Region:       c.Region,
		UsePathStyle: true,
		Credentials:  credentials.NewStaticCredentialsProvider(c.AccessKeyID, c.SecretAccessKey, ""),
	})
	b.mu.Lock()
	b.cfg, b.key, b.client = &c, key, client
	b.remoteMax = map[string]int64{}
	b.mu.Unlock()
	return nil
}

// verifyKey makes sure every device uses the same key: the first device to
// use the bucket writes an encrypted check object; the rest must open it.
func (b *Bucket) verifyKey(ctx context.Context) error {
	client, cfg := b.conn()
	body, err := b.seal([]byte(checkText))
	if err != nil {
		return err
	}
	_, err = client.PutObject(ctx, &s3.PutObjectInput{Bucket: &cfg.Bucket, Key: aws.String(checkKey), Body: bytes.NewReader(body), IfNoneMatch: aws.String("*")})
	if err == nil {
		return nil
	}
	if !isPreconditionFailed(err) {
		return fmt.Errorf("bucket not writable: %w", err)
	}
	got, err := b.get(ctx, checkKey)
	if err != nil {
		return err
	}
	plain, err := b.open(got)
	if err != nil || string(plain) != checkText {
		return errors.New("this bucket was set up with a different key; import that recovery kit with bucket_import instead")
	}
	return nil
}

func (b *Bucket) conn() (*s3.Client, Config) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.cfg == nil {
		return nil, Config{}
	}
	return b.client, *b.cfg
}

func (b *Bucket) poke() {
	select {
	case b.wake <- struct{}{}:
	default:
	}
}

func (b *Bucket) loop(ctx context.Context) {
	upload := time.NewTicker(5 * time.Second)
	download := time.NewTicker(time.Minute)
	defer upload.Stop()
	defer download.Stop()
	var lastVec map[string]int64
	for {
		full := false
		select {
		case <-ctx.Done():
			return
		case <-b.wake:
			full = true
		case <-download.C:
			full = true
		case <-upload.C:
		}
		if !b.Configured() {
			continue
		}
		var err error
		if full {
			err = b.downloadAll(ctx)
		}
		if err == nil {
			vec, verr := b.store.Vector()
			if verr == nil && !equal(vec, lastVec) {
				err = b.upload(ctx, vec)
				if err == nil {
					lastVec = vec
				}
			}
		}
		b.mu.Lock()
		if err != nil {
			b.lastErr = err.Error()
		} else {
			b.lastErr = ""
		}
		b.mu.Unlock()
		if err != nil {
			b.log.Warn("bucket sync", "err", err)
		}
	}
}

func equal(a, b map[string]int64) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if b[k] != v {
			return false
		}
	}
	return true
}

type segment struct {
	device     string
	first, end int64
	key        string
}

func (b *Bucket) list(ctx context.Context) (map[string][]segment, error) {
	client, cfg := b.conn()
	out := map[string][]segment{}
	p := s3.NewListObjectsV2Paginator(client, &s3.ListObjectsV2Input{Bucket: &cfg.Bucket, Prefix: aws.String("logs/")})
	for p.HasMorePages() {
		page, err := p.NextPage(ctx)
		if err != nil {
			return nil, err
		}
		for _, o := range page.Contents {
			var s segment
			parts := strings.Split(strings.TrimSuffix(aws.ToString(o.Key), ".seg"), "/")
			if len(parts) != 3 {
				continue
			}
			if _, err := fmt.Sscanf(parts[2], "%d-%d", &s.first, &s.end); err != nil {
				continue
			}
			s.device, s.key = parts[1], aws.ToString(o.Key)
			out[s.device] = append(out[s.device], s)
		}
	}
	for d := range out {
		sort.Slice(out[d], func(i, j int) bool { return out[d][i].first < out[d][j].first })
	}
	return out, nil
}

// downloadAll pulls every segment holding entries this node lacks, then
// notes how far the bucket has each device's log.
func (b *Bucket) downloadAll(ctx context.Context) error {
	segs, err := b.list(ctx)
	if err != nil {
		return err
	}
	remote := map[string]int64{}
	got := 0
	for device, list := range segs {
		for _, s := range list {
			remote[device] = max(remote[device], s.end)
			vec, err := b.store.Vector()
			if err != nil {
				return err
			}
			if s.end <= vec[device] || s.first > vec[device]+1 {
				continue
			}
			raw, err := b.get(ctx, s.key)
			if err != nil {
				return err
			}
			entries, err := b.decode(raw)
			if err != nil {
				return fmt.Errorf("%s: %w", s.key, err)
			}
			n, err := b.node.Ingest(ctx, entries)
			if err != nil {
				return err
			}
			got += n
		}
	}
	b.mu.Lock()
	b.remoteMax = remote
	b.lastDown = time.Now()
	b.mu.Unlock()
	if got > 0 {
		b.log.Info("restored from bucket", "entries", got)
	}
	return nil
}

// upload writes every entry the bucket lacks, for any device this node
// holds entries of: each device's own entries, and backfill for others.
func (b *Bucket) upload(ctx context.Context, vec map[string]int64) error {
	b.mu.Lock()
	remote := map[string]int64{}
	for k, v := range b.remoteMax {
		remote[k] = v
	}
	listed := !b.lastDown.IsZero()
	b.mu.Unlock()
	if !listed {
		return nil // wait for the first listing, so we know what is there
	}
	client, cfg := b.conn()
	for device, have := range vec {
		for remote[device] < have {
			entries, err := b.store.DeviceEntries(device, remote[device], 5000)
			if err != nil || len(entries) == 0 {
				return err
			}
			body, err := b.encode(entries)
			if err != nil {
				return err
			}
			first, last := entries[0].Seq, entries[len(entries)-1].Seq
			key := fmt.Sprintf("logs/%s/%012d-%012d.seg", device, first, last)
			if _, err := client.PutObject(ctx, &s3.PutObjectInput{Bucket: &cfg.Bucket, Key: &key, Body: bytes.NewReader(body)}); err != nil {
				return err
			}
			remote[device] = last
			b.mu.Lock()
			b.remoteMax[device] = max(b.remoteMax[device], last)
			b.lastUp = time.Now()
			b.mu.Unlock()
		}
	}
	return nil
}

func (b *Bucket) get(ctx context.Context, key string) ([]byte, error) {
	client, cfg := b.conn()
	out, err := client.GetObject(ctx, &s3.GetObjectInput{Bucket: &cfg.Bucket, Key: &key})
	if err != nil {
		return nil, err
	}
	defer out.Body.Close()
	return io.ReadAll(out.Body)
}

func (b *Bucket) seal(plain []byte) ([]byte, error) {
	b.mu.Lock()
	aead, err := chacha20poly1305.NewX(b.key)
	b.mu.Unlock()
	if err != nil {
		return nil, err
	}
	nonce := make([]byte, aead.NonceSize(), aead.NonceSize()+len(plain)+aead.Overhead())
	if _, err := rand.Read(nonce); err != nil {
		return nil, err
	}
	return aead.Seal(nonce, nonce, plain, nil), nil
}

func (b *Bucket) open(sealed []byte) ([]byte, error) {
	b.mu.Lock()
	aead, err := chacha20poly1305.NewX(b.key)
	b.mu.Unlock()
	if err != nil {
		return nil, err
	}
	if len(sealed) < aead.NonceSize() {
		return nil, errors.New("segment too short")
	}
	return aead.Open(nil, sealed[:aead.NonceSize()], sealed[aead.NonceSize():], nil)
}

func (b *Bucket) encode(entries []protocol.Entry) ([]byte, error) {
	var buf bytes.Buffer
	zw := gzip.NewWriter(&buf)
	enc := json.NewEncoder(zw)
	for _, e := range entries {
		if err := enc.Encode(e); err != nil {
			return nil, err
		}
	}
	if err := zw.Close(); err != nil {
		return nil, err
	}
	return b.seal(buf.Bytes())
}

func (b *Bucket) decode(sealed []byte) ([]protocol.Entry, error) {
	plain, err := b.open(sealed)
	if err != nil {
		return nil, errors.New("cannot decrypt: wrong key")
	}
	zr, err := gzip.NewReader(bytes.NewReader(plain))
	if err != nil {
		return nil, err
	}
	dec := json.NewDecoder(zr)
	var out []protocol.Entry
	for {
		var e protocol.Entry
		if err := dec.Decode(&e); err == io.EOF {
			return out, nil
		} else if err != nil {
			return nil, err
		}
		out = append(out, e)
	}
}

func (b *Bucket) statusResult() protocol.Result {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.cfg == nil {
		return protocol.Result{Text: "No bucket. Configure one with bucket_configure (or `one bucket setup-r2 <name>`)."}
	}
	stamp := func(t time.Time) string {
		if t.IsZero() {
			return "never"
		}
		return t.Format(time.RFC3339)
	}
	text := fmt.Sprintf("Bucket %s at %s\nlast upload: %s\nlast download: %s", b.cfg.Bucket, b.cfg.Endpoint, stamp(b.lastUp), stamp(b.lastDown))
	if b.lastErr != "" {
		text += "\nerror: " + b.lastErr
	}
	return protocol.Result{Text: text}
}

func isPreconditionFailed(err error) bool {
	var re interface{ HTTPStatusCode() int }
	if errors.As(err, &re) {
		return re.HTTPStatusCode() == http.StatusPreconditionFailed || re.HTTPStatusCode() == http.StatusConflict
	}
	return false
}

// PutBlob stores an encrypted blob, e.g. a plugin binary.
func (b *Bucket) PutBlob(ctx context.Context, key string, data []byte) error {
	client, cfg := b.conn()
	if client == nil {
		return errors.New("no bucket configured: publishing needs one")
	}
	sealed, err := b.seal(data)
	if err != nil {
		return err
	}
	_, err = client.PutObject(ctx, &s3.PutObjectInput{Bucket: &cfg.Bucket, Key: aws.String("blobs/" + key), Body: bytes.NewReader(sealed)})
	return err
}

// GetBlob fetches and decrypts a blob.
func (b *Bucket) GetBlob(ctx context.Context, key string) ([]byte, error) {
	if !b.Configured() {
		return nil, errors.New("no bucket configured")
	}
	raw, err := b.get(ctx, "blobs/"+key)
	if err != nil {
		return nil, err
	}
	return b.open(raw)
}
