package client

import (
	"strings"
	"testing"
)

// The library's functions that need no network, through purego: a RustBuffer each way, and an error's.
func TestCalls(t *testing.T) {
	s := NewDeviceSecret()
	if len(s) != 32 {
		t.Fatalf("secret is %d bytes", len(s))
	}
	j, err := DeviceJoin(s)
	if err != nil || j == "" {
		t.Fatalf("join %q %v", j, err)
	}
	if _, err := DeviceJoin([]byte{1, 2}); err == nil || !strings.Contains(err.Error(), "32 bytes") {
		t.Fatalf("a short key: %v", err)
	}
	if page := ClientScreen("settings", "{}"); !strings.Contains(page, "<") {
		t.Fatalf("screen %q", page[:min(80, len(page))])
	}
	d, err := NewThisDevice(s, "http://127.0.0.1:1", t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	if k := d.Key(); len(k) != 64 {
		t.Fatalf("key %q", k)
	}
	if c := d.Code(); len(c) != 9 {
		t.Fatalf("code %q", c)
	}
	if d.Kept("a", "b", []byte("{}")) != nil {
		t.Fatal("kept something")
	}
}
