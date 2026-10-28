package core

import (
	"testing"

	"github.com/drevantonder/the-one-app/protocol"
)

func TestMergedOrderIsClockThenDeviceThenSeq(t *testing.T) {
	list := []protocol.Entry{
		{Device: "b", Seq: 1, HLC: 20},
		{Device: "a", Seq: 2, HLC: 20},
		{Device: "a", Seq: 1, HLC: 10},
	}
	sortEntries(list)
	want := []string{"a1", "a2", "b1"}
	for i, e := range list {
		if got := e.Device + string(rune('0'+e.Seq)); got != want[i] {
			t.Fatalf("position %d = %s, want %s", i, got, want[i])
		}
	}
}
