package memory

import (
	"fmt"
	"strings"
	"testing"
)

func fill(m *Memory, ids ...string) {
	for _, id := range ids {
		m.Add(id, "2026-10-28", "memory "+id)
	}
}

// napAll answers every pending compression the way an agent would.
func napAll(m *Memory) int {
	n := 0
	for {
		todo := m.Pending(1)
		if len(todo) == 0 {
			return n
		}
		b := todo[0]
		m.PutSummary(b, m.Digest(b), "summary of "+b.String())
		n++
	}
}

func TestCompressionsAreSmallestFirstAndComplete(t *testing.T) {
	m := New()
	for i := range 40 {
		fill(m, fmt.Sprintf("a:%d", i))
	}
	first := m.Pending(3)
	want := []block{{0, 2}, {2, 4}, {4, 6}}
	for i := range want {
		if first[i] != want[i] {
			t.Fatalf("pending = %v, want %v first", first, want)
		}
	}
	napAll(m)
	// Every aligned block up to the largest that fits is now settled.
	for size := 2; size <= m.T(); size *= 2 {
		for k := 0; k < m.T()/size; k++ {
			if _, ok := m.Summary(block{k * size, (k + 1) * size}); !ok {
				t.Fatalf("block %d-%d missing", k*size, (k+1)*size-1)
			}
		}
	}
}

// A note that arrives late from another device lands mid-log. Summaries that
// no longer match their memories must stop counting and be asked for again.
func TestLateArrivalInvalidatesOnlyShiftedSummaries(t *testing.T) {
	m := New()
	fill(m, "mac:1", "mac:2", "mac:3", "mac:4", "mac:5", "mac:6", "mac:7", "mac:8")
	napAll(m)

	// Rebuild the log as the merged order would replay it: linux:1 sorts
	// between mac:4 and mac:5. Old summary entries replay as before.
	merged := New()
	fill(merged, "mac:1", "mac:2", "mac:3", "mac:4", "linux:1", "mac:5", "mac:6", "mac:7", "mac:8")
	for b, list := range m.sums {
		for _, s := range list {
			merged.PutSummary(b, s.Digest, s.Text)
		}
	}
	if _, ok := merged.Summary(block{0, 4}); !ok {
		t.Fatal("0-3 covers the same memories and must stay valid")
	}
	if _, ok := merged.Summary(block{4, 6}); ok {
		t.Fatal("4-5 now covers different memories and must be invalid")
	}
	if got := merged.Pending(1); len(got) == 0 || got[0] != (block{4, 6}) {
		t.Fatalf("next compression = %v, want 4-5", got)
	}
	napAll(merged)
	if _, err := merged.Wake(1, 0); err != nil {
		t.Fatal(err)
	}
}

func TestForgetDropsTheSummaryAndEverythingBuiltOnIt(t *testing.T) {
	m := New()
	for i := range 8 {
		fill(m, fmt.Sprintf("a:%d", i))
	}
	napAll(m)
	m.Forget(block{2, 4})
	for _, b := range []block{{2, 4}, {0, 4}, {0, 8}} {
		if _, ok := m.Summary(b); ok {
			t.Errorf("%s should be gone", b)
		}
	}
	for _, b := range []block{{0, 2}, {4, 6}, {4, 8}} {
		if _, ok := m.Summary(b); !ok {
			t.Errorf("%s should remain", b)
		}
	}
	if got := m.Pending(1)[0]; got != (block{2, 4}) {
		t.Fatalf("next compression = %s, want 2-3", got)
	}
}

func TestWakeRefusesUntilNeededSummariesExist(t *testing.T) {
	m := New()
	for i := range 200 {
		fill(m, fmt.Sprintf("a:%d", i))
	}
	out, err := m.Wake(1, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(out, "Cannot wake") || !strings.Contains(out, "memory_nap") {
		t.Fatalf("expected a compression request, got:\n%s", out)
	}
	napAll(m)
	out, _ = m.Wake(1, 0)
	if !strings.HasSuffix(out, "You are awake.") {
		t.Fatalf("expected a full wake, got:\n%s", out)
	}
	if n := strings.Count(out, "\n#"); n+1 > WakeLines {
		t.Fatalf("wake printed %d lines, budget %d", n+1, WakeLines)
	}
}

func TestParseBlockAndCheckText(t *testing.T) {
	if b, err := ParseBlock("#16-31"); err != nil || b != (block{16, 32}) {
		t.Fatalf("ParseBlock = %v, %v", b, err)
	}
	for _, bad := range []string{"4-5x", "5-6", "0-2", "3"} {
		if _, err := ParseBlock(bad); err == nil {
			t.Errorf("ParseBlock(%q) should fail", bad)
		}
	}
	if _, err := CheckText(strings.Repeat("é", 141)); err == nil {
		t.Error("282 bytes should be too long")
	}
	if _, err := CheckText("two\nlines"); err == nil {
		t.Error("two lines should fail")
	}
}
