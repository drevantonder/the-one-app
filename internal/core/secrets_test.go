package core

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"path/filepath"
	"strings"
	"testing"

	"github.com/drevantonder/the-one-app/internal/store"
	"github.com/drevantonder/the-one-app/protocol"
)

func TestSecrets(t *testing.T) {
	dir := t.TempDir()
	st, err := store.Open(filepath.Join(dir, "one.db"))
	if err != nil {
		t.Fatal(err)
	}
	n, err := New(st, dir, nil, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	n.gens["models"] = &generation{manifest: protocol.Manifest{ID: "models", Secrets: []string{"OPENCODE_API_KEY"}}}
	n.gens["board"] = &generation{manifest: protocol.Manifest{ID: "board"}}
	ctx, caller := context.Background(), protocol.Caller{Actor: "test"}
	call := func(op, input string) (protocol.Result, error) {
		return n.callCore(ctx, op, []byte(input), caller)
	}

	if _, err := n.secret("models", "OPENCODE_API_KEY"); !errors.Is(err, errNoSecret) {
		t.Fatalf("unset: %v", err)
	}
	if _, err := call("secrets_set", `{"name":"opencode key","value":"x"}`); err == nil {
		t.Fatal("a name that isn't in capitals")
	}
	if _, err := call("secrets_set", `{"name":"OPENCODE_API_KEY","value":" old "}`); err != nil {
		t.Fatal(err)
	}
	if _, err := call("secrets_set", `{"name":"OPENCODE_API_KEY","value":"sk-new"}`); err != nil {
		t.Fatal(err)
	}
	if v, err := n.secret("models", "OPENCODE_API_KEY"); err != nil || v != "sk-new" {
		t.Fatalf("the latest value: %q %v", v, err)
	}
	if _, err := n.secret("board", "OPENCODE_API_KEY"); err == nil {
		t.Fatal("a plugin that doesn't declare a secret read it")
	}
	res, err := call("secrets_list", `{}`)
	if err != nil || res.Text != "OPENCODE_API_KEY: used by models" || strings.Contains(res.Text, "sk-new") {
		t.Fatalf("list: %q %v", res.Text, err)
	}
	if _, err := call("secrets_remove", `{"name":"OPENCODE_API_KEY"}`); err != nil {
		t.Fatal(err)
	}
	if res, _ := call("secrets_list", `{}`); res.Text != "OPENCODE_API_KEY (not set): used by models" {
		t.Fatalf("after removing: %q", res.Text)
	}
}
