package spaces

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// apple runs Spaces with Apple's container (github.com/apple/container),
// each in a light VM of its own. Its server is reached through a Unix socket
// the VM publishes on the Mac, rather than over the VM's network: macOS
// treats that network as the local network, which a background service
// can't be granted. A small bridge in the Space joins the socket to the
// server, and the plugin serves the socket on 127.0.0.1:<port>.
type apple struct {
	mu       sync.Mutex
	forwards map[string]net.Listener // by Space
}

const spaceSocket = "/run/spacesd.sock" // the bridge's end, in the Space

// bridge joins spaceSocket to the server. It runs in the Space, which has
// Python, and is started again whenever the Space has restarted.
const bridge = `
import os, socket, threading
path = "` + spaceSocket + `"
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

func newApple() (runtime, error) {
	if _, err := exec.LookPath("container"); err != nil {
		return nil, fmt.Errorf("needs Apple's container (brew install container; brew services start container)")
	}
	a := &apple{forwards: map[string]net.Listener{}}
	if spaces, err := a.list(context.Background()); err == nil {
		for _, s := range spaces {
			a.forward(s.Name, s.port)
		}
	}
	return a, nil
}

// socket is where a Space's server is published on the Mac.
func socket(name string) string {
	dir, err := os.UserCacheDir()
	if err != nil {
		dir = os.TempDir()
	}
	return filepath.Join(dir, "TheOneApp", "spaces", name+".sock")
}

func (*apple) cmd(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, "container", args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("container %s: %v: %s", args[0], err, strings.TrimSpace(stderr.String()))
	}
	return out, nil
}

func (a *apple) list(ctx context.Context) ([]space, error) {
	out, err := a.cmd(ctx, "list", "--all", "--format", "json")
	if err != nil {
		return nil, err
	}
	var all []struct {
		Configuration struct {
			Labels       map[string]string `json:"labels"`
			CreationDate time.Time         `json:"creationDate"`
		} `json:"configuration"`
		Status struct {
			State       string    `json:"state"`
			StartedDate time.Time `json:"startedDate"`
		} `json:"status"`
	}
	if err := json.Unmarshal(out, &all); err != nil {
		return nil, err
	}
	var spaces []space
	for _, c := range all {
		l := c.Configuration.Labels
		if l["one.space"] == "" {
			continue
		}
		s := space{Name: l["one.space"], State: c.Status.State, Created: c.Configuration.CreationDate, token: l["one.token"]}
		if s.State != "running" {
			s.State = "stopped"
		}
		if !c.Status.StartedDate.IsZero() {
			s.LastUsed = c.Status.StartedDate
		}
		s.port, _ = strconv.Atoi(l["one.port"])
		spaces = append(spaces, s)
	}
	return spaces, nil
}

func (a *apple) run(ctx context.Context, s space) error {
	c, vol := prefix+s.Name, prefix+s.Name+"-home"
	if _, err := a.cmd(ctx, "volume", "create", vol); err != nil {
		return err
	}
	// A new volume is empty: copy the image's home folder in first.
	if _, err := a.cmd(ctx, "run", "--rm", "--entrypoint", "/bin/sh", "--volume", vol+":/mnt/home", image,
		"-c", "cp -a "+home+"/. /mnt/home/ && rm -rf /mnt/home/lost+found && chown 1000:1000 /mnt/home"); err != nil {
		return err
	}
	sock := socket(s.Name)
	if err := os.MkdirAll(filepath.Dir(sock), 0o700); err != nil {
		return err
	}
	_ = os.Remove(sock)
	if _, err := a.cmd(ctx, "run", "--detach", "--name", c,
		"--label", "one.space="+s.Name, "--label", fmt.Sprintf("one.port=%d", s.port), "--label", "one.token="+s.token,
		"--env", "CUA_ENV_TOKEN="+s.token,
		"--env", "HOME="+home, // see podman.run
		"--volume", vol+":"+home,
		"--publish-socket", sock+":"+spaceSocket,
		"--shm-size", "1G", "--memory", "4G", "--cpus", "2",
		image); err != nil {
		return err
	}
	a.forward(s.Name, s.port)
	return a.bridge(ctx, s.Name)
}

// bridge starts the bridge in a running Space.
func (a *apple) bridge(ctx context.Context, name string) error {
	_, err := a.cmd(ctx, "exec", "--detach", "--user", "root", prefix+name, "python3", "-c", bridge)
	return err
}

func (a *apple) start(ctx context.Context, name string) error {
	if _, err := a.cmd(ctx, "start", prefix+name); err != nil {
		return err
	}
	return a.bridge(ctx, name)
}

func (a *apple) stop(ctx context.Context, name string) error {
	_, err := a.cmd(ctx, "stop", prefix+name)
	return err
}

func (a *apple) remove(ctx context.Context, name string) error {
	a.mu.Lock()
	if ln := a.forwards[name]; ln != nil {
		ln.Close()
		delete(a.forwards, name)
	}
	a.mu.Unlock()
	if _, err := a.cmd(ctx, "delete", "--force", prefix+name); err != nil && !strings.Contains(err.Error(), "not found") {
		return err
	}
	if _, err := a.cmd(ctx, "volume", "delete", prefix+name+"-home"); err != nil && !strings.Contains(err.Error(), "not found") {
		return err
	}
	_ = os.Remove(socket(name))
	return nil
}

func (*apple) exec(ctx context.Context, name string, stdin io.Reader, stdout io.Writer, args ...string) error {
	cmd := exec.CommandContext(ctx, "container", append([]string{"exec", "--interactive", "--user", "root", prefix + name}, args...)...)
	cmd.Stdin, cmd.Stdout = stdin, stdout
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return &execError{err: err, stderr: strings.TrimSpace(stderr.String())}
	}
	return nil
}

// dial connects to a Space's server through its socket, starting the bridge
// once if it isn't answering (as after the Space restarted).
func (a *apple) dial(name string) (net.Conn, error) {
	c, err := net.DialTimeout("unix", socket(name), 2*time.Second)
	if err == nil {
		return c, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := a.bridge(ctx, name); err != nil {
		return nil, err
	}
	time.Sleep(500 * time.Millisecond)
	return net.DialTimeout("unix", socket(name), 2*time.Second)
}

// forward serves a Space on 127.0.0.1:port.
func (a *apple) forward(name string, port int) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.forwards[name] != nil || port == 0 {
		return
	}
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
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
					fmt.Fprintf(os.Stderr, "space %s: %v\n", name, err) // the node logs it
					return
				}
				defer backend.Close()
				go func() { _, _ = io.Copy(backend, conn); backend.(*net.UnixConn).CloseWrite() }()
				_, _ = io.Copy(conn, backend)
			}()
		}
	}()
}
