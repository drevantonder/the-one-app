package main

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
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

// A save or backup leaves out Chrome's caches and locks, and keeps everything
// else, extensions' scripts and folders named like a cache that aren't one.
func TestSavesLeaveOutOnlyChromesCaches(t *testing.T) {
	dir := t.TempDir()
	at := strings.TrimPrefix(profiles, home+"/")
	files := map[string]bool{ // kept?
		".cache/x":                                       false,
		"projects/Cache/notes":                           true,
		at + "PTC/Local State":                           true,
		at + "PTC/SingletonLock":                         false,
		at + "PTC/GPUPersistentCache/GPUCache/x":         false,
		at + "PTC/Default/Cookies":                       true,
		at + "PTC/Default/Cache/Cache_Data/x":            false,
		at + "PTC/Default/Code Cache/js/x":               false,
		at + "PTC/Default/Service Worker/CacheStorage/x": false,
		at + "PTC/Default/Service Worker/ScriptCache/x":  true,
		at + "PTC/Default/Extensions/id/1.0_0/Cache/x":   true,
	}
	for f := range files {
		os.MkdirAll(filepath.Join(dir, filepath.Dir(f)), 0o755)
		os.WriteFile(filepath.Join(dir, f), nil, 0o644)
	}
	kept := func(from string, args []string) []string {
		out, err := exec.Command("sh", append([]string{"-c", `tar -C "$0" -cf - "$@" . | tar -tf - | grep -v '/$'`, from}, args...)...).Output()
		if err != nil {
			t.Fatal(err)
		}
		return strings.Fields(strings.ReplaceAll(string(out), " ", "_"))
	}
	backup := kept(dir, excludes("./"+at+"*", "./.cache"))
	save := kept(filepath.Join(dir, at, "PTC"), excludes("."))
	for f, keep := range files {
		in := "./" + strings.ReplaceAll(f, " ", "_")
		if slices.Contains(backup, in) != keep {
			t.Errorf("backup kept %s: %v", f, !keep)
		}
		if p, ok := strings.CutPrefix(in, "./"+at+"PTC/"); ok && slices.Contains(save, "./"+p) != keep {
			t.Errorf("save kept %s: %v", f, !keep)
		}
	}
}
