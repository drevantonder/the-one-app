package spaces

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// podman runs Spaces as rootless Podman containers. They run in the user's
// own systemd instance, not the node's service, so restarting the node
// leaves them running, and with --restart=always (and podman-restart) they
// come back after a reboot. That needs lingering: loginctl enable-linger.
type podman struct{}

func newPodman() (runtime, error) {
	if _, err := exec.LookPath("podman"); err != nil {
		return nil, fmt.Errorf("needs Podman (https://podman.io)")
	}
	// A service has no login session; point Podman at the user's runtime
	// directory and systemd, which lingering keeps.
	if os.Getenv("XDG_RUNTIME_DIR") == "" {
		dir := fmt.Sprintf("/run/user/%d", os.Getuid())
		if _, err := os.Stat(dir); err != nil {
			return nil, fmt.Errorf("Podman needs %s: run `loginctl enable-linger %s`", dir, os.Getenv("USER"))
		}
		os.Setenv("XDG_RUNTIME_DIR", dir)
		os.Setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path="+dir+"/bus")
	}
	// Containers with --restart=always start again at boot through this.
	_ = exec.Command("systemctl", "--user", "enable", "podman-restart.service").Run()
	return podman{}, nil
}

func (podman) cmd(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, "podman", args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("podman %s: %v: %s", args[0], err, strings.TrimSpace(stderr.String()))
	}
	return out, nil
}

func (p podman) list(ctx context.Context) ([]space, error) {
	out, err := p.cmd(ctx, "ps", "--all", "--filter", "label=one.space", "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		State     string            `json:"State"`
		Created   int64             `json:"Created"`
		StartedAt int64             `json:"StartedAt"`
		Labels    map[string]string `json:"Labels"`
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	var spaces []space
	for _, r := range raw {
		s := space{Name: r.Labels["one.space"], State: r.State, Created: time.Unix(r.Created, 0), token: r.Labels["one.token"]}
		if s.State != "running" {
			s.State = "stopped"
		}
		if r.StartedAt > 0 {
			s.LastUsed = time.Unix(r.StartedAt, 0)
		}
		s.port, _ = strconv.Atoi(r.Labels["one.port"])
		spaces = append(spaces, s)
	}
	return spaces, nil
}

func (p podman) run(ctx context.Context, s space) error {
	c := prefix + s.Name
	_, err := p.cmd(ctx, "run", "--detach", "--name", c, "--restart=always",
		"--label", "one.space="+s.Name, "--label", fmt.Sprintf("one.port=%d", s.port), "--label", "one.token="+s.token,
		"--env", "CUA_ENV_TOKEN="+s.token,
		// The image says HOME=/root, which its desktop user inherits; there
		// Chromium can't keep crash reports, and won't start.
		"--env", "HOME="+home,
		"--publish", fmt.Sprintf("127.0.0.1:%d:3211", s.port),
		// A new named volume starts as a copy of the image's home folder.
		"--volume", c+"-home:"+home,
		"--shm-size=1g", "--memory=4g",
		image)
	return err
}

func (p podman) start(ctx context.Context, name string) error {
	_, err := p.cmd(ctx, "start", prefix+name)
	return err
}

func (p podman) stop(ctx context.Context, name string) error {
	_, err := p.cmd(ctx, "stop", "--time", "10", prefix+name)
	return err
}

func (p podman) remove(ctx context.Context, name string) error {
	if _, err := p.cmd(ctx, "rm", "--force", "--ignore", prefix+name); err != nil {
		return err
	}
	_, err := p.cmd(ctx, "volume", "rm", "--force", prefix+name+"-home")
	if err != nil && strings.Contains(err.Error(), "no such volume") {
		return nil
	}
	return err
}

func (podman) exec(ctx context.Context, name string, stdin io.Reader, stdout io.Writer, args ...string) error {
	cmd := exec.CommandContext(ctx, "podman", append([]string{"exec", "--interactive", "--user", "root", prefix + name}, args...)...)
	cmd.Stdin, cmd.Stdout = stdin, stdout
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return &execError{err: err, stderr: strings.TrimSpace(stderr.String())}
	}
	return nil
}

// execError keeps a command's exit code, which some callers accept.
type execError struct {
	err    error
	stderr string
}

func (e *execError) Error() string { return fmt.Sprintf("%v: %s", e.err, e.stderr) }
func (e *execError) Unwrap() error { return e.err }
