package main

import (
	"os"
	"path/filepath"
	"testing"
)

// A Windows copy's base is its disk's backing file: deleting it while the copy is there, or writing a new one at its
// path, loses the copy. Bases go only once no copy is on them and copies aren't made on them now.
func TestBasesStayWhileACopyIsOnThem(t *testing.T) {
	dataDir = t.TempDir()
	ds := &devices{dir: dataDir}
	lasting := &vm{Name: "win", Kind: "windows", Dir: filepath.Join(dataDir, "windows", "win")}
	bases := basesOf(lasting)
	for _, gen := range []string{"used", "now", "stale"} {
		os.MkdirAll(filepath.Join(bases, gen), 0o700)
		os.WriteFile(filepath.Join(bases, gen, "disk.qcow2"), nil, 0o600)
	}
	os.WriteFile(filepath.Join(bases, "current"), []byte("now"), 0o600)
	os.WriteFile(winFile(lasting, "base.qcow2"), nil, 0o600) // from before bases had folders
	ds.saveVM(lasting)
	ds.saveVM(&vm{Name: "copy", Kind: "windows", Of: "win", Base: filepath.Join(bases, "used")})
	ds.saveVM(&vm{Name: "old-copy", Kind: "windows", Of: "win"})

	ds.pruneBases()
	for gen, kept := range map[string]bool{"used": true, "now": true, "stale": false} {
		if _, err := os.Stat(filepath.Join(bases, gen)); (err == nil) != kept {
			t.Errorf("base %s: kept %v, want %v", gen, err == nil, kept)
		}
	}
	if _, err := os.Stat(winFile(lasting, "base.qcow2")); err != nil {
		t.Error("the old base went while a copy is on it")
	}

	ds.forgetVM("copy")
	ds.forgetVM("old-copy")
	os.Remove(filepath.Join(bases, "current")) // it stopped
	ds.pruneBases()
	for _, gone := range []string{filepath.Join(bases, "used"), filepath.Join(bases, "now"), winFile(lasting, "base.qcow2")} {
		if _, err := os.Stat(gone); err == nil {
			t.Errorf("%s stayed with no copy on it", gone)
		}
	}
}
