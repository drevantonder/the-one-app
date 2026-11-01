package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestSealedProfileOpensOnlyWithItsKeyAndSalt(t *testing.T) {
	plain := []byte("a profile's tarball")
	sealed, err := seal("key", "salt", plain)
	if err != nil || bytes.Contains(sealed, plain) {
		t.Fatalf("%v %q", err, sealed)
	}
	if got, err := unseal("key", "salt", sealed); err != nil || !bytes.Equal(got, plain) {
		t.Errorf("%v %q", err, got)
	}
	if _, err := unseal("key", "other salt", sealed); err == nil {
		t.Error("each save's salt makes its own key")
	}
	if _, err := unseal("other key", "salt", sealed); err == nil {
		t.Error("only its item's key opens it")
	}
}

func TestProfileFolders(t *testing.T) {
	for title, want := range map[string]string{"PTC": "PTC", "Work stuff": "Work-stuff", " ../x ": "x", "": "profile"} {
		if got := folder(title); got != want {
			t.Errorf("folder(%q) = %q, want %q", title, got, want)
		}
	}
	if _, err := profileIn(map[string]any{"profile": map[string]any{"item": "0123456789abcdef", "name": "PTC", "key": "{{0123456789abcdef:profile_key}}"}}); err == nil {
		t.Error("a key the node didn't fill in is refused")
	}
}

// The script that reads a profile's History gives each site once, with the day it was last used, newest first.
func TestHistorySites(t *testing.T) {
	py, err := exec.LookPath("python3")
	if err != nil {
		t.Skip("no python3")
	}
	db := filepath.Join(t.TempDir(), "History")
	// Chromium's time: microseconds since 1601. 13406000000000000 is 2025-10-27.
	setup := `import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("create table urls (url text, last_visit_time integer)")
c.executemany("insert into urls values (?, ?)", [
  ("https://outlook.live.com/mail/", 13406000000000000),
  ("https://www.messenger.com/t/1", 13405900000000000),
  ("https://m.messenger.com/", 13405000000000000),
  ("https://my.gov.au/x", 13404000000000000),
  ("chrome://settings", 13407000000000000),
  ("http://10.0.0.1/", 13407000000000000)])
c.commit()`
	if out, err := exec.Command(py, "-c", setup, db).CombinedOutput(); err != nil {
		t.Fatalf("%v %s", err, out)
	}
	script := filepath.Join(t.TempDir(), "history.py")
	os.WriteFile(script, []byte(history), 0o600)
	out, err := exec.Command(py, script, db).Output()
	if err != nil {
		t.Fatal(err)
	}
	if got, want := string(bytes.TrimSpace(out)), "live.com 2025-10-27, messenger.com 2025-10-25, my.gov.au 2025-10-03"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
