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
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/google/jsonschema-go/jsonschema"

	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/rpc"
	"github.com/drevantonder/the-one-app/sdk/widget"
)

type (
	Mode     = protocol.Mode
	Entry    = protocol.Entry
	Result   = protocol.Result
	Image    = protocol.Image
	Caller   = protocol.Caller
	FileInfo = protocol.FileInfo
	// MenubarItem is a view in the menu bar; see Ctx.Menubar.
	MenubarItem = protocol.MenubarItem
	// PageItem is an entry in a page's list; see Ctx.Page.
	PageItem = protocol.PageItem
)

const (
	Available  = protocol.Available
	Consistent = protocol.Consistent
)

type Plugin struct {
	ID        string
	Name      string // shown to people, e.g. "iMessage"; the ID if empty
	Version   string
	Platforms []string // GOOS values the plugin runs on; empty means all
	Secrets   []string // the user's secrets the plugin reads with Call.Secret, like OPENCODE_API_KEY
	Setup     func(*Ctx) error
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
	hooks      map[string]func(*Call, protocol.HookRequest) (protocol.HookResponse, error)
	ready      []func(*Call)
	pages      []protocol.Page
	resources  []protocol.Resource
}

// OnReady runs once the plugin's log has been replayed, so its state is
// current: a good time to connect to outside services.
func (c *Ctx) OnReady(fn func(*Call)) { c.ready = append(c.ready, fn) }

// ID is the plugin's id.
func (c *Ctx) ID() string { return c.id }

// Dir is a folder of the plugin's own on this device, for what can't live
// in its log, such as another service's session. Other devices don't see it.
func (c *Ctx) Dir() (string, error) {
	dir := os.Getenv("ONE_PLUGIN_DIR")
	if dir == "" {
		return "", errors.New("no plugin folder: the plugin isn't run by a node")
	}
	return dir, os.MkdirAll(dir, 0o700)
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
	c.OpRaw(name, description, raw, kind, func(call *Call, input json.RawMessage) (Result, error) {
		var in In
		if len(input) > 0 && string(input) != "null" {
			if err := json.Unmarshal(input, &in); err != nil {
				return Result{}, fmt.Errorf("bad input: %w", err)
			}
		}
		return fn(call, in)
	})
}

// OpRaw registers an operation with a JSON Schema given as is, for tools
// imported from elsewhere. Registering a name again replaces it.
func (c *Ctx) OpRaw(name, description string, schema json.RawMessage, kind Kind, fn func(*Call, json.RawMessage) (Result, error)) {
	c.RemoveOp(name)
	c.ops = append(c.ops, protocol.Operation{Name: name, Description: description, InputSchema: schema, ReadOnly: bool(kind)})
	c.opFns[name] = fn
}

// RemoveOp drops an operation.
func (c *Ctx) RemoveOp(name string) {
	for i, o := range c.ops {
		if o.Name == name {
			c.ops = append(c.ops[:i], c.ops[i+1:]...)
			break
		}
	}
	delete(c.opFns, name)
}

// Changed tells the core the plugin's operations changed, after Setup (e.g.
// tools imported once an account is connected).
func (c *Ctx) Changed() {
	if c.conn != nil {
		_ = c.conn.Notify(protocol.MethodChanged, struct{}{})
	}
}

