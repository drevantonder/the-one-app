package main

import (
	"encoding/json"
	"testing"

	"github.com/drevantonder/the-one-app/internal/core"
)

func TestBuildInputArrays(t *testing.T) {
	op := core.Op{Plugin: "p", Name: "o", InputSchema: json.RawMessage(`{"type":"object","properties":{"path":{"type":"array","items":{"type":"string"}}}}`)}
	for args, want := range map[string]string{
		`a,b`:                          `{"path":["a","b"]}`,
		`["Calculator","Quit, maybe"]`: `{"path":["Calculator","Quit, maybe"]}`,
	} {
		got, err := buildInput(op, []string{"--path", args})
		if err != nil || string(got) != want {
			t.Errorf("--path %s: got %s %v, want %s", args, got, err, want)
		}
	}
}
