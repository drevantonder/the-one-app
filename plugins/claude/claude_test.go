package claude

import (
	"slices"
	"strings"
	"testing"
)

func TestAuthURL(t *testing.T) {
	out := "Opening browser to sign in…\nIf the browser didn't open, visit: \x1b]8;;https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y\x07https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y\x1b]8;;\x07\nPaste code here if prompted > "
	if got := authURL.FindString(out); got != "https://claude.com/cai/oauth/authorize?code=true&client_id=x&state=y" {
		t.Fatalf("got %q", got)
	}
}

func TestMissing(t *testing.T) {
	devices := []device{
		{Name: "mac", Online: true, Coding: true, Accounts: []account{{Email: "a@x"}, {Email: "b@x"}}},
		{Name: "ptc", Online: true, Coding: true, Accounts: []account{{Email: "b@x"}}},
		{Name: "linux", Online: true},
		{Name: "ipad", Coding: true},
	}
	got := missing(devices)
	if len(got) != 1 || !slices.Equal(got["ptc"], []string{"a@x"}) {
		t.Fatalf("got %v", got)
	}
	r := report(devices)
	for _, want := range []string{"ptc (coding)", "a@x not signed in here", "ipad (coding): offline", "linux: no Claude Code or claude-swap"} {
		if !strings.Contains(r, want) {
			t.Errorf("report lacks %q:\n%s", want, r)
		}
	}
}
