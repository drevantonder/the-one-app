package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

// A computer is a container running Cua's desktop image (image.go), named prefix+name,
// its home folder on a volume of its own, so its sign-ins survive restarts
// and image updates. Its server's port and token are labels on the container,
// where the old app kept them too, so its computers carry on here. The
// container runtime is Podman on Linux and Apple's container on a Mac.
const (
	prefix    = "computer-"
	cuaImage  = "ghcr.io/trycua/linux:24.04"
	home      = "/home/cua"
	firstPort = 32100
)

type computer struct {
	Name    string    `json:"name"`
	State   string    `json:"state"` // running, paused or stopped
	Started time.Time `json:"started,omitzero"`
	pool    bool      // made to be borrowed (pool.go)
	image   string    // the image it runs
	port    int
	token   string
}

var validName = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,39}$`)

// validProfile is a browser profile's name, as people name them: Personal, PTC.
var validProfile = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9-]{0,39}$`)

// A containers runs computers' containers.
type containers interface {
	list(ctx context.Context) ([]computer, error)
	// run makes the computer's container and home volume from the image,
	// and starts it.
	run(ctx context.Context, d computer, sz size, image string) error
	// start starts a computer, and again after the server restarts; stop
	// stops it until it's started again.
	start(ctx context.Context, d computer) error
	stop(ctx context.Context, d computer) error
	restart(ctx context.Context, d computer) error
	// remove deletes the container and its volumes.
	remove(ctx context.Context, d computer) error
	// pause freezes a computer's processes: it keeps its memory but uses no
	// CPU until unpaused, which takes a moment.
	pause(ctx context.Context, d computer) error
	unpause(ctx context.Context, d computer) error
	// exec runs a command in a running computer, as root.
	exec(ctx context.Context, d computer, stdin io.Reader, stdout io.Writer, args ...string) error
	limits(ctx context.Context, all []computer) (map[string]limit, error)
	resize(ctx context.Context, d computer, sz size) error
	stats(ctx context.Context) (map[string]usage, error)
	hasImage(ctx context.Context, image string) bool
	// build builds an image from the Containerfile in dir.
	build(ctx context.Context, image, dir string) error
	images(ctx context.Context) ([]string, error)
	removeImage(ctx context.Context, image string) error
}

// rt is this server's.
var rt containers

func setup(dir string) error {
	var err error
	if runtime.GOOS == "darwin" {
		rt, err = newApple(dir)
	} else {
		rt, err = newPodman()
	}
	return err
}

func list(ctx context.Context) ([]computer, error)          { return rt.list(ctx) }
func pause(ctx context.Context, d computer) error           { return rt.pause(ctx, d) }
func unpause(ctx context.Context, d computer) error         { return rt.unpause(ctx, d) }
func resize(ctx context.Context, d computer, sz size) error { return rt.resize(ctx, d, sz) }
func stats(ctx context.Context) (map[string]usage, error)   { return rt.stats(ctx) }
func limits(ctx context.Context, all []computer) (map[string]limit, error) {
	return rt.limits(ctx, all)
}

// in runs a command in a running computer, as root. An exitError keeps its
// exit code, which some callers accept.
func in(ctx context.Context, d computer, stdin io.Reader, stdout io.Writer, args ...string) error {
	return rt.exec(ctx, d, stdin, stdout, args...)
}

// pick is the computer named, or with no name the only one.
func pick(all []computer, name string) (computer, error) {
	var names []string
	for _, d := range all {
		if d.Name == name || name == "" && len(all) == 1 {
			return d, nil
		}
		names = append(names, d.Name)
	}
	switch {
	case len(all) == 0:
		return computer{}, fmt.Errorf("no computers: make one with create")
	case name == "":
		return computer{}, fmt.Errorf("which computer? pass computer: %s", strings.Join(names, ", "))
	}
	return computer{}, fmt.Errorf("no computer named %q: there's %s", name, strings.Join(names, ", "))
}

func find(ctx context.Context, name string) (computer, error) {
	all, err := list(ctx)
	if err != nil {
		return computer{}, err
	}
	return pick(all, name)
}

