package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// podmanRT runs devices as rootless Podman containers, in the user's own
// systemd instance rather than this service, so restarting it leaves them
// running; with --restart=always and podman-restart they start again after a
// reboot.
type podmanRT struct{}

// newPodman points Podman at the user's runtime directory and systemd, which a
// service has no login session for; lingering keeps them.
func newPodman() (containers, error) {
	if _, err := exec.LookPath("podman"); err != nil {
		return nil, fmt.Errorf("needs Podman")
	}
	if os.Getenv("XDG_RUNTIME_DIR") == "" {
		dir := fmt.Sprintf("/run/user/%d", os.Getuid())
		if _, err := os.Stat(dir); err != nil {
			return nil, fmt.Errorf("Podman needs %s: run loginctl enable-linger %s", dir, os.Getenv("USER"))
		}
		os.Setenv("XDG_RUNTIME_DIR", dir)
		os.Setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path="+dir+"/bus")
	}
	// Containers with --restart=always start again at boot through this. At
	// boot the user's systemd may not answer yet; revive covers that.
	if out, err := exec.Command("systemctl", "--user", "enable", "podman-restart.service").CombinedOutput(); err != nil {
		log.Printf("enabling podman-restart: %v: %s", err, strings.TrimSpace(string(out)))
	}
	return podmanRT{}, nil
}

func podman(ctx context.Context, args ...string) ([]byte, error) {
	return command(ctx, "podman", args...)
}

func (podmanRT) list(ctx context.Context) ([]device, error) {
	out, err := podman(ctx, "ps", "--all", "--filter", "label=one.token", "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		Names     []string
		Image     string
		State     string
		StartedAt int64
		Labels    map[string]string
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	var all []device
	for _, r := range raw {
		if len(r.Names) == 0 || !strings.HasPrefix(r.Names[0], prefix) {
			continue
		}
		d := device{Name: strings.TrimPrefix(r.Names[0], prefix), State: "stopped", token: r.Labels["one.token"], pool: r.Labels["one.pool"] != "", image: r.Image}
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

func (podmanRT) run(ctx context.Context, d device, sz size, image string) error {
	c := prefix + d.Name
	args := append([]string{"run", "--detach", "--name", c, "--restart=always"}, d.labels()...)
	args = append(args, sz.limits()...)
	_, err := podman(ctx, append(args,
		"--env", "CUA_ENV_TOKEN="+d.token,
		// The image says HOME=/root, which its user inherits; there
		// Chrome can't keep crash reports, and won't start.
		"--env", "HOME="+home,
		"--publish", fmt.Sprintf("127.0.0.1:%d:3211", d.port),
		// A new named volume starts as a copy of the image's home folder.
		"--volume", c+"-home:"+home,
		"--shm-size=1g",
		image)...)
	return err
}

func (podmanRT) hasImage(ctx context.Context, image string) bool {
	_, err := podman(ctx, "image", "inspect", image)
	return err == nil
}

// build pulls Cua's image again first, and then deletes the one it replaced.
// Docker's format keeps Cua's health check, which OCI's drops.
func (podmanRT) build(ctx context.Context, image, dir string) error {
	if _, err := podman(ctx, "build", "--pull=always", "--format", "docker", "--tag", image, dir); err != nil {
		return err
	}
	if _, err := podman(ctx, "image", "prune", "--force", "--filter", "label=org.opencontainers.image.source=https://github.com/trycua/cua"); err != nil {
		log.Print(err)
	}
	return nil
}

func (podmanRT) images(ctx context.Context) ([]string, error) {
	out, err := podman(ctx, "images", "--format", "{{.Repository}}:{{.Tag}}")
	return strings.Fields(string(out)), err
}

func (podmanRT) removeImage(ctx context.Context, image string) error {
	_, err := podman(ctx, "image", "rm", image)
	return err
}

func (podmanRT) start(ctx context.Context, d device) error {
	if _, err := podman(ctx, "update", "--restart=always", prefix+d.Name); err != nil {
		return err
	}
	_, err := podman(ctx, "start", prefix+d.Name)
	return err
}

func (podmanRT) stop(ctx context.Context, d device) error {
	if _, err := podman(ctx, "update", "--restart=no", prefix+d.Name); err != nil {
		return err
	}
	_, err := podman(ctx, "stop", "--time", "10", prefix+d.Name)
	return err
}

func (podmanRT) restart(ctx context.Context, d device) error {
	_, err := podman(ctx, "restart", "--time", "10", prefix+d.Name)
	return err
}

func (podmanRT) remove(ctx context.Context, d device) error {
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
	return nil
}

func (podmanRT) pause(ctx context.Context, d device) error {
	_, err := podman(ctx, "pause", prefix+d.Name)
	return err
}

func (podmanRT) unpause(ctx context.Context, d device) error {
	_, err := podman(ctx, "unpause", prefix+d.Name)
	return err
}

func (podmanRT) exec(ctx context.Context, d device, stdin io.Reader, stdout io.Writer, args ...string) error {
	return execIn(ctx, "podman", prefix+d.Name, stdin, stdout, args...)
}

func (podmanRT) limits(ctx context.Context, all []device) (map[string]limit, error) {
	out := map[string]limit{}
	if len(all) == 0 {
		return out, nil
	}
	args := []string{"inspect", "--format", "json"}
	for _, d := range all {
		args = append(args, prefix+d.Name)
	}
	raw, err := podman(ctx, args...)
	if err != nil {
		return nil, err
	}
	var got []struct {
		Name       string
		HostConfig struct {
			Memory        int64
			NanoCpus      int64
			RestartPolicy struct{ Name string }
		}
	}
	if err := json.Unmarshal(raw, &got); err != nil {
		return nil, err
	}
	for _, g := range got {
		out[strings.TrimPrefix(strings.TrimPrefix(g.Name, "/"), prefix)] = limit{CPUs: float64(g.HostConfig.NanoCpus) / 1e9, Memory: g.HostConfig.Memory,
			Restart: g.HostConfig.RestartPolicy.Name == "always"}
	}
	return out, nil
}

func (podmanRT) resize(ctx context.Context, d device, sz size) error {
	_, err := podman(ctx, append([]string{"update"}, append(sz.limits(), prefix+d.Name)...)...)
	return err
}

func (podmanRT) stats(ctx context.Context) (map[string]usage, error) {
	out, err := podman(ctx, "stats", "--no-stream", "--format", "json")
	if err != nil {
		return nil, err
	}
	var raw []struct {
		Name string `json:"name"`
		CPU  string `json:"cpu_percent"`
		Mem  string `json:"mem_usage"` // "88.25MB / 4.295GB"
	}
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, err
	}
	all := map[string]usage{}
	for _, r := range raw {
		name, ok := strings.CutPrefix(r.Name, prefix)
		if !ok {
			continue
		}
		var cpu float64
		fmt.Sscan(strings.TrimSuffix(r.CPU, "%"), &cpu)
		mem, _, _ := strings.Cut(r.Mem, " /")
		all[name] = usage{CPU: cpu, Memory: bytesOf(mem)}
	}
	return all, nil
}
