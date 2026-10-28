// Package sdk is how plugins are written. A plugin is a Go program whose main
// calls sdk.Run. The core starts it, replays the plugin's log entries into it,
// and routes operations and transforms to it.
//
//	func main() {
//		sdk.Run(sdk.Plugin{ID: "habits", Version: "0.1.0", Setup: func(c *sdk.Ctx) error {
//			c.Entry("checkIn", sdk.Available)
//			sdk.Handle(c, "checkIn", func(e sdk.Entry, in CheckIn) error { ...; return nil })
//			sdk.Op(c, "checkIn", "Record a habit check-in.", sdk.Write, func(call *sdk.Call, in CheckIn) (sdk.Result, error) {
//				_, err := call.Append("checkIn", in)
//				return sdk.Text("Done."), err
//			})
//			return nil
//		}})
//	}
//
// Plugin state lives in memory and is rebuilt from the log every time the
// plugin starts, so a plugin never has to migrate stored state.
package sdk

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/google/jsonschema-go/jsonschema"

	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/rpc"
)

type (
	Mode   = protocol.Mode
	Entry  = protocol.Entry
	Result = protocol.Result
	Caller = protocol.Caller
)

const (
	Available  = protocol.Available
	Consistent = protocol.Consistent
)

type Plugin struct {
	ID      string
	Version string
	Setup   func(*Ctx) error
}

// Ctx is what Setup registers against.
type Ctx struct {
	id         string
	entries    []protocol.EntryType
	ops        []protocol.Operation
	opFns      map[string]func(*Call, json.RawMessage) (Result, error)
	apply      map[string]func(Entry) error
	reset      []func()
	transforms map[string]func(doc, args json.RawMessage) (any, error)
	conn       *rpc.Conn
	last       *Entry // the last entry applied, to refuse out-of-order ones
	schedules  []protocol.Schedule
	jobs       map[string]func(*Call) error
}

// ErrLate is the error code for an entry that sorts before one already
// applied. The core answers it by replaying the plugin's log in order.
const ErrLate = -32010

func before(a, b Entry) bool {
	if a.HLC != b.HLC {
		return a.HLC < b.HLC
	}
	if a.Device != b.Device {
		return a.Device < b.Device
	}
	return a.Seq < b.Seq
}

// Call is one operation invocation.
type Call struct {
	Context context.Context
	Caller  Caller
	ctx     *Ctx
}

// Kind marks whether an operation changes state.
type Kind bool

const (
	Read  Kind = true
	Write Kind = false
)

func Text(s string) Result { return Result{Text: s} }

// Entry declares an entry type the plugin writes.
func (c *Ctx) Entry(typ string, mode Mode) {
	c.entries = append(c.entries, protocol.EntryType{Type: typ, Mode: mode})
}

// OnReset runs before the log is replayed, so state can be cleared.
func (c *Ctx) OnReset(fn func()) { c.reset = append(c.reset, fn) }

// Handle registers how an entry type changes plugin state.
func Handle[T any](c *Ctx, typ string, fn func(Entry, T) error) {
	c.apply[typ] = func(e Entry) error {
		var v T
		if len(e.Payload) > 0 {
			if err := json.Unmarshal(e.Payload, &v); err != nil {
				return fmt.Errorf("%s: decode %s: %w", c.id, typ, err)
			}
		}
		return fn(e, v)
	}
}

// Op registers an operation. Its input schema is derived from In, which
// becomes an API endpoint, a CLI command and an MCP tool.
func Op[In any](c *Ctx, name, description string, kind Kind, fn func(*Call, In) (Result, error)) {
	schema, err := jsonschema.For[In](nil)
	if err != nil {
		panic(fmt.Sprintf("%s.%s: input schema: %v", c.id, name, err))
	}
	raw, _ := json.Marshal(schema)
	c.ops = append(c.ops, protocol.Operation{Name: name, Description: description, InputSchema: raw, ReadOnly: bool(kind)})
	c.opFns[name] = func(call *Call, input json.RawMessage) (Result, error) {
		var in In
		if len(input) > 0 && string(input) != "null" {
			if err := json.Unmarshal(input, &in); err != nil {
				return Result{}, fmt.Errorf("bad input: %w", err)
			}
		}
		return fn(call, in)
	}
}

// Wake registers a transform over the wake document.
func (c *Ctx) Wake(fn func(doc *protocol.WakeDoc, args protocol.WakeArgs) error) {
	c.transforms[protocol.TransformWake] = func(d, a json.RawMessage) (any, error) {
		var doc protocol.WakeDoc
		var args protocol.WakeArgs
		if err := json.Unmarshal(d, &doc); err != nil {
			return nil, err
		}
		if len(a) > 0 {
			_ = json.Unmarshal(a, &args)
		}
		err := fn(&doc, args)
		return doc, err
	}
}

