package spaces

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// A Space is an Incus container running Cua's Linux desktop image. Incus
// holds all of its state: the token is the container's environment, the
// port its proxy device, and the home folder a volume of its own, so the
// desktop's sign-ins survive restarts and image updates.

const (
	prefix    = "space-"
	image     = "ghcr:trycua/linux:24.04"
	home      = "/home/cua"
	firstPort = 32100 // each Space's server, on 127.0.0.1 and over Tailscale HTTPS
)

type space struct {
	Name     string    `json:"name"`
	State    string    `json:"state"`
	Created  time.Time `json:"created"`
	LastUsed time.Time `json:"lastUsed,omitzero"`
	BackedUp bool      `json:"backedUp,omitempty"`
	port     int
	token    string
}

var validName = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,39}$`)

func incus(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, "incus", args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("incus %s: %v: %s", args[0], err, strings.TrimSpace(stderr.String()))
	}
	return out, nil
}

func list(ctx context.Context) ([]space, error) {
	out, err := incus(ctx, "list", prefix, "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		Name     string                       `json:"name"`
		Status   string                       `json:"status"`
		Created  time.Time                    `json:"created_at"`
		LastUsed time.Time                    `json:"last_used_at"`
		Config   map[string]string            `json:"config"`
		Devices  map[string]map[string]string `json:"devices"`
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	var spaces []space
	for _, r := range raw {
		if !strings.HasPrefix(r.Name, prefix) {
			continue
		}
		s := space{Name: strings.TrimPrefix(r.Name, prefix), State: strings.ToLower(r.Status), Created: r.Created, token: r.Config["environment.CUA_ENV_TOKEN"]}
		if r.LastUsed.Year() > 1 {
			s.LastUsed = r.LastUsed
		}
		if l := r.Devices["spacesd"]["listen"]; l != "" {
			s.port, _ = strconv.Atoi(l[strings.LastIndex(l, ":")+1:])
		}
		spaces = append(spaces, s)
	}
	return spaces, nil
}

func find(ctx context.Context, name string) (space, error) {
	spaces, err := list(ctx)
	if err != nil {
		return space{}, err
	}
	for _, s := range spaces {
		if s.Name == name {
			return s, nil
		}
	}
	return space{}, fmt.Errorf("no Space named %q", name)
}

func create(ctx context.Context, name string) (space, error) {
	if !validName.MatchString(name) {
		return space{}, fmt.Errorf("a Space's name is lowercase letters, digits and dashes")
	}
	spaces, err := list(ctx)
	if err != nil {
		return space{}, err
	}
	port := firstPort
	for _, s := range spaces {
		if s.Name == name {
			return space{}, fmt.Errorf("Space %q already exists", name)
		}
		port = max(port, s.port+1)
	}
	remotes, err := incus(ctx, "remote", "list", "--format", "csv")
	if err != nil {
		return space{}, err
	}
	if !strings.Contains(string(remotes), "ghcr,") {
		if _, err := incus(ctx, "remote", "add", "ghcr", "https://ghcr.io", "--protocol=oci", "--public"); err != nil {
			return space{}, err
		}
	}
	b := make([]byte, 24)
	_, _ = rand.Read(b)
	token := hex.EncodeToString(b)
	c, vol := prefix+name, prefix+name+"-home"
	pool, err := incus(ctx, "profile", "device", "get", "default", "root", "pool")
	if err != nil {
		return space{}, err
	}
	p := strings.TrimSpace(string(pool))
	steps := [][]string{
		{"launch", image, c, "-c", "environment.CUA_ENV_TOKEN=" + token, "-c", "limits.memory=4GiB"},
		// The home folder moves onto its own volume, seeded from the image.
		{"storage", "volume", "create", p, vol},
		{"storage", "volume", "attach", p, vol, c, "seed", "/mnt/home"},
		{"exec", c, "--", "cp", "-a", home + "/.", "/mnt/home/"},
		{"storage", "volume", "detach", p, vol, c},
		{"storage", "volume", "attach", p, vol, c, "home", home},
		{"config", "device", "add", c, "spacesd", "proxy", fmt.Sprintf("listen=tcp:127.0.0.1:%d", port), "connect=tcp:127.0.0.1:3211"},
		{"restart", c},
	}
	for _, s := range steps {
		if _, err := incus(ctx, s...); err != nil {
			_ = remove(context.WithoutCancel(ctx), space{Name: name, port: port})
			return space{}, err
		}
	}
	if err := serve(ctx, port, true); err != nil {
		return space{}, err
	}
	s := space{Name: name, port: port, token: token}
	return s, ready(ctx, s)
}

// ready waits for a Space's server to answer.
func ready(ctx context.Context, s space) error {
	ctx, cancel := context.WithTimeout(ctx, time.Minute)
	defer cancel()
	for {
		req, _ := http.NewRequestWithContext(ctx, http.MethodGet, s.local()+"/health", nil)
		if resp, err := http.DefaultClient.Do(req); err == nil {
			resp.Body.Close()
			if resp.StatusCode < 300 {
				return nil
			}
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("Space %q didn't start: %w", s.Name, ctx.Err())
		case <-time.After(time.Second):
		}
	}
}

func remove(ctx context.Context, s space) error {
	if s.port > 0 {
		_ = serve(ctx, s.port, false)
	}
	_, err := incus(ctx, "delete", "--force", prefix+s.Name)
	pool, _ := incus(ctx, "profile", "device", "get", "default", "root", "pool")
	_, verr := incus(ctx, "storage", "volume", "delete", strings.TrimSpace(string(pool)), prefix+s.Name+"-home")
	if err != nil && !strings.Contains(err.Error(), "not found") {
		return err
	}
	if verr != nil && !strings.Contains(verr.Error(), "not found") {
		return verr
	}
	return nil
}

// serve publishes a Space's server over HTTPS on the tailnet, which the
// viewer needs: browsers only allow its video on secure pages.
func serve(ctx context.Context, port int, on bool) error {
	p := strconv.Itoa(port)
	args := []string{"serve", "--bg", "--https=" + p, "http://127.0.0.1:" + p}
	if !on {
		args = []string{"serve", "--https=" + p, "off"}
	}
	out, err := exec.CommandContext(ctx, "tailscale", args...).CombinedOutput()
	if err != nil {
		return fmt.Errorf("tailscale serve: %v: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func (s space) local() string { return fmt.Sprintf("http://127.0.0.1:%d", s.port) }

// tailnetName is this machine's name on the tailnet, for the viewer's URL.
func tailnetName(ctx context.Context) (string, error) {
	out, err := exec.CommandContext(ctx, "tailscale", "status", "--json").Output()
	if err != nil {
		return "", fmt.Errorf("tailscale status: %w", err)
	}
	var st struct{ Self struct{ DNSName string } }
	if err := json.Unmarshal(out, &st); err != nil {
		return "", err
	}
	return strings.TrimSuffix(st.Self.DNSName, "."), nil
}
