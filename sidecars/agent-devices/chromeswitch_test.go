package main

import "testing"

// Chrome is allowed on a machine that runs only its owner's calls, and not on one that runs anyone else's, until its
// owner chooses; what they chose holds either way.
func TestChromeDefault(t *testing.T) {
	yes, no := true, false
	for _, c := range []struct {
		owner   string
		runsFor []string
		chosen  *bool
		want    bool
	}{
		{"u1", []string{"user:u1"}, nil, true},
		{"u1", nil, nil, true},
		{"u1", []string{"user:u1", "global"}, nil, false},
		{"u1", []string{"user:u1", "group:g1"}, nil, false},
		{"u1", []string{"user:u1", "user:u2"}, nil, false},
		{"u1", []string{"user:u1", "global"}, &yes, true},
		{"u1", []string{"user:u1"}, &no, false},
	} {
		if got := chromeAllowed(c.chosen, runsShared(c.owner, c.runsFor)); got != c.want {
			t.Errorf("owner %s running %v, chosen %v: allowed %v, want %v", c.owner, c.runsFor, c.chosen, got, c.want)
		}
	}
}

// Only a machine's owner switches Chrome on or off there.
func TestMayChooseChrome(t *testing.T) {
	if err := mayChooseChrome("u1", "u1", "linux"); err != nil {
		t.Errorf("the owner was refused: %v", err)
	}
	if err := mayChooseChrome("u2", "u1", "linux"); err == nil {
		t.Error("someone else may switch it")
	}
	if err := mayChooseChrome("", "u1", "linux"); err == nil {
		t.Error("a call from no one may switch it")
	}
	if err := mayChooseChrome("u1", "", "linux"); err == nil {
		t.Error("it may be switched before its owner is known")
	}
}
