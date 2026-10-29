package models

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// local is this device's own model servers.
type local struct {
	ollama ollama
	comfy  comfy
	apple  *apple

	gpu sync.Mutex // one model at a time on the GPU, so a chat and an image don't both load

	mu   sync.Mutex
	pull map[string]*download
}

type download struct {
	Status    string `json:"status"`
	Completed int64  `json:"completed,omitempty"`
	Total     int64  `json:"total,omitempty"`
	Error     string `json:"error,omitempty"`
}

func newLocal(c *sdk.Ctx) *local {
	host := os.Getenv("OLLAMA_HOST")
	switch {
	case host == "":
		host = "http://127.0.0.1:11434"
	case !strings.Contains(host, "://"):
		host = "http://" + host
	}
	l := &local{ollama: ollama{base: host}, comfy: comfy{base: envOr("COMFYUI_URL", "http://127.0.0.1:8188")}, pull: map[string]*download{}}
	if runtime.GOOS == "darwin" {
		dir, _ := c.Dir()
		l.apple = &apple{socket: filepath.Join(dir, "fm.sock")}
	}
	return l
}

func envOr(k, def string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return def
}

// models lists what this device can run; a server that isn't running has
// no models.
func (l *local) models(ctx context.Context, device string) []Model {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	var out []Model
	if ms, err := l.ollama.models(ctx); err == nil {
		out = append(out, ms...)
	}
	if l.apple != nil && l.apple.available(ctx) {
		out = append(out, Model{Name: "apple", Backend: "apple", Kind: "chat"})
	}
	if names, err := l.comfy.models(ctx); err == nil {
		for _, n := range names {
			out = append(out, Model{Name: n, Backend: "comfyui", Kind: "image"})
		}
	}
	for i := range out {
		out[i].Device = device
	}
	return out
}

// chat answers with a local model, if this device has it.
func (l *local) chat(ctx context.Context, model string, req chatRequest) (string, bool, error) {
	if model == "apple" && l.apple != nil && l.apple.available(ctx) {
		req.Model = "system"
		client, err := l.apple.client(ctx)
		if err != nil {
			return "", true, err
		}
		text, err := client.chat(ctx, req)
		return text, true, err
	}
	ms, err := l.ollama.models(ctx)
	if err != nil {
		return "", false, nil
	}
	for _, m := range ms {
		if !sameModel(m.Name, model) {
			continue
		}
		l.gpu.Lock()
		defer l.gpu.Unlock()
		l.comfy.free(ctx)
		req.Model = m.Name
		if req.ReasoningEffort == "" {
			req.ReasoningEffort = "none"
		}
		text, err := openAI{baseURL: l.ollama.base + "/v1"}.chat(ctx, req)
		return text, true, err
	}
	return "", false, nil
}

// decide answers with a local decision model, if this device has it.
func (l *local) decide(ctx context.Context, req decideRequest) (json.RawMessage, bool, error) {
	ms, err := l.ollama.models(ctx)
	if err != nil {
		return nil, false, nil
	}
	for _, m := range ms {
		if m.Kind != "decide" || !sameModel(m.Name, req.Model) {
			continue
		}
		l.gpu.Lock()
		defer l.gpu.Unlock()
		l.comfy.free(ctx)
		req.Model = m.Name
		out, err := systemOne{baseURL: l.ollama.base + "/v1"}.decide(ctx, req)
		return out, true, err
	}
	return nil, false, nil
}

func (l *local) image(ctx context.Context, in imageIn) (sdk.Image, error) {
	l.gpu.Lock()
	defer l.gpu.Unlock()
	if err := l.ollama.unload(ctx, ""); err != nil && !errors.Is(err, errNotRunning) {
		return sdk.Image{}, err
	}
	return l.comfy.image(ctx, in)
}

func (l *local) unload(ctx context.Context, model string) error {
	err := l.ollama.unload(ctx, model)
	if model == "" {
		l.comfy.free(ctx)
	}
	if errors.Is(err, errNotRunning) {
		return nil
	}
	return err
}

func (l *local) download(model string) error {
	if strings.TrimSpace(model) == "" {
		return errors.New("give a model to download")
	}
	if _, err := l.ollama.version(context.Background()); err != nil {
		return fmt.Errorf("Ollama isn't running on this device: install it from https://ollama.com/download")
	}
	l.mu.Lock()
	if d := l.pull[model]; d != nil && d.Error == "" && d.Status != "success" {
		l.mu.Unlock()
		return nil
	}
	d := &download{Status: "starting"}
	l.pull[model] = d
	l.mu.Unlock()
	go func() {
		err := l.ollama.pull(context.Background(), model, func(p download) {
			l.mu.Lock()
			*d = p
			l.mu.Unlock()
		})
		l.mu.Lock()
		defer l.mu.Unlock()
		if err != nil {
			d.Error = err.Error()
		} else {
			delete(l.pull, model)
		}
	}()
	return nil
}

func (l *local) downloads() map[string]download {
	l.mu.Lock()
	defer l.mu.Unlock()
	out := map[string]download{}
	for k, v := range l.pull {
		out[k] = *v
	}
	return out
}

var errNotRunning = errors.New("not running")

type ollama struct{ base string }

func (o ollama) get(ctx context.Context, path string, out any) error {
	req, _ := http.NewRequestWithContext(ctx, "GET", o.base+path, nil)
	return do(req, out)
}

