package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// appleRT runs computers with Apple's container (github.com/apple/container),
// each in a light VM of its own, on a Mac. A computer's server is reached
// through a Unix socket the VM publishes on the Mac, not over the VM's
// network or a published port: macOS treats that network as the local
// network, which a background service isn't allowed to reach. A small bridge
// in the computer joins the socket to its server, and this serves the socket
// on 127.0.0.1:<port>. Apple's container can't pause or resize a container,
// and has no restart policy: revive starts computers not stopped with stop.
type appleRT struct {
	dir string // sockets, and which computers were stopped

	mu       sync.Mutex
	forwards map[string]net.Listener // by computer
	cpu      map[string]cpuSample    // last CPU reading, by computer
}

type cpuSample struct {
	usec int64
	at   time.Time
}

var errCant = errors.New("Apple's container can't do that")

const computerSocket = "/run/one-computer.sock" // the bridge's end, in the computer

// bridge joins computerSocket to the computer's server. It runs in the computer,
// which has Python, and is started again whenever the computer has restarted.
const bridge = `
import os, socket, threading
path = "` + computerSocket + `"
try: os.unlink(path)
except OSError: pass
s = socket.socket(socket.AF_UNIX); s.bind(path); s.listen(64)
def pipe(a, b):
    try:
        while (d := a.recv(65536)): b.sendall(d)
    except OSError: pass
    try: b.shutdown(socket.SHUT_WR)
    except OSError: pass
while True:
    c, _ = s.accept()
    try: u = socket.create_connection(("127.0.0.1", 3211))
    except OSError: c.close(); continue
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
`

func newApple(dir string) (containers, error) {
	if _, err := exec.LookPath("container"); err != nil {
		return nil, fmt.Errorf("needs Apple's container (brew install container; container system start)")
	}
	for _, d := range []string{"sockets", "stopped"} {
		if err := os.MkdirAll(filepath.Join(dir, d), 0o700); err != nil {
			return nil, err
		}
	}
	return &appleRT{dir: dir, forwards: map[string]net.Listener{}, cpu: map[string]cpuSample{}}, nil
}

func ctr(ctx context.Context, args ...string) ([]byte, error) {
	return command(ctx, "container", args...)
}

func (a *appleRT) socket(name string) string  { return filepath.Join(a.dir, "sockets", name+".sock") }
func (a *appleRT) stopped(name string) string { return filepath.Join(a.dir, "stopped", name) }

type appleContainer struct {
	Configuration struct {
		ID     string            `json:"id"`
		Labels map[string]string `json:"labels"`
		Image  struct {
			Reference string `json:"reference"`
		} `json:"image"`
		Resources struct {
			CPUs          float64 `json:"cpus"`
			MemoryInBytes int64   `json:"memoryInBytes"`
		} `json:"resources"`
	} `json:"configuration"`
	Status struct {
		State       string    `json:"state"`
		StartedDate time.Time `json:"startedDate"`
	} `json:"status"`
}

func (a *appleRT) containers(ctx context.Context) ([]appleContainer, error) {
	out, err := ctr(ctx, "list", "--all", "--format", "json")
	if err != nil {
		return nil, err
	}
	var all []appleContainer
	return all, json.Unmarshal(out, &all)
}

func (a *appleRT) list(ctx context.Context) ([]computer, error) {
	all, err := a.containers(ctx)
	if err != nil {
		return nil, err
	}
	var out []computer
	for _, c := range all {
		l := c.Configuration.Labels
		name, ok := strings.CutPrefix(c.Configuration.ID, prefix)
		if !ok || l["one.token"] == "" {
			continue
		}
		d := computer{Name: name, State: "stopped", Started: c.Status.StartedDate, token: l["one.token"], pool: l["one.pool"] != "", image: c.Configuration.Image.Reference}
		if c.Status.State == "running" {
			d.State = "running"
		}
		d.port, _ = strconv.Atoi(l["one.port"])
		a.forward(d.Name, d.port) // once the runtime answers, which at login can take a while
		out = append(out, d)
	}
	return out, nil
}

func (a *appleRT) run(ctx context.Context, d computer, sz size, image string) error {
	c, vol := prefix+d.Name, prefix+d.Name+"-home"
	if _, err := ctr(ctx, "volume", "create", vol); err != nil {
		return err
	}
	// A new volume is empty: copy the image's home folder in first.
	if _, err := ctr(ctx, "run", "--rm", "--entrypoint", "/bin/sh", "--volume", vol+":/mnt/home", image,
		"-c", "cp -a "+home+"/. /mnt/home/ && rm -rf /mnt/home/lost+found && chown 1000:1000 /mnt/home"); err != nil {
		return err
	}
	sock := a.socket(d.Name)
	os.Remove(sock)
	os.Remove(a.stopped(d.Name))
	args := append([]string{"run", "--detach", "--name", c}, d.labels()...)
	if _, err := ctr(ctx, append(args,
		"--env", "CUA_ENV_TOKEN="+d.token,
		"--env", "HOME="+home, // see podmanRT.run
		"--volume", vol+":"+home,
		"--publish-socket", sock+":"+computerSocket,
		"--shm-size", "1G", "--memory", fmt.Sprintf("%dG", sz.GB), "--cpus", strconv.Itoa(sz.CPUs),
		image)...); err != nil {
		return err
	}
	a.forward(d.Name, d.port)
	return a.bridge(ctx, d.Name)
}

// bridge starts the bridge in a running computer.
func (a *appleRT) bridge(ctx context.Context, name string) error {
	_, err := ctr(ctx, "exec", "--detach", "--user", "root", prefix+name, "python3", "-c", bridge)
	return err
}

