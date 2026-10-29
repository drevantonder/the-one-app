package spaces

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// A Space is a container running Cua's Linux desktop image, with its home
// folder on a volume of its own, so the desktop's sign-ins survive restarts
// and image updates. The container runtime holds all of a Space's state: its
// name, port and token are labels on the container.

const (
	prefix    = "space-"
	image     = "ghcr.io/trycua/linux:24.04"
	home      = "/home/cua"
	firstPort = 32100 // each Space's server, on 127.0.0.1 and over Tailscale HTTPS
)

type space struct {
	Name     string    `json:"name"`
	State    string    `json:"state"` // running, stopped
	Created  time.Time `json:"created"`
	LastUsed time.Time `json:"lastUsed,omitzero"` // last started
	BackedUp bool      `json:"backedUp,omitempty"`
	port     int
	token    string
}

// A runtime runs Spaces' containers: Podman on Linux, Apple's container on
// Mac.
type runtime interface {
	list(ctx context.Context) ([]space, error)
	// run creates and starts a Space's container, its home volume seeded
	// from the image.
	run(ctx context.Context, s space) error
	start(ctx context.Context, name string) error
	stop(ctx context.Context, name string) error
	// remove deletes the container and its home volume; either missing is
	// fine.
	remove(ctx context.Context, name string) error
	// exec runs a command in a running Space, as root.
	exec(ctx context.Context, name string, stdin io.Reader, stdout io.Writer, args ...string) error
}

var rt runtime

var validName = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,39}$`)

func list(ctx context.Context) ([]space, error) { return rt.list(ctx) }

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
	b := make([]byte, 24)
	_, _ = rand.Read(b)
	s := space{Name: name, port: port, token: hex.EncodeToString(b)}
	if err := rt.run(ctx, s); err != nil {
		_ = rt.remove(context.WithoutCancel(ctx), name)
		return space{}, err
	}
	if err := serve(ctx, port, true); err != nil {
		return space{}, err
	}
	return s, ready(ctx, s)
}

// restart starts this device's stopped Spaces that weren't stopped with
// stop, once the runtime answers: at login it may still be starting. It
// reports whether it waited or started any.
func restart(call *sdk.Call, down func(string) bool) bool {
	spaces, err := list(call.Context)
	changed := err != nil
	for i := 0; err != nil; i++ {
		if i == 60 {
			call.Logf("spaces: not restarting Spaces: %v", err)
			return true
		}
		time.Sleep(5 * time.Second)
		spaces, err = list(call.Context)
	}
	for _, s := range spaces {
		if s.State == "running" || down(s.Name) {
			continue
		}
		if err := rt.start(call.Context, s.Name); err != nil {
			call.Logf("spaces: restarting %s: %v", s.Name, err)
			continue
		}
		changed = true
		if err := ready(call.Context, s); err != nil {
			call.Logf("spaces: %v", err)
		}
	}
	return changed
}

func remove(ctx context.Context, s space) error {
	if s.port > 0 {
		_ = serve(ctx, s.port, false)
	}
	return rt.remove(ctx, s.Name)
}

// ready waits for a Space's server to answer.
func ready(ctx context.Context, s space) error {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Minute)
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

var tailnet struct {
	sync.Mutex
	name string
}

// tailnetName is this machine's name on the tailnet: for the viewer's URL,
// and to record which device holds a Space.
func tailnetName(ctx context.Context) (string, error) {
	tailnet.Lock()
	defer tailnet.Unlock()
	if tailnet.name != "" {
		return tailnet.name, nil
	}
	out, err := exec.CommandContext(ctx, "tailscale", "status", "--json").Output()
	if err != nil {
		return "", fmt.Errorf("tailscale status: %w", err)
	}
	var st struct{ Self struct{ DNSName string } }
	if err := json.Unmarshal(out, &st); err != nil {
		return "", err
	}
	tailnet.name = strings.TrimSuffix(st.Self.DNSName, ".")
	return tailnet.name, nil
}
