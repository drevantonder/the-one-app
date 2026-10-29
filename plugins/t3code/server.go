package t3code

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	goruntime "runtime"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/plugins/internal/cli"
)

// desktopApp is the T3 Code desktop app on this Mac, if any. It runs its
// own server, which a background service would compete with.
func desktopApp() string {
	if goruntime.GOOS != "darwin" {
		return ""
	}
	for _, name := range []string{"T3 Code.app", "T3 Code (Beta).app"} {
		if _, err := os.Stat(filepath.Join("/Applications", name)); err == nil {
			return name
		}
	}
	return ""
}

// t3 finds the t3 command: the installer's, the desktop app's launcher, or
// the newest version the service runs.
func t3() (string, error) {
	if p, err := cli.Which("t3"); err == nil {
		return p, nil
	}
	home, _ := os.UserHomeDir()
	if p := filepath.Join(home, ".t3", "bin", "t3"); executable(p) {
		return p, nil
	}
	versions, _ := filepath.Glob(filepath.Join(home, ".t3", "runtime", "versions", "*", "t3"))
	for i := len(versions) - 1; i >= 0; i-- {
		if executable(versions[i]) {
			return versions[i], nil
		}
	}
	return "", errors.New("T3 Code isn't installed here")
}

func executable(p string) bool {
	info, err := os.Stat(p)
	return err == nil && !info.IsDir() && info.Mode()&0o111 != 0
}

func runT3(ctx context.Context, args ...string) (string, error) {
	bin, err := t3()
	if err != nil {
		return "", err
	}
	return cli.Run(ctx, bin, args...)
}

// serve installs T3 Code if missing and, unless the desktop app serves,
// installs its background service if it isn't.
func serve(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	if _, err := t3(); err != nil {
		if desktopApp() != "" {
			return errors.New("the desktop app's t3 command is missing: open the app once")
		}
		if _, err := cli.Run(ctx, "sh", "-c", "curl -fsSL https://t3.codes/install.sh | sh"); err != nil {
			return err
		}
	}
	if desktopApp() != "" {
		return nil
	}
	if out, err := runT3(ctx, "service", "status"); err == nil && !strings.Contains(out, "not installed") {
		return nil
	}
	_, err := runT3(ctx, "service", "install")
	return err
}

// pairLink mints a one-time pairing link over Tailscale HTTPS, which also
// keeps the server published there.
func pairLink(ctx context.Context, label string) (string, error) {
	if strings.TrimSpace(label) == "" {
		return "", errors.New("say which app the link is for")
	}
	ctx, cancel := context.WithTimeout(ctx, time.Minute)
	defer cancel()
	out, err := runT3(ctx, "pair", "--tailscale", "--ttl", "15m", "--label", label)
	if err != nil {
		return "", err
	}
	for line := range strings.Lines(out) {
		if url, ok := strings.CutPrefix(strings.TrimSpace(line), "Pairing URL:"); ok {
			return strings.TrimSpace(url), nil
		}
	}
	return "", fmt.Errorf("t3 pair gave no link: %s", cli.Tail(out))
}

type session struct {
	ID            string    `json:"id"`
	Client        string    `json:"client"`
	Subject       string    `json:"subject,omitempty"`
	LastConnected time.Time `json:"lastConnected,omitzero"`
	Expires       time.Time `json:"expires,omitzero"`
}

// sessions parses `t3 auth session list`: each session is its id on a line
// of its own, then indented "key: value" lines.
func sessions(out string) []session {
	var ss []session
	for line := range strings.Lines(out) {
		if !strings.HasPrefix(line, " ") {
			if id := strings.TrimSpace(line); id != "" {
				ss = append(ss, session{ID: id})
			}
			continue
		}
		if len(ss) == 0 {
			continue
		}
		key, val, ok := strings.Cut(strings.TrimSpace(line), ": ")
		if !ok {
			continue
		}
		s := &ss[len(ss)-1]
		switch key {
		case "client":
			s.Client = val
		case "subject":
			s.Subject = val
		case "last connected":
			s.LastConnected, _ = time.Parse(time.RFC3339, val)
		case "expires":
			s.Expires, _ = time.Parse(time.RFC3339, val)
		}
	}
	return ss
}

// apps are the sessions of apps paired with a one-time link, not outside
// agents.
func apps(ss []session) []session {
	var out []session
	for _, s := range ss {
		if s.Subject == "one-time-token" {
			out = append(out, s)
		}
	}
	return out
}
