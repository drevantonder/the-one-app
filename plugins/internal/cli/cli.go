// Package cli runs the command-line tools plugins install for the user,
// finding them where their installers put them even when the node was
// started without those folders in PATH.
package cli

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

// Path is PATH with the folders installers use first.
func Path() string {
	home, _ := os.UserHomeDir()
	return strings.Join([]string{filepath.Join(home, ".local", "bin"), "/opt/homebrew/bin", "/usr/local/bin", os.Getenv("PATH")}, string(os.PathListSeparator))
}

// Which finds a tool in Path.
func Which(name string) (string, error) {
	for _, dir := range filepath.SplitList(Path()) {
		p := filepath.Join(dir, name)
		if info, err := os.Stat(p); err == nil && !info.IsDir() && info.Mode()&0o111 != 0 {
			return p, nil
		}
	}
	return "", exec.ErrNotFound
}

// Command is exec.CommandContext for a tool found with Which, run with Path.
func Command(ctx context.Context, name string, args ...string) *exec.Cmd {
	if !filepath.IsAbs(name) {
		if p, err := Which(name); err == nil {
			name = p
		}
	}
	cmd := exec.CommandContext(ctx, name, args...)
	cmd.Env = append(os.Environ(), "PATH="+Path())
	cmd.Env = append(cmd.Env, userSession()...)
	return cmd
}

// userSession is what tools need to reach the user's systemd and D-Bus
// when the node was started without it, as a system service is.
func userSession() []string {
	if runtime.GOOS != "linux" || os.Getenv("XDG_RUNTIME_DIR") != "" {
		return nil
	}
	dir := fmt.Sprintf("/run/user/%d", os.Getuid())
	if _, err := os.Stat(dir); err != nil {
		return nil
	}
	env := []string{"XDG_RUNTIME_DIR=" + dir}
	if os.Getenv("DBUS_SESSION_BUS_ADDRESS") == "" {
		env = append(env, "DBUS_SESSION_BUS_ADDRESS=unix:path="+dir+"/bus")
	}
	return env
}

// Run runs a tool and returns its combined output; on failure the error
// ends with the output's tail.
func Run(ctx context.Context, name string, args ...string) (string, error) {
	var out bytes.Buffer
	cmd := Command(ctx, name, args...)
	cmd.Stdout, cmd.Stderr = &out, &out
	if err := cmd.Run(); err != nil {
		return out.String(), fmt.Errorf("%s %s: %w: %s", filepath.Base(name), strings.Join(args, " "), err, Tail(out.String()))
	}
	return out.String(), nil
}

// Tail is the end of a tool's output, short enough for an error.
func Tail(s string) string {
	s = strings.TrimSpace(s)
	if len(s) > 500 {
		s = "…" + s[len(s)-500:]
	}
	return s
}
