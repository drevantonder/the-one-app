package core

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/drevantonder/the-one-app/internal/store"
	"github.com/drevantonder/the-one-app/protocol"
)

type forwarded struct{ device, input string }

type fakeCoord struct{ calls []forwarded }

func (f *fakeCoord) Acquire(context.Context, string) (string, bool, error) { return "", true, nil }
func (f *fakeCoord) ClaimRun(context.Context, string, time.Duration) (bool, error) {
	return true, nil
}
func (f *fakeCoord) Flush(context.Context) error { return nil }
func (f *fakeCoord) Forward(_ context.Context, device, _, _ string, input json.RawMessage, _ protocol.Caller) (protocol.Result, error) {
	f.calls = append(f.calls, forwarded{device, string(input)})
	return protocol.Result{Text: "ok"}, nil
}

func TestDeviceArgument(t *testing.T) {
	dir := t.TempDir()
	st, err := store.Open(filepath.Join(dir, "one.db"))
	if err != nil {
		t.Fatal(err)
	}
	n, err := New(st, dir, nil, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil {
		t.Fatal(err)
	}
	coord := &fakeCoord{}
	n.SetCoordinator(coord)
	n.SetName("mac")
	run := Op{Plugin: "shell", Name: "run", Description: "Run a command.", InputSchema: json.RawMessage(`{"type":"object","properties":{"command":{"type":"string"}}}`)}
	own := Op{Plugin: "phone", Name: "ring", InputSchema: json.RawMessage(`{"type":"object","properties":{"device":{"type":"string"}}}`)}
	n.SetRemoteOps([]RemoteOp{
		{Op: run, Device: "id-linux", DeviceName: "linux"},
		{Op: run, Device: "id-pi", DeviceName: "pi"},
		{Op: own, Device: "id-pi", DeviceName: "pi"},
	})

	// Each operation is listed once, with every device that has it.
	var listed []Op
	for _, o := range n.Operations() {
		if o.Plugin == "shell" {
			listed = append(listed, o)
		}
	}
	if len(listed) != 1 || !strings.HasSuffix(listed[0].Description, "Available on: linux, pi.") {
		t.Fatalf("listed %+v", listed)
	}
	var schema struct {
		Properties map[string]struct {
			Enum []string `json:"enum"`
		} `json:"properties"`
	}
	_ = json.Unmarshal(listed[0].InputSchema, &schema)
	if got := schema.Properties["device"].Enum; strings.Join(got, ",") != "linux,pi" {
		t.Fatalf("device enum = %v", got)
	}

	ctx := context.Background()
	call := func(plugin, op, input string) error {
		_, err := n.Call(ctx, plugin, op, json.RawMessage(input), protocol.Caller{})
		return err
	}
	// Named: it runs there, without the device argument.
	if err := call("shell", "run", `{"command":"ls","device":"pi"}`); err != nil {
		t.Fatal(err)
	}
	// Not named, and not here: a device that has it.
	if err := call("shell", "run", `{"command":"ls"}`); err != nil {
		t.Fatal(err)
	}
	// A plugin's own device argument is left alone.
	if err := call("phone", "ring", `{"device":"kitchen"}`); err != nil {
		t.Fatal(err)
	}
	want := []forwarded{{"id-pi", `{"command":"ls"}`}, {"id-linux", `{"command":"ls"}`}, {"id-pi", `{"device":"kitchen"}`}}
	if len(coord.calls) != len(want) {
		t.Fatalf("forwarded %+v", coord.calls)
	}
	for i, w := range want {
		if coord.calls[i] != w {
			t.Errorf("call %d forwarded %+v, want %+v", i, coord.calls[i], w)
		}
	}
	// A device without it: the error says where it is.
	if err := call("shell", "run", `{"command":"ls","device":"tv"}`); err == nil || !strings.Contains(err.Error(), "it is on: linux, pi") {
		t.Fatalf("got %v", err)
	}
	// This device, which doesn't have it: no quiet fallback elsewhere.
	if err := call("shell", "run", `{"command":"ls","device":"mac"}`); err == nil || !strings.Contains(err.Error(), "it is on: linux, pi") {
		t.Fatalf("running on this device, which lacks it: got %v", err)
	}
}
