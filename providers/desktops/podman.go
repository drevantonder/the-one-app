package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

// A desktop is a rootless Podman container named prefix+name, its home folder
// on a volume of its own, so its sign-ins survive restarts and image updates.
// Its server's port and token are labels on the container, where the old app
// kept them too, so its desktops carry on here.
const (
	prefix    = "desktop-"
	cuaImage  = "ghcr.io/trycua/linux:24.04"
	home      = "/home/cua"
	firstPort = 32100
)

type desktop struct {
	Name    string    `json:"name"`
	State   string    `json:"state"` // running, paused or stopped
	Started time.Time `json:"started,omitzero"`
	pool    bool      // made to be borrowed (pool.go)
	port    int
	token   string
}

var validName = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,39}$`)

// podmanEnv points Podman at the user's runtime directory and systemd, which a
// service has no login session for; lingering keeps them.
func podmanEnv() error {
	if _, err := exec.LookPath("podman"); err != nil {
		return fmt.Errorf("needs Podman")
	}
	if os.Getenv("XDG_RUNTIME_DIR") == "" {
		dir := fmt.Sprintf("/run/user/%d", os.Getuid())
		if _, err := os.Stat(dir); err != nil {
			return fmt.Errorf("Podman needs %s: run loginctl enable-linger %s", dir, os.Getenv("USER"))
		}
		os.Setenv("XDG_RUNTIME_DIR", dir)
		os.Setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path="+dir+"/bus")
	}
	// Containers with --restart=always start again at boot through this. At
	// boot the user's systemd may not answer yet; revive covers that.
	if out, err := exec.Command("systemctl", "--user", "enable", "podman-restart.service").CombinedOutput(); err != nil {
		log.Printf("enabling podman-restart: %v: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

func podman(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, "podman", args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("podman %s: %v: %s", args[0], err, strings.TrimSpace(stderr.String()))
	}
	return out, nil
}

func list(ctx context.Context) ([]desktop, error) {
	out, err := podman(ctx, "ps", "--all", "--filter", "label=one.token", "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		Names     []string
		State     string
		StartedAt int64
		Labels    map[string]string
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	var all []desktop
	for _, r := range raw {
		if len(r.Names) == 0 || !strings.HasPrefix(r.Names[0], prefix) {
			continue
		}
		d := desktop{Name: strings.TrimPrefix(r.Names[0], prefix), State: "stopped", token: r.Labels["one.token"], pool: r.Labels["one.pool"] != ""}
		if r.State == "running" || r.State == "paused" {
			d.State = r.State
		}
		if r.StartedAt > 0 {
			d.Started = time.Unix(r.StartedAt, 0)
		}
		d.port, _ = strconv.Atoi(r.Labels["one.port"])
		all = append(all, d)
	}
	return all, nil
}

// pick is the desktop named, or with no name the only one.
func pick(all []desktop, name string) (desktop, error) {
	var names []string
	for _, d := range all {
		if d.Name == name || name == "" && len(all) == 1 {
			return d, nil
		}
		names = append(names, d.Name)
	}
	switch {
	case len(all) == 0:
		return desktop{}, fmt.Errorf("no desktops: make one with create")
	case name == "":
		return desktop{}, fmt.Errorf("which desktop? pass desktop: %s", strings.Join(names, ", "))
	}
	return desktop{}, fmt.Errorf("no desktop named %q: there's %s", name, strings.Join(names, ", "))
}

func find(ctx context.Context, name string) (desktop, error) {
	all, err := list(ctx)
	if err != nil {
		return desktop{}, err
	}
	return pick(all, name)
}

// nextPort is the first port after every desktop's.
func nextPort(all []desktop) int {
	port := firstPort
	for _, d := range all {
		port = max(port, d.port+1)
	}
	return port
}

// creating keeps two desktops being made at once from taking the same port.
var creating sync.Mutex

// create makes a desktop and starts it, waiting for its server to answer
// if wait.
func create(ctx context.Context, name string, sz size, pool, wait bool) (desktop, error) {
	if !validName.MatchString(name) {
		return desktop{}, fmt.Errorf("a desktop's name is lowercase letters, digits and dashes")
	}
	creating.Lock()
	defer creating.Unlock()
	all, err := list(ctx)
	if err != nil {
		return desktop{}, err
	}
	if _, err := pick(all, name); err == nil {
		return desktop{}, fmt.Errorf("desktop %s already exists", name)
	}
	b := make([]byte, 24)
	rand.Read(b)
	d := desktop{Name: name, State: "running", pool: pool, port: nextPort(all), token: hex.EncodeToString(b)}
	c := prefix + name
	labels := []string{"--label", fmt.Sprintf("one.port=%d", d.port), "--label", "one.token=" + d.token}
	if pool {
		labels = append(labels, "--label", "one.pool=1")
	}
	args := append([]string{"run", "--detach", "--name", c, "--restart=always"}, labels...)
	args = append(args, sz.limits()...)
	_, err = podman(ctx, append(args,
		"--env", "CUA_ENV_TOKEN="+d.token,
		// The image says HOME=/root, which its desktop user inherits; there
		// Chromium can't keep crash reports, and won't start.
		"--env", "HOME="+home,
		"--publish", fmt.Sprintf("127.0.0.1:%d:3211", d.port),
		// A new named volume starts as a copy of the image's home folder.
		"--volume", c+"-home:"+home,
		"--shm-size=1g",
		cuaImage)...)
	if err != nil {
		remove(context.WithoutCancel(ctx), d)
		return desktop{}, err
	}
	if !wait {
		return d, nil
	}
	return d, ready(ctx, d)
}

// start starts a desktop, and again after the device restarts.
func start(ctx context.Context, d desktop) error {
	if _, err := podman(ctx, "update", "--restart=always", prefix+d.Name); err != nil {
		return err
	}
	if _, err := podman(ctx, "start", prefix+d.Name); err != nil {
		return err
	}
	return ready(ctx, d)
}

// stop stops a desktop until it's started again, restarts of the device too.
func stop(ctx context.Context, d desktop) error {
	if d.State == "paused" {
		if err := unpause(ctx, d); err != nil {
			return err
		}
	}
	if _, err := podman(ctx, "update", "--restart=no", prefix+d.Name); err != nil {
		return err
	}
	_, err := podman(ctx, "stop", "--time", "10", prefix+d.Name)
	return err
}

// remove deletes a desktop's container and its volumes.
func remove(ctx context.Context, d desktop) error {
	out, err := podman(ctx, "inspect", "--format", "{{range .Mounts}}{{.Name}} {{end}}", prefix+d.Name)
	if err != nil {
		return err
	}
	if _, err := podman(ctx, "rm", "--force", "--time", "10", prefix+d.Name); err != nil {
		return err
	}
	for _, v := range strings.Fields(string(out)) {
		if _, err := podman(ctx, "volume", "rm", v); err != nil {
			return err
		}
	}
	if d.port > 0 {
		tailscale(ctx, "serve", "--https="+strconv.Itoa(d.port), "off")
	}
	return nil
}

// pause freezes a desktop's processes: it keeps its memory but uses no CPU
// until unpaused, which takes a moment.
func pause(ctx context.Context, d desktop) error {
	_, err := podman(ctx, "pause", prefix+d.Name)
	return err
}

func unpause(ctx context.Context, d desktop) error {
	_, err := podman(ctx, "unpause", prefix+d.Name)
	return err
}

// sh runs a script in a running desktop, as root.
func sh(ctx context.Context, d desktop, script string) error {
	var out bytes.Buffer
	return in(ctx, d, strings.NewReader(script), &out, "sh", "-s")
}

// in runs a command in a running desktop, as root. An exitError keeps its
// exit code, which some callers accept.
func in(ctx context.Context, d desktop, stdin io.Reader, stdout io.Writer, args ...string) error {
	cmd := exec.CommandContext(ctx, "podman", append([]string{"exec", "--interactive", "--user", "root", prefix + d.Name}, args...)...)
	cmd.Stdin, cmd.Stdout = stdin, stdout
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return &exitError{err: err, stderr: strings.TrimSpace(stderr.String())}
	}
	return nil
}

type exitError struct {
	err    error
	stderr string
}

func (e *exitError) Error() string { return fmt.Sprintf("%v: %s", e.err, e.stderr) }
func (e *exitError) Unwrap() error { return e.err }

// tarred reports whether tar's error is only that files changed while it read
// them, as a running browser's do.
func tarred(err error) bool {
	var exit *exec.ExitError
	return err == nil || errors.As(err, &exit) && exit.ExitCode() == 1
}

func (d desktop) local() string { return fmt.Sprintf("http://127.0.0.1:%d", d.port) }

// ready waits for a desktop's server to answer.
func ready(ctx context.Context, d desktop) error {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Minute)
	defer cancel()
	for {
		req, _ := http.NewRequestWithContext(ctx, http.MethodGet, d.local()+"/health", nil)
		if resp, err := http.DefaultClient.Do(req); err == nil {
			resp.Body.Close()
			if resp.StatusCode < 300 {
				return nil
			}
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("desktop %s didn't start: %w", d.Name, ctx.Err())
		case <-time.After(time.Second):
		}
	}
}

func tailscale(ctx context.Context, args ...string) (string, error) {
	out, err := exec.CommandContext(ctx, "tailscale", args...).CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("tailscale %s: %v: %s", args[0], err, strings.TrimSpace(string(out)))
	}
	return string(out), nil
}