func (a *appleRT) start(ctx context.Context, d computer) error {
	os.Remove(a.stopped(d.Name))
	if _, err := ctr(ctx, "start", prefix+d.Name); err != nil {
		return err
	}
	return a.bridge(ctx, d.Name)
}

func (a *appleRT) stop(ctx context.Context, d computer) error {
	if err := os.WriteFile(a.stopped(d.Name), nil, 0o600); err != nil {
		return err
	}
	_, err := ctr(ctx, "stop", prefix+d.Name)
	return err
}

func (a *appleRT) restart(ctx context.Context, d computer) error {
	if _, err := ctr(ctx, "stop", prefix+d.Name); err != nil {
		return err
	}
	if _, err := ctr(ctx, "start", prefix+d.Name); err != nil {
		return err
	}
	return a.bridge(ctx, d.Name)
}

func (a *appleRT) remove(ctx context.Context, d computer) error {
	a.mu.Lock()
	if ln := a.forwards[d.Name]; ln != nil {
		ln.Close()
		delete(a.forwards, d.Name)
	}
	a.mu.Unlock()
	if _, err := ctr(ctx, "delete", "--force", prefix+d.Name); err != nil && !strings.Contains(err.Error(), "not found") {
		return err
	}
	if _, err := ctr(ctx, "volume", "delete", prefix+d.Name+"-home"); err != nil && !strings.Contains(err.Error(), "not found") {
		return err
	}
	os.Remove(a.socket(d.Name))
	os.Remove(a.stopped(d.Name))
	return nil
}

func (a *appleRT) hasImage(ctx context.Context, image string) bool {
	_, err := ctr(ctx, "image", "inspect", image)
	return err == nil
}

func (a *appleRT) build(ctx context.Context, image, dir string) error {
	_, err := ctr(ctx, "build", "--tag", image, dir)
	return err
}

func (a *appleRT) images(ctx context.Context) ([]string, error) {
	out, err := ctr(ctx, "image", "list", "--format", "json")
	if err != nil {
		return nil, err
	}
	var all []struct {
		Reference string `json:"reference"`
	}
	if err := json.Unmarshal(out, &all); err != nil {
		return nil, err
	}
	var refs []string
	for _, i := range all {
		refs = append(refs, i.Reference)
	}
	return refs, nil
}

func (a *appleRT) removeImage(ctx context.Context, image string) error {
	_, err := ctr(ctx, "image", "delete", image)
	return err
}

func (a *appleRT) pause(context.Context, computer) error   { return errCant }
func (a *appleRT) unpause(context.Context, computer) error { return errCant }

func (a *appleRT) exec(ctx context.Context, d computer, stdin io.Reader, stdout io.Writer, args ...string) error {
	return execIn(ctx, "container", prefix+d.Name, stdin, stdout, args...)
}

func (a *appleRT) limits(ctx context.Context, _ []computer) (map[string]limit, error) {
	all, err := a.containers(ctx)
	if err != nil {
		return nil, err
	}
	out := map[string]limit{}
	for _, c := range all {
		if name, ok := strings.CutPrefix(c.Configuration.ID, prefix); ok {
			_, err := os.Stat(a.stopped(name))
			out[name] = limit{CPUs: c.Configuration.Resources.CPUs, Memory: c.Configuration.Resources.MemoryInBytes, Restart: err != nil}
		}
	}
	return out, nil
}

func (a *appleRT) resize(context.Context, computer, size) error {
	return fmt.Errorf("%w: back it up and restore it with another size", errCant)
}

// stats reads memory and, since the last reading, CPU.
func (a *appleRT) stats(ctx context.Context) (map[string]usage, error) {
	out, err := ctr(ctx, "stats", "--no-stream", "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		ID           string `json:"id"`
		CPUUsageUsec int64  `json:"cpuUsageUsec"`
		Memory       int64  `json:"memoryUsageBytes"`
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	all := map[string]usage{}
	now := time.Now()
	for _, r := range raw {
		name, ok := strings.CutPrefix(r.ID, prefix)
		if !ok {
			continue
		}
		u := usage{Memory: r.Memory}
		if last, ok := a.cpu[name]; ok && now.After(last.at) && r.CPUUsageUsec >= last.usec {
			u.CPU = float64(r.CPUUsageUsec-last.usec) / float64(now.Sub(last.at).Microseconds()) * 100
		}
		a.cpu[name] = cpuSample{usec: r.CPUUsageUsec, at: now}
		all[name] = u
	}
	return all, nil
}

// dial connects to a computer's server through its socket, starting the
// bridge once if it isn't answering, as after the computer restarted.
func (a *appleRT) dial(name string) (net.Conn, error) {
	c, err := net.DialTimeout("unix", a.socket(name), 2*time.Second)
	if err == nil {
		return c, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := a.bridge(ctx, name); err != nil {
		return nil, err
	}
	time.Sleep(500 * time.Millisecond)
	return net.DialTimeout("unix", a.socket(name), 2*time.Second)
}

// forward serves a computer on 127.0.0.1:port.
func (a *appleRT) forward(name string, port int) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.forwards[name] != nil || port == 0 {
		return
	}
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		log.Printf("serving %s: %v", name, err)
		return
	}
	a.forwards[name] = ln
	go func() {
		for {
			conn, err := ln.Accept()
			if err != nil {
				return
			}
			go func() {
				defer conn.Close()
				backend, err := a.dial(name)
				if err != nil {
					log.Printf("computer %s: %v", name, err)
					return
				}
				defer backend.Close()
				go func() { io.Copy(backend, conn); backend.(*net.UnixConn).CloseWrite() }()
				io.Copy(conn, backend)
			}()
		}
	}()
}
