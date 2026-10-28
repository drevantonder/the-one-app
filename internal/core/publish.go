package core

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/drevantonder/the-one-app/protocol"
)

// Blobs stores published plugin binaries and plugins' files (the bucket).
type Blobs interface {
	PutBlob(ctx context.Context, key string, data []byte) error
	GetBlob(ctx context.Context, key string) ([]byte, error)
	PutFile(ctx context.Context, key, path string) error
	GetFile(ctx context.Context, key, path string) error
	ListFiles(ctx context.Context, prefix string) ([]protocol.FileInfo, error)
	DeleteFile(ctx context.Context, key string) error
}

// SetBlobs enables publishing plugins to every device.
func (n *Node) SetBlobs(b Blobs) {
	n.mu.Lock()
	n.blobs = b
	n.mu.Unlock()
}

// Targets a published plugin is built for.
var Targets = []string{"darwin/arm64", "darwin/amd64", "linux/amd64", "linux/arm64", "linux/arm", "windows/amd64"}

type published struct {
	ID      string   `json:"id"`
	Version string   `json:"version"`
	Build   string   `json:"build"` // content hash of the source
	Targets []string `json:"targets"`
}

func blobKey(id, build, target string) string {
	return fmt.Sprintf("plugins/%s/%s/%s", id, build, strings.ReplaceAll(target, "/", "-"))
}

// publishedState folds plugin.published entries: the latest build per id.
func (n *Node) publishedState() map[string]published {
	out := map[string]published{}
	entries, err := n.store.Entries(CoreID)
	if err != nil {
		return out
	}
	for _, e := range entries {
		if e.Type != "plugin.published" {
			continue
		}
		var p published
		if json.Unmarshal(e.Payload, &p) == nil {
			out[p.ID] = p
		}
	}
	return out
}

// publish cross-compiles a linked plugin for every target, stores the
// binaries in the bucket and announces the build to every device.
func (n *Node) publish(ctx context.Context, id, actor string) (protocol.Result, error) {
	n.mu.RLock()
	src, blobs := n.sources[id], n.blobs
	g := n.gens[id]
	n.mu.RUnlock()
	if src == nil || src.builtin || src.dir == "" {
		return protocol.Result{}, fmt.Errorf("%q is not linked from source on this device", id)
	}
	if blobs == nil {
		return protocol.Result{}, fmt.Errorf("publishing needs a bucket")
	}
	build := fingerprint(src.dir)
	dir, err := os.MkdirTemp("", "one-publish-")
	if err != nil {
		return protocol.Result{}, err
	}
	defer os.RemoveAll(dir)
	for _, t := range Targets {
		goos, goarch, _ := strings.Cut(t, "/")
		out := filepath.Join(dir, strings.ReplaceAll(t, "/", "-"))
		cmd := exec.CommandContext(ctx, "go", "build", "-trimpath", "-o", out, ".")
		cmd.Dir = src.dir
		cmd.Env = append(os.Environ(), "GOOS="+goos, "GOARCH="+goarch, "CGO_ENABLED=0")
		if goarch == "arm" {
			cmd.Env = append(cmd.Env, "GOARM=7")
		}
		if b, err := cmd.CombinedOutput(); err != nil {
			return protocol.Result{}, fmt.Errorf("build for %s failed:\n%s", t, strings.TrimSpace(string(b)))
		}
		data, err := os.ReadFile(out)
		if err != nil {
			return protocol.Result{}, err
		}
		if err := blobs.PutBlob(ctx, blobKey(id, build, t), data); err != nil {
			return protocol.Result{}, err
		}
	}
	version := ""
	if g != nil {
		version = g.manifest.Version
	}
	payload, _ := json.Marshal(published{ID: id, Version: version, Build: build, Targets: Targets})
	if _, err := n.store.Append(CoreID, "plugin.published", payload, actor); err != nil {
		return protocol.Result{}, err
	}
	return protocol.Result{Text: fmt.Sprintf("Published %s build %s for %d platforms. Every device picks it up as it syncs.", id, build, len(Targets))}, nil
}

// fetchPublished makes sure this device has the published binary for its
// platform, downloading it from the bucket if needed.
func (n *Node) fetchPublished(ctx context.Context, p published) (string, error) {
	target := runtime.GOOS + "/" + runtime.GOARCH
	path := filepath.Join(n.dataDir, "plugins", p.ID, "published-"+p.Build, "plugin")
	if runtime.GOOS == "windows" {
		path += ".exe"
	}
	if _, err := os.Stat(path); err == nil {
		return path, nil
	}
	n.mu.RLock()
	blobs := n.blobs
	n.mu.RUnlock()
	if blobs == nil {
		return "", fmt.Errorf("no bucket to fetch %s from", p.ID)
	}
	data, err := blobs.GetBlob(ctx, blobKey(p.ID, p.Build, target))
	if err != nil {
		return "", fmt.Errorf("fetch %s for %s: %w", p.ID, target, err)
	}
	sum := sha256.Sum256(data)
	tmp := path + ".tmp-" + hex.EncodeToString(sum[:4])
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return "", err
	}
	if err := os.WriteFile(tmp, data, 0o755); err != nil {
		return "", err
	}
	return path, os.Rename(tmp, path)
}

// declaredState folds plugin.declared and plugin.removed entries.
func (n *Node) declaredState() (map[string]json.RawMessage, map[string]bool) {
	declared, removed := map[string]json.RawMessage{}, map[string]bool{}
	entries, err := n.store.Entries(CoreID)
	if err != nil {
		return declared, removed
	}
	for _, e := range entries {
		var p struct {
			ID   string          `json:"id"`
			Spec json.RawMessage `json:"spec"`
		}
		if json.Unmarshal(e.Payload, &p) != nil {
			continue
		}
		switch e.Type {
		case "plugin.declared":
			declared[p.ID], removed[p.ID] = p.Spec, false
		case "plugin.removed":
			delete(declared, p.ID)
			removed[p.ID] = true
		case "plugin.published":
			removed[p.ID] = false
		}
	}
	for id, r := range removed {
		if !r {
			delete(removed, id)
		}
	}
	return declared, removed
}

// files stores a plugin's files in the bucket, each plugin under its own id.
func (n *Node) files(ctx context.Context, plugin string, in protocol.FilesParams) (any, error) {
	n.mu.RLock()
	blobs := n.blobs
	n.mu.RUnlock()
	if blobs == nil {
		return nil, errors.New("storing files needs a bucket: configure one with bucket_configure")
	}
	if strings.Contains(in.Name, "..") || strings.HasPrefix(in.Name, "/") || (in.Name == "" && in.Op != "list") {
		return nil, fmt.Errorf("bad file name %q", in.Name)
	}
	key := plugin + "/" + in.Name
	switch in.Op {
	case "put":
		return struct{}{}, blobs.PutFile(ctx, key, in.Path)
	case "get":
		return struct{}{}, blobs.GetFile(ctx, key, in.Path)
	case "delete":
		return struct{}{}, blobs.DeleteFile(ctx, key)
	case "list":
		found, err := blobs.ListFiles(ctx, key)
		if err != nil {
			return nil, err
		}
		for i := range found {
			found[i].Name = strings.TrimPrefix(found[i].Name, plugin+"/")
		}
		return found, nil
	}
	return nil, fmt.Errorf("unknown files op %q", in.Op)
}
