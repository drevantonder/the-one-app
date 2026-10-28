package store

import (
	"encoding/json"
	"path/filepath"
	"testing"

	"github.com/drevantonder/the-one-app/protocol"
)

func open(t *testing.T) *Store {
	t.Helper()
	s, err := Open(filepath.Join(t.TempDir(), "one.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

// Two stores exchanging entries end up with the same log; entries are only
// accepted in sequence, so a gap is never stored.
func TestIngestKeepsEachDeviceLogContiguous(t *testing.T) {
	a, b := open(t), open(t)
	for range 3 {
		if _, err := a.Append("p", "x", json.RawMessage(`{}`), ""); err != nil {
			t.Fatal(err)
		}
	}
	all, err := a.After(nil, 100)
	if err != nil || len(all) != 3 {
		t.Fatalf("After = %d entries, %v", len(all), err)
	}
	// Out of order and with a gap: only #1 is acceptable at first.
	added, err := b.Ingest([]protocol.Entry{all[2], all[0]})
	if err != nil {
		t.Fatal(err)
	}
	if len(added) != 1 || added[0].Seq != 1 {
		t.Fatalf("added %v, want only seq 1", added)
	}
	added, _ = b.Ingest(all) // the rest, plus a duplicate
	if len(added) != 2 {
		t.Fatalf("added %d, want 2", len(added))
	}
	va, _ := a.Vector()
	vb, _ := b.Vector()
	if vb[a.Device()] != va[a.Device()] {
		t.Fatalf("vectors differ: %v vs %v", va, vb)
	}
	// b's next entry sorts after everything it has seen from a.
	e, _ := b.Append("p", "y", nil, "")
	if e.HLC <= all[2].HLC {
		t.Fatal("clock did not advance past ingested entries")
	}
	if rest, _ := b.After(va, 100); len(rest) != 1 || rest[0].Device != b.Device() {
		t.Fatalf("After(a's vector) = %v, want only b's entry", rest)
	}
}
