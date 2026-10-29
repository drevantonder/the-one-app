package devices

import (
	"slices"
	"testing"
)

func TestNest(t *testing.T) {
	devices := []map[string]any{{"name": "personal"}, {"name": "mac"}, {"name": "linux"}, {"name": "ptc"}}
	usages := []*Usage{{}, nil, {Guests: []string{"personal", "ptc", "stremio"}}, {}}
	var names []string
	for _, d := range nest(devices, usages) {
		names = append(names, d["name"].(string))
	}
	if want := []string{"mac", "linux", "personal", "ptc"}; !slices.Equal(names, want) {
		t.Fatalf("got %v, want %v", names, want)
	}
	if devices[0]["host"] != "linux" || devices[1]["host"] != nil {
		t.Fatalf("hosts: %v", devices)
	}
}
