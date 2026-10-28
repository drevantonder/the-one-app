package bucket

import (
	"bufio"
	"context"
	"crypto/cipher"
	"crypto/rand"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"golang.org/x/crypto/chacha20poly1305"

	"github.com/drevantonder/the-one-app/protocol"
)

// Plugins' files live under files/, encrypted in chunks so a file of any
// size streams through without being held in memory.
//
// Format: a random 16-byte nonce prefix, then records of a 4-byte length
// and a sealed chunk. Chunk i's nonce is the prefix and i; the last chunk
// is sealed with additional data 1 (the others 0), so a truncated file
// fails to open.

const (
	filesPrefix = "files/"
	chunkSize   = 1 << 20
)

func (b *Bucket) aead() (cipher.AEAD, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.key == nil {
		return nil, errors.New("no bucket configured")
	}
	return chacha20poly1305.NewX(b.key)
}

func sealStream(aead cipher.AEAD, r *bufio.Reader, w io.Writer) error {
	prefix := make([]byte, 16)
	if _, err := rand.Read(prefix); err != nil {
		return err
	}
	if _, err := w.Write(prefix); err != nil {
		return err
	}
	nonce := make([]byte, chacha20poly1305.NonceSizeX)
	copy(nonce, prefix)
	buf := make([]byte, chunkSize)
	for i := uint64(0); ; i++ {
		n, err := io.ReadFull(r, buf)
		if err != nil && err != io.EOF && err != io.ErrUnexpectedEOF {
			return err
		}
		last := err != nil
		if !last {
			_, perr := r.Peek(1)
			last = perr == io.EOF
		}
		binary.BigEndian.PutUint64(nonce[16:], i)
		ad := []byte{0}
		if last {
			ad[0] = 1
		}
		sealed := aead.Seal(nil, nonce, buf[:n], ad)
		var size [4]byte
		binary.BigEndian.PutUint32(size[:], uint32(len(sealed)))
		if _, err := w.Write(size[:]); err != nil {
			return err
		}
		if _, err := w.Write(sealed); err != nil {
			return err
		}
		if last {
			return nil
		}
	}
}

func openStream(aead cipher.AEAD, r io.Reader, w io.Writer) error {
	nonce := make([]byte, chacha20poly1305.NonceSizeX)
	if _, err := io.ReadFull(r, nonce[:16]); err != nil {
		return fmt.Errorf("file too short: %w", err)
	}
	for i := uint64(0); ; i++ {
		var size [4]byte
		if _, err := io.ReadFull(r, size[:]); err != nil {
			return errors.New("file is truncated")
		}
		n := binary.BigEndian.Uint32(size[:])
		if n > chunkSize+uint32(aead.Overhead()) {
			return errors.New("file is corrupt")
		}
		sealed := make([]byte, n)
		if _, err := io.ReadFull(r, sealed); err != nil {
			return errors.New("file is truncated")
		}
		binary.BigEndian.PutUint64(nonce[16:], i)
		plain, err := aead.Open(nil, nonce, sealed, []byte{0})
		last := false
		if err != nil {
			if plain, err = aead.Open(nil, nonce, sealed, []byte{1}); err != nil {
				return errors.New("cannot decrypt: wrong key or corrupt file")
			}
			last = true
		}
		if _, err := w.Write(plain); err != nil {
			return err
		}
		if last {
			if _, err := r.Read(make([]byte, 1)); err != io.EOF {
				return errors.New("file has data after its end")
			}
			return nil
		}
	}
}

// PutFile encrypts the file at path and stores it as key.
func (b *Bucket) PutFile(ctx context.Context, key, path string) error {
	client, cfg := b.conn()
	if client == nil {
		return errors.New("no bucket configured")
	}
	aead, err := b.aead()
	if err != nil {
		return err
	}
	in, err := os.Open(path)
	if err != nil {
		return err
	}
	defer in.Close()
	tmp, err := os.CreateTemp("", "one-upload-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	defer tmp.Close()
	bw := bufio.NewWriter(tmp)
	if err := sealStream(aead, bufio.NewReader(in), bw); err != nil {
		return err
	}
	if err := bw.Flush(); err != nil {
		return err
	}
	size, err := tmp.Seek(0, io.SeekCurrent)
	if err != nil {
		return err
	}
	if _, err := tmp.Seek(0, io.SeekStart); err != nil {
		return err
	}
	_, err = client.PutObject(ctx, &s3.PutObjectInput{Bucket: &cfg.Bucket, Key: aws.String(filesPrefix + key), Body: tmp, ContentLength: aws.Int64(size)})
	return err
}

// GetFile fetches key and decrypts it to path, replacing it only once the
// whole file has been read and checked.
func (b *Bucket) GetFile(ctx context.Context, key, path string) error {
	client, cfg := b.conn()
	if client == nil {
		return errors.New("no bucket configured")
	}
	aead, err := b.aead()
	if err != nil {
		return err
	}
	out, err := client.GetObject(ctx, &s3.GetObjectInput{Bucket: &cfg.Bucket, Key: aws.String(filesPrefix + key)})
	if err != nil {
		return err
	}
	defer out.Body.Close()
	tmp, err := os.CreateTemp(filepath.Dir(path), ".one-download-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	bw := bufio.NewWriter(tmp)
	if err := openStream(aead, bufio.NewReader(out.Body), bw); err != nil {
		tmp.Close()
		return err
	}
	if err := bw.Flush(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}

// ListFiles lists the files whose keys start with prefix.
func (b *Bucket) ListFiles(ctx context.Context, prefix string) ([]protocol.FileInfo, error) {
	client, cfg := b.conn()
	if client == nil {
		return nil, errors.New("no bucket configured")
	}
	var out []protocol.FileInfo
	p := s3.NewListObjectsV2Paginator(client, &s3.ListObjectsV2Input{Bucket: &cfg.Bucket, Prefix: aws.String(filesPrefix + prefix)})
	for p.HasMorePages() {
		page, err := p.NextPage(ctx)
		if err != nil {
			return nil, err
		}
		for _, o := range page.Contents {
			out = append(out, protocol.FileInfo{Name: strings.TrimPrefix(aws.ToString(o.Key), filesPrefix), Size: aws.ToInt64(o.Size), Modified: aws.ToTime(o.LastModified)})
		}
	}
	return out, nil
}

// DeleteFile removes key.
func (b *Bucket) DeleteFile(ctx context.Context, key string) error {
	client, cfg := b.conn()
	if client == nil {
		return errors.New("no bucket configured")
	}
	_, err := client.DeleteObject(ctx, &s3.DeleteObjectInput{Bucket: &cfg.Bucket, Key: aws.String(filesPrefix + key)})
	return err
}
