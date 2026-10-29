package spaces

import "testing"

func TestBytesOf(t *testing.T) {
	for in, want := range map[string]int64{"88.25MB": 88250000, "4.295GB": 4295000000, "512kB": 512000, "0B": 0, "1.5GiB": 1610612736} {
		if got := bytesOf(in); got != want {
			t.Errorf("bytesOf(%q) = %d, want %d", in, got, want)
		}
	}
}