// Hook handles HTTP requests to /hooks/<plugin>/<name> on this device's
// node, e.g. an OAuth redirect or a webhook.
func (c *Ctx) Hook(name string, fn func(*Call, protocol.HookRequest) (protocol.HookResponse, error)) {
	c.hooks[name] = fn
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

// View adds an MCP Apps view at ui://<plugin id>/<name>: an HTML page that
// shows an operation's result in an MCP host, such as Claude or the menu
// bar. It has no network access unless meta's ui.csp allows it, so it gets
// its data through tool calls. It returns the view's URI.
func (c *Ctx) View(name, html string, meta json.RawMessage) string {
	uri := "ui://" + c.id + "/" + name
	c.resources = append(c.resources, protocol.Resource{URI: uri, Name: c.id + " " + name, MIMEType: protocol.MIMEApp, Text: html, Meta: meta})
	return uri
}

// ShowIn sets the view that shows an operation's result. With appOnly, only
// views call it: agents don't see it as a tool.
func (c *Ctx) ShowIn(op, viewURI string, appOnly bool) {
	for i, o := range c.ops {
		if o.Name == op {
			ui := &protocol.OperationUI{ResourceURI: viewURI}
			if appOnly {
				ui.Visibility = []string{"app"}
			}
			c.ops[i].UI = ui
			return
		}
	}
	panic(fmt.Sprintf("%s: ShowIn: no operation %q", c.id, op))
}

// Widget registers an operation that returns a widget (package widget),
// drawn by the apps, the menu bar, home screens and MCP Apps hosts. render
// gets the room the widget is drawn in, if known, and returns the widget and
// a plain-text summary for agents. Renderers fetch it again after refresh
// (15 minutes if zero), or sooner.
func (c *Ctx) Widget(name, title, description string, refresh time.Duration, render func(*Call, widget.Space) (widget.Node, string, error)) {
	if refresh == 0 {
		refresh = 15 * time.Minute
	}
	Op(c, name, description, Read, func(call *Call, space widget.Space) (Result, error) {
		root, text, err := render(call, space)
		if err != nil {
			return Result{}, err
		}
		now := time.Now()
		return Result{Text: text, Data: widget.Description{
			V: widget.Version, App: c.id, Widget: name, Title: title,
			At: now.UnixMilli(), Next: now.Add(refresh).UnixMilli(), W: space.W, H: space.H, Root: root,
		}}, nil
	})
	op := &c.ops[len(c.ops)-1]
	op.Widget = true
	op.UI = &protocol.OperationUI{ResourceURI: protocol.WidgetView}
}

// Menubar registers a transform over the menu bar's items: add this plugin's
// own, or reorder or hide any.
func (c *Ctx) Menubar(fn func(items []MenubarItem) []MenubarItem) {
	c.transforms[protocol.TransformMenubar] = func(d, _ json.RawMessage) (any, error) {
		var doc protocol.MenubarDoc
		if err := json.Unmarshal(d, &doc); err != nil {
			return nil, err
		}
		doc.Items = fn(doc.Items)
		return doc, nil
	}
}

type pageIn struct {
	Item string `json:"item,omitempty" jsonschema:"the ID of the page's item to show, as its list gives it"`
	TZ   string `json:"tz,omitempty" jsonschema:"the IANA time zone to show times in, like Australia/Brisbane; omit for the node's own"`
}

// Page adds a page to the app's sidebar, titled title with an SF Symbol
// icon. It lists items, and draws the one chosen with render, for a person
// in loc; with no items, render draws the whole page, for item "". It adds
// operations name, which draws an item as a widget, and name_items.
func (c *Ctx) Page(name, title, icon, description string, items func(*Call) ([]PageItem, error), render func(call *Call, item string, loc *time.Location) (widget.Node, string, error)) {
	c.pageOp(name, title, description, render)
	page := protocol.Page{ID: c.id + "." + name, Title: title, Icon: icon, Tool: c.id + "_" + name}
	if items != nil {
		page.Items = c.id + "_" + name + "_items"
		c.pageItems(name, title, items)
	}
	c.pages = append(c.pages, page)
	c.transforms[protocol.TransformPages] = func(d, _ json.RawMessage) (any, error) {
		var doc protocol.PagesDoc
		if err := json.Unmarshal(d, &doc); err != nil {
			return nil, err
		}
		doc.Pages = append(doc.Pages, c.pages...)
		return doc, nil
	}
}

// PageSettings gives page name settings, drawn by render behind a gear. It
// adds the operation name_settings.
func (c *Ctx) PageSettings(name string, render func(call *Call, loc *time.Location) (widget.Node, string, error)) {
	for i, p := range c.pages {
		if p.ID == c.id+"."+name {
			c.pageOp(name+"_settings", p.Title+" settings", "Draw the "+p.Title+" page's settings.", func(call *Call, _ string, loc *time.Location) (widget.Node, string, error) {
				return render(call, loc)
			})
			c.pages[i].Settings = c.id + "_" + name + "_settings"
			return
		}
	}
	panic(fmt.Sprintf("%s: PageSettings: no page %q", c.id, name))
}

// PageAction adds a button beside page name's title, opening popout (a URL
// or a path on the node) and running this plugin's operation close, if
// any, when its window closes.
func (c *Ctx) PageAction(name, title, icon, popout, close string) {
	for i, p := range c.pages {
		if p.ID == c.id+"."+name {
			a := protocol.PageAction{Title: title, Icon: icon, Popout: popout}
			if close != "" {
				a.Close = c.id + "_" + close
			}
			c.pages[i].Actions = append(c.pages[i].Actions, a)
			return
		}
	}
	panic(fmt.Sprintf("%s: PageAction: no page %q", c.id, name))
}

func (c *Ctx) pageItems(name, title string, items func(*Call) ([]PageItem, error)) {
	Op(c, name+"_items", "List the items of the "+title+" page.", Read, func(call *Call, _ struct{}) (Result, error) {
		list, err := items(call)
		if err != nil {
			return Result{}, err
		}
		if list == nil {
			list = []PageItem{}
		}
		lines := make([]string, len(list))
		for i, it := range list {
			lines[i] = strings.Repeat("  ", it.Depth) + it.Title
		}
		return Result{Text: strings.Join(lines, "\n"), Data: map[string]any{"items": list}}, nil
	})
}

func (c *Ctx) pageOp(name, title, description string, render func(call *Call, item string, loc *time.Location) (widget.Node, string, error)) {
	Op(c, name, description, Read, func(call *Call, in pageIn) (Result, error) {
		loc, err := time.LoadLocation(in.TZ)
		if err != nil {
			return Result{}, fmt.Errorf("tz: %w", err)
		}
		if in.TZ == "" {
			loc = time.Local
		}
		root, text, err := render(call, in.Item, loc)
		if err != nil {
			return Result{}, err
		}
		now := time.Now()
		return Result{Text: text, Data: widget.Description{
			V: widget.Version, App: c.id, Widget: name, Title: title, At: now.UnixMilli(), Next: now.Add(time.Minute).UnixMilli(), Root: root,
		}}, nil
	})
	c.ops[len(c.ops)-1].UI = &protocol.OperationUI{ResourceURI: protocol.WidgetView}
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

// Invoke runs another plugin's operation, on whichever device has it, or
// the one named by a device field in input.
func (call *Call) Invoke(plugin, op string, input any) (Result, error) {
	raw, err := json.Marshal(input)
	if err != nil {
		return Result{}, err
	}
	var res Result
	err = call.ctx.conn.Call(call.Context, protocol.MethodInvoke, protocol.InvokeParams{Plugin: plugin, Operation: op, Input: raw, Agent: call.Caller.Agent}, &res)
	return res, err
}

// ErrNoSecret means the user hasn't set a secret the plugin reads.
var ErrNoSecret = errors.New("secret not set")

// Secret is the value of one of the user's secrets, named in Plugin.Secrets.
// The user sets them with secrets_set; read it on each use, since it can
// change.
func (call *Call) Secret(name string) (string, error) {
	var v string
	err := call.ctx.conn.Call(call.Context, protocol.MethodSecret, protocol.SecretParams{Name: name}, &v)
	if e, ok := errors.AsType[*rpc.Error](err); ok && e.Code == protocol.ErrCodeNoSecret {
		return "", fmt.Errorf("%w: %s; set it with secrets_set", ErrNoSecret, name)
	}
	return v, err
}

// PutFile stores the file at path in the bucket as name, encrypted. Each
// plugin has its own names.
func (call *Call) PutFile(name, path string) error {
	return call.ctx.conn.Call(call.Context, protocol.MethodFiles, protocol.FilesParams{Op: "put", Name: name, Path: path}, nil)
}

// GetFile downloads the stored file name to path.
func (call *Call) GetFile(name, path string) error {
	return call.ctx.conn.Call(call.Context, protocol.MethodFiles, protocol.FilesParams{Op: "get", Name: name, Path: path}, nil)
}

// Files lists the stored files whose names start with prefix.
func (call *Call) Files(prefix string) ([]FileInfo, error) {
	var out []FileInfo
	err := call.ctx.conn.Call(call.Context, protocol.MethodFiles, protocol.FilesParams{Op: "list", Name: prefix}, &out)
	return out, err
}

// DeleteFile removes the stored file name.
func (call *Call) DeleteFile(name string) error {
	return call.ctx.conn.Call(call.Context, protocol.MethodFiles, protocol.FilesParams{Op: "delete", Name: name}, nil)
}

// Logf writes to the node's log, as Ctx.Logf.
func (call *Call) Logf(format string, args ...any) { call.ctx.Logf(format, args...) }

// Detached is a copy of the call that outlives it, for work that carries
// on in the background.
func (call *Call) Detached() *Call {
	return &Call{Context: context.WithoutCancel(call.Context), Caller: call.Caller, ctx: call.ctx}
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
		hooks:      map[string]func(*Call, protocol.HookRequest) (protocol.HookResponse, error){},
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
			hooks := make([]string, 0, len(c.hooks))
			for h := range c.hooks {
				hooks = append(hooks, h)
			}
			return protocol.Manifest{Protocol: protocol.Version, ID: p.ID, Name: p.Name, Version: p.Version, Entries: c.entries, Operations: c.ops, Transforms: names, Schedules: c.schedules, Hooks: hooks, Platforms: p.Platforms, Resources: c.resources, Secrets: p.Secrets}, nil
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
		case protocol.MethodReady:
			for _, fn := range c.ready {
				fn(&Call{Context: ctx, Caller: Caller{Actor: "ready"}, ctx: c})
			}
			return nil, nil
		case protocol.MethodHook:
			var in protocol.HookRequest
			if err := json.Unmarshal(params, &in); err != nil {
				return nil, err
			}
			fn := c.hooks[in.Name]
			if fn == nil {
				return protocol.HookResponse{Status: 404, Body: "no such hook"}, nil
			}
			return fn(&Call{Context: ctx, Caller: Caller{Actor: "hook:" + in.Name}, ctx: c}, in)
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
