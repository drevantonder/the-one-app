package main

import "testing"

func TestStringField(t *testing.T) {
	// ticket "ab" (1), expires_at {seconds: 5} (2), viewer_path "/viewer/#ticket=ab" (3)
	m := []byte{0x0a, 2, 'a', 'b', 0x12, 2, 0x08, 5, 0x1a, 18}
	m = append(m, "/viewer/#ticket=ab"...)
	if got := stringField(m, 3); got != "/viewer/#ticket=ab" {
		t.Errorf("field 3: %q", got)
	}
	if got := stringField(m, 1); got != "ab" {
		t.Errorf("field 1: %q", got)
	}
	if got := stringField(m[:12], 3); got != "" {
		t.Errorf("a truncated message must give nothing, got %q", got)
	}
}