// Every registers a job run once per interval on exactly one of the user's
// devices. A job's Call has no caller; it can Append like an operation.
func (c *Ctx) Every(name string, every time.Duration, fn func(*Call) error) {
	c.schedules = append(c.schedules, protocol.Schedule{Name: name, EveryMs: every.Milliseconds()})
	c.jobs[name] = fn
}

// Instructions registers a transform over the MCP server instructions.
func (c *Ctx) Instructions(fn func(text string) string) {
	c.transforms[protocol.TransformInstructions] = func(d, _ json.RawMessage) (any, error) {
		var doc protocol.InstructionsDoc
		if err := json.Unmarshal(d, &doc); err != nil {
			return nil, err
		}
		doc.Text = fn(doc.Text)
		return doc, nil
	}
}

// Logf writes to the core's log.
func (c *Ctx) Logf(format string, args ...any) {
	if c.conn == nil {
		return
	}
	_ = c.conn.Call(context.Background(), protocol.MethodLog, protocol.LogParams{Level: "info", Message: fmt.Sprintf(format, args...)}, nil)
}

// Append writes an entry to this device's log and applies it to the plugin's
// own state before returning.
func (call *Call) Append(typ string, payload any) (Entry, error) {
	raw, err := json.Marshal(payload)
	if err != nil {
		return Entry{}, err
	}
	var e Entry
	if err := call.ctx.conn.Call(call.Context, protocol.MethodAppend, protocol.AppendParams{Type: typ, Payload: raw, Actor: call.Caller.Actor}, &e); err != nil {
		return Entry{}, err
	}
	return e, call.ctx.applyOne(e)
}

func (c *Ctx) applyOne(e Entry) error {
	if c.last != nil && !before(*c.last, e) {
		return &rpc.Error{Code: ErrLate, Message: "entry " + e.ID + " arrived out of order"}
	}
	c.last = &e
	fn := c.apply[e.Type]
	if fn == nil {
		return nil // unknown types (e.g. from a newer version) are skipped
	}
	return fn(e)
}

// Run starts the plugin. It only returns when the core closes the connection.
func Run(p Plugin) {
	c := &Ctx{
		id:         p.ID,
		opFns:      map[string]func(*Call, json.RawMessage) (Result, error){},
		apply:      map[string]func(Entry) error{},
		transforms: map[string]func(doc, args json.RawMessage) (any, error){},
		jobs:       map[string]func(*Call) error{},
	}
	if err := p.Setup(c); err != nil {
		fmt.Fprintf(os.Stderr, "%s: setup: %v\n", p.ID, err)
		os.Exit(1)
	}
	c.conn = rpc.New(os.Stdin, os.Stdout, c.handle(p), true)
	if err := c.conn.Run(context.Background()); err != nil {
		fmt.Fprintf(os.Stderr, "%s: %v\n", p.ID, err)
		os.Exit(1)
	}
}

func (c *Ctx) handle(p Plugin) rpc.Handler {
	return func(ctx context.Context, method string, params json.RawMessage) (any, error) {
		switch method {
		case protocol.MethodDescribe:
			names := make([]string, 0, len(c.transforms))
			for n := range c.transforms {
				names = append(names, n)
			}
			return protocol.Manifest{Protocol: protocol.Version, ID: p.ID, Version: p.Version, Entries: c.entries, Operations: c.ops, Transforms: names, Schedules: c.schedules}, nil
		case protocol.MethodReset:
			c.last = nil
			for _, fn := range c.reset {
				fn()
			}
			return struct{}{}, nil
		case protocol.MethodApply:
			var in protocol.ApplyParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			for _, e := range in.Entries {
				if err := c.applyOne(e); err != nil {
					return nil, err
				}
			}
			return struct{}{}, nil
		case protocol.MethodCall:
			var in protocol.CallParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			fn := c.opFns[in.Operation]
			if fn == nil {
				return nil, &rpc.Error{Code: -32601, Message: "unknown operation " + in.Operation}
			}
			return fn(&Call{Context: ctx, Caller: in.Caller, ctx: c}, in.Input)
		case protocol.MethodRun:
			var in protocol.RunParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			fn := c.jobs[in.Schedule]
			if fn == nil {
				return nil, &rpc.Error{Code: -32601, Message: "unknown schedule " + in.Schedule}
			}
			return struct{}{}, fn(&Call{Context: ctx, Caller: Caller{Actor: "schedule:" + in.Schedule}, ctx: c})
		case protocol.MethodTransform:
			var in protocol.TransformParams
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			fn := c.transforms[in.Name]
			if fn == nil {
				return json.RawMessage(in.Doc), nil
			}
			return fn(in.Doc, in.Args)
		}
		return nil, &rpc.Error{Code: -32601, Message: "unknown method " + method}
	}
}