func (o ollama) post(ctx context.Context, path string, in, out any) error {
	body, _ := json.Marshal(in)
	req, _ := http.NewRequestWithContext(ctx, "POST", o.base+path, bytes.NewReader(body))
	req.Header.Set("content-type", "application/json")
	return do(req, out)
}

func (o ollama) version(ctx context.Context) (string, error) {
	var v struct {
		Version string `json:"version"`
	}
	err := o.get(ctx, "/api/version", &v)
	return v.Version, err
}

func (o ollama) models(ctx context.Context) ([]Model, error) {
	var tags, ps struct {
		Models []struct {
			Name         string   `json:"name"`
			Size         int64    `json:"size"`
			Capabilities []string `json:"capabilities"`
		} `json:"models"`
	}
	if err := o.get(ctx, "/api/tags", &tags); err != nil {
		return nil, errNotRunning
	}
	if err := o.get(ctx, "/api/ps", &ps); err != nil {
		return nil, err
	}
	out := make([]Model, 0, len(tags.Models))
	for _, t := range tags.Models {
		m := Model{Name: strings.TrimSuffix(t.Name, ":latest"), Backend: "ollama", Kind: "chat", Size: t.Size}
		if slices.Contains(t.Capabilities, "decision") {
			m.Kind = "decide"
		}
		for _, p := range ps.Models {
			m.Loaded = m.Loaded || p.Name == t.Name
		}
		out = append(out, m)
	}
	return out, nil
}

// unload unloads model, or every loaded model.
func (o ollama) unload(ctx context.Context, model string) error {
	var ps struct {
		Models []struct {
			Name string `json:"name"`
		} `json:"models"`
	}
	if err := o.get(ctx, "/api/ps", &ps); err != nil {
		return errNotRunning
	}
	for _, m := range ps.Models {
		if model != "" && !sameModel(m.Name, model) {
			continue
		}
		if err := o.post(ctx, "/api/generate", map[string]any{"model": m.Name, "keep_alive": 0}, nil); err != nil {
			return err
		}
	}
	return nil
}

func (o ollama) delete(ctx context.Context, model string) error {
	body, _ := json.Marshal(map[string]string{"model": model})
	req, _ := http.NewRequestWithContext(ctx, "DELETE", o.base+"/api/delete", bytes.NewReader(body))
	return do(req, nil)
}

// pull downloads model, reporting progress as Ollama gives it.
func (o ollama) pull(ctx context.Context, model string, progress func(download)) error {
	body, _ := json.Marshal(map[string]any{"model": model, "stream": true})
	req, _ := http.NewRequestWithContext(ctx, "POST", o.base+"/api/pull", bytes.NewReader(body))
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	if res.StatusCode >= 300 {
		return httpError(res)
	}
	sc := bufio.NewScanner(res.Body)
	for sc.Scan() {
		var d download
		if json.Unmarshal(sc.Bytes(), &d) != nil {
			continue
		}
		if d.Error != "" {
			return errors.New(d.Error)
		}
		progress(d)
	}
	return sc.Err()
}

// apple is Apple's on-device model, served by macOS's fm command on a
// socket in the plugin's folder while the plugin runs.
type apple struct {
	socket string

	mu    sync.Mutex
	serve *exec.Cmd
	known *bool
}

func (a *apple) available(ctx context.Context) bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.known == nil {
		// It exits non-zero when Private Cloud Compute isn't available, even
		// if the on-device model is.
		out, _ := exec.CommandContext(ctx, "fm", "available").CombinedOutput()
		ok := strings.Contains(string(out), "System model available")
		a.known = &ok
	}
	return *a.known
}

func (a *apple) client(ctx context.Context) (openAI, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	c := openAI{baseURL: "http://fm/v1", http: &http.Client{Transport: &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "unix", a.socket)
		},
	}}}
	if a.serve != nil && a.serve.ProcessState == nil || a.serve == nil && c.healthy(ctx) {
		return c, nil // started by this plugin, or by a run of it before a restart
	}
	_ = os.Remove(a.socket)
	a.serve = exec.Command("fm", "serve", "--socket", a.socket)
	if err := a.serve.Start(); err != nil {
		return c, fmt.Errorf("start Apple's model server: %w", err)
	}
	go a.serve.Wait()
	for range 50 {
		if _, err := os.Stat(a.socket); err == nil {
			return c, nil
		}
		time.Sleep(100 * time.Millisecond)
	}
	return c, errors.New("Apple's model server didn't start")
}

func do(req *http.Request, out any) error {
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	if res.StatusCode >= 300 {
		return httpError(res)
	}
	if out == nil {
		return nil
	}
	return json.NewDecoder(res.Body).Decode(out)
}

func httpError(res *http.Response) error {
	b, _ := io.ReadAll(io.LimitReader(res.Body, 2000))
	var e struct {
		Error any `json:"error"`
	}
	if json.Unmarshal(b, &e) == nil && e.Error != nil {
		if m, ok := e.Error.(map[string]any); ok && m["message"] != nil {
			return fmt.Errorf("%s: %v", res.Status, m["message"])
		}
		return fmt.Errorf("%s: %v", res.Status, e.Error)
	}
	return fmt.Errorf("%s: %s", res.Status, strings.TrimSpace(string(b)))
}
