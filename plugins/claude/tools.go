package claude

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/plugins/internal/cli"
)

var (
	which   = cli.Which
	command = cli.Command
	run     = cli.Run
	tail    = cli.Tail
)

func installed() bool {
	_, e1 := which("claude")
	_, e2 := which("claude-swap")
	return e1 == nil && e2 == nil
}

// install installs Claude Code, and uv to install claude-swap with, if
// missing, and installs or updates claude-swap.
func install(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	if _, err := which("claude"); err != nil {
		if _, err := run(ctx, "sh", "-c", "curl -fsSL https://claude.ai/install.sh | bash"); err != nil {
			return err
		}
	}
	if _, err := which("uv"); err != nil {
		if _, err := run(ctx, "sh", "-c", "curl -LsSf https://astral.sh/uv/install.sh | env UV_NO_MODIFY_PATH=1 sh"); err != nil {
			return err
		}
	}
	verb := "install"
	if _, err := which("claude-swap"); err == nil {
		verb = "upgrade"
	}
	_, err := run(ctx, "uv", "tool", verb, "claude-swap")
	return err
}

func versions(ctx context.Context) (claude, swap string) {
	if out, err := run(ctx, "claude", "--version"); err == nil {
		claude, _, _ = strings.Cut(strings.TrimSpace(out), " ")
	}
	if out, err := run(ctx, "claude-swap", "--version"); err == nil {
		for line := range strings.Lines(out) {
			if f := strings.Fields(line); len(f) == 2 && (f[0] == "cswap" || f[0] == "claude-swap") {
				swap = f[1]
			}
		}
	}
	return claude, swap
}

// accounts is the accounts claude-swap manages on this device.
func accounts(ctx context.Context) ([]account, error) {
	out, err := run(ctx, "claude-swap", "list", "--json")
	if err != nil {
		return nil, err
	}
	var l struct {
		Accounts []account `json:"accounts"`
	}
	if i := strings.Index(out, "{"); i >= 0 {
		out = out[i:]
	}
	if err := json.NewDecoder(strings.NewReader(out)).Decode(&l); err != nil {
		return nil, fmt.Errorf("claude-swap list: %w", err)
	}
	return l.Accounts, nil
}

// signedIn is the account Claude Code is signed in to here, if any.
func signedIn(ctx context.Context) string {
	out, err := run(ctx, "claude", "auth", "status")
	if err != nil {
		return ""
	}
	var st struct {
		LoggedIn bool   `json:"loggedIn"`
		Method   string `json:"authMethod"`
		Email    string `json:"email"`
	}
	if json.Unmarshal([]byte(out), &st) != nil || !st.LoggedIn || st.Method != "claude.ai" {
		return ""
	}
	return st.Email
}

// adopt adds the account Claude Code is signed in to to claude-swap, so
// signing in to another doesn't lose it. Adding one already there just
// refreshes its saved login.
func adopt(ctx context.Context) error {
	if signedIn(ctx) == "" {
		return nil
	}
	_, err := run(ctx, "claude-swap", "add")
	return err
}

func active(ctx context.Context) string {
	as, _ := accounts(ctx)
	for _, a := range as {
		if a.Active {
			return a.Email
		}
	}
	return ""
}

var errNoSwap = errors.New("claude-swap isn't installed here: make this a coding device, or run install")