// nextPort is the first port after every computer's.
func nextPort(all []computer) int {
	port := firstPort
	for _, d := range all {
		port = max(port, d.port+1)
	}
	return port
}

// creating keeps two computers being made at once from taking the same port.
var creating sync.Mutex

// create makes a computer and starts it, waiting for its server to answer
// if wait.
func create(ctx context.Context, name string, sz size, pool, wait bool) (computer, error) {
	if !validName.MatchString(name) {
		return computer{}, fmt.Errorf("a computer's name is lowercase letters, digits and dashes")
	}
	img, err := ensureImage(ctx)
	if err != nil {
		return computer{}, err
	}
	creating.Lock()
	defer creating.Unlock()
	all, err := list(ctx)
	if err != nil {
		return computer{}, err
	}
	if _, err := pick(all, name); err == nil {
		return computer{}, fmt.Errorf("computer %s already exists", name)
	}
	b := make([]byte, 24)
	rand.Read(b)
	d := computer{Name: name, State: "running", pool: pool, port: nextPort(all), token: hex.EncodeToString(b)}
	if err := rt.run(ctx, d, sz, img); err != nil {
		remove(context.WithoutCancel(ctx), d)
		return computer{}, err
	}
	if !wait {
		return d, nil
	}
	return d, ready(ctx, d)
}

// labels are what a computer's container carries.
func (d computer) labels() []string {
	l := []string{"--label", fmt.Sprintf("one.port=%d", d.port), "--label", "one.token=" + d.token}
	if d.pool {
		l = append(l, "--label", "one.pool=1")
	}
	return l
}

// start starts a computer, and again after the server restarts.
func start(ctx context.Context, d computer) error {
	if err := rt.start(ctx, d); err != nil {
		return err
	}
	return ready(ctx, d)
}

// stop stops a computer until it's started again, restarts of the server too.
func stop(ctx context.Context, d computer) error {
	if d.State == "paused" {
		if err := unpause(ctx, d); err != nil {
			return err
		}
	}
	return rt.stop(ctx, d)
}

// remove deletes a computer's container and its volumes.
func remove(ctx context.Context, d computer) error {
	if err := rt.remove(ctx, d); err != nil {
		return err
	}
	if d.port > 0 {
		tailscale(ctx, "serve", "--https="+strconv.Itoa(d.port), "off")
	}
	return nil
}

// sh runs a script in a running computer, as root.
func sh(ctx context.Context, d computer, script string) error {
	var out bytes.Buffer
	return in(ctx, d, strings.NewReader(script), &out, "sh", "-s")
}

// command runs a runtime's command, its errors carrying what it said.
func command(ctx context.Context, name string, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, name, args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("%s %s: %v: %s", name, args[0], err, strings.TrimSpace(stderr.String()))
	}
	return out, nil
}

// execIn runs a command in a container with a runtime's exec.
func execIn(ctx context.Context, runtime, container string, stdin io.Reader, stdout io.Writer, args ...string) error {
	cmd := exec.CommandContext(ctx, runtime, append([]string{"exec", "--interactive", "--user", "root", container}, args...)...)
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

func (d computer) local() string { return fmt.Sprintf("http://127.0.0.1:%d", d.port) }

// ready waits for a computer's server to answer.
func ready(ctx context.Context, d computer) error {
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
			return fmt.Errorf("computer %s didn't start: %w", d.Name, ctx.Err())
		case <-time.After(time.Second):
		}
	}
}

// tailscale runs a tailscale command. Changing what it serves is refused to
// an operator once anything on the machine serves a Unix socket, so then it
// runs as root, where sudo needs no password.
func tailscale(ctx context.Context, args ...string) (string, error) {
	out, err := exec.CommandContext(ctx, "tailscale", args...).CombinedOutput()
	if err != nil && strings.Contains(string(out), "must be root") {
		out, err = exec.CommandContext(ctx, "sudo", append([]string{"-n", "tailscale"}, args...)...).CombinedOutput()
	}
	if err != nil {
		return "", fmt.Errorf("tailscale %s: %v: %s", args[0], err, strings.TrimSpace(string(out)))
	}
	return string(out), nil
}
