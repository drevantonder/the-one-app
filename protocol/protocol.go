// Package protocol defines the messages exchanged between the core and plugin
// processes. Plugins speak JSON-RPC 2.0 over stdin/stdout, one message per line.
package protocol

import (
	"encoding/json"
	"time"
)

// Version is the plugin protocol version. Bump it on breaking changes.
const Version = 10

// Core → plugin methods.
const (
	MethodDescribe  = "describe"
	MethodReset     = "reset"
	MethodApply     = "apply"
	MethodCall      = "call"
	MethodTransform = "transform"
	MethodRun       = "run"
	MethodHook      = "hook"
	// MethodReady is a notification sent once the log has been replayed.
	MethodReady = "ready"
)

// Plugin → core methods.
const (
	MethodAppend = "append"
	MethodLog    = "log"
	// MethodChanged is a notification: the plugin's operations changed, so
	// the core should describe it again.
	MethodChanged = "changed"
	// MethodInvoke runs another plugin's operation, routed like any call.
	MethodInvoke = "invoke"
	// MethodFiles stores the plugin's files in the bucket, encrypted.
	MethodFiles = "files"
	// MethodSecret gives the value of one of the user's secrets that the
	// plugin's manifest names.
	MethodSecret = "secret"
)

// Transform names a plugin can register.
const (
	TransformInstructions = "instructions"
	TransformMenubar      = "menubar"
	TransformPages        = "pages"
)

// Mode is how an entry type behaves when devices disagree.
type Mode string

const (
	// Available entries can be written on any device, even offline, and merge.
	Available Mode = "available"
	// Consistent entries have a single writer at a time.
	Consistent Mode = "consistent"
)

type Manifest struct {
	Protocol   int         `json:"protocol"`
	ID         string      `json:"id"`
	Name       string      `json:"name,omitempty"` // shown to people; the id if empty
	Version    string      `json:"version"`
	Entries    []EntryType `json:"entries"`
	Operations []Operation `json:"operations"`
	Transforms []string    `json:"transforms"`
	Schedules  []Schedule  `json:"schedules,omitempty"`
	Hooks      []string    `json:"hooks,omitempty"`
	// Platforms the plugin runs on (GOOS values); empty means all.
	Platforms []string `json:"platforms,omitempty"`
	// Resources are MCP resources, such as MCP Apps views. Each URI starts
	// with ui://<id>/.
	Resources []Resource `json:"resources,omitempty"`
	// Secrets names the user's secrets the plugin reads, such as
	// OPENCODE_API_KEY.
	Secrets []string `json:"secrets,omitempty"`
}

// MIMEApp is the MIME type of an MCP Apps view.
const MIMEApp = "text/html;profile=mcp-app"

// Resource is an MCP resource served as is, e.g. an MCP Apps view (HTML,
// MIMEApp). Meta is its _meta, such as {"ui":{"csp":{...}}}.
type Resource struct {
	URI         string          `json:"uri"`
	Name        string          `json:"name"`
	Description string          `json:"description,omitempty"`
	MIMEType    string          `json:"mimeType"`
	Text        string          `json:"text"`
	Meta        json.RawMessage `json:"meta,omitempty"`
}

// HookRequest is an HTTP request to /hooks/<plugin>/<name> on a node.
type HookRequest struct {
	Name   string            `json:"name"`
	Method string            `json:"method"`
	Query  map[string]string `json:"query,omitempty"`
	Body   string            `json:"body,omitempty"`
}

type HookResponse struct {
	Status      int    `json:"status,omitempty"`
	ContentType string `json:"contentType,omitempty"`
	Body        string `json:"body,omitempty"`
}

// Schedule is a job the plugin wants run every EveryMs, on exactly one of
// the user's devices at a time.
type Schedule struct {
	Name    string `json:"name"`
	EveryMs int64  `json:"everyMs"`
}

type SecretParams struct {
	Name string `json:"name"`
}

// ErrCodeNoSecret is the error code for a secret that isn't set.
const ErrCodeNoSecret = -32012

type RunParams struct {
	Schedule string `json:"schedule"`
}

// ErrNotHolder is returned when a consistent entry is appended on a device
// that does not hold the plugin's lease. The core forwards the operation.
const ErrNotHolder = -32011

type EntryType struct {
	Type string `json:"type"`
	Mode Mode   `json:"mode"`
}

type Operation struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"`
	ReadOnly    bool            `json:"readOnly,omitempty"`
	UI          *OperationUI    `json:"ui,omitempty"`
	// Widget means the operation's result data is a widget (package
	// sdk/widget), drawn natively on the user's devices and by WidgetView in
	// MCP Apps hosts.
	Widget bool `json:"widget,omitempty"`
}

// WidgetView is the core's MCP Apps view that draws any widget.
const WidgetView = "ui://one/widget"

// OperationUI is an operation's MCP Apps metadata (the tool's _meta.ui):
// the view that shows its result, and who may call it ("model", "app";
// empty means both).
type OperationUI struct {
	ResourceURI string   `json:"resourceUri,omitempty"`
	Visibility  []string `json:"visibility,omitempty"`
}

// Entry is one record in a device's append-only log.
type Entry struct {
	ID      string          `json:"id"`
	Device  string          `json:"device"`
	Seq     int64           `json:"seq"`
	HLC     uint64          `json:"hlc"`
	Plugin  string          `json:"plugin"`
	Type    string          `json:"type"`
	Payload json.RawMessage `json:"payload"`
	Actor   string          `json:"actor,omitempty"`
	Time    string          `json:"time"`
}

type ApplyParams struct {
	Entries []Entry `json:"entries"`
}

type Caller struct {
	Actor  string `json:"actor"`
	Device string `json:"device"`
	Agent  string `json:"agent,omitempty"` // the MCP client calling, e.g. claude-code
}

type CallParams struct {
	Operation string          `json:"operation"`
	Input     json.RawMessage `json:"input"`
	Caller    Caller          `json:"caller"`
}

type InvokeParams struct {
	Plugin    string          `json:"plugin"`
	Operation string          `json:"operation"`
	Input     json.RawMessage `json:"input"`
	Agent     string          `json:"agent,omitempty"` // the agent the invoking call is for
}

// FilesParams asks for one of put (upload Path as Name), get (download Name
// to Path), list (names starting with Name) or delete. Paths are on the
// device both the core and the plugin run on.
type FilesParams struct {
	Op   string `json:"op"`
	Name string `json:"name"`
	Path string `json:"path,omitempty"`
}

// FileInfo describes a stored file.
type FileInfo struct {
	Name     string    `json:"name"`
	Size     int64     `json:"size"`
	Modified time.Time `json:"modified"`
}

type Result struct {
	Text   string  `json:"text,omitempty"`
	Data   any     `json:"data,omitempty"`
	Images []Image `json:"images,omitempty"`
}

// An Image is part of a result, such as a screenshot. Data is base64 in JSON.
type Image struct {
	MIMEType string `json:"mimeType"`
	Data     []byte `json:"data"`
}

type TransformParams struct {
	Name string          `json:"name"`
	Doc  json.RawMessage `json:"doc"`
	Args json.RawMessage `json:"args,omitempty"`
}

type AppendParams struct {
	Type    string          `json:"type"`
	Payload json.RawMessage `json:"payload"`
	Actor   string          `json:"actor,omitempty"`
}

type LogParams struct {
	Level   string `json:"level"`
	Message string `json:"message"`
}

// InstructionsDoc is the MCP server instructions, built by transforms.
type InstructionsDoc struct {
	Text string `json:"text"`
}

// MenubarDoc is the menu bar, built by transforms from the core's items.
// Items show in order.
type MenubarDoc struct {
	Items []MenubarItem `json:"items"`
}

// MenubarItem is one view in the menu bar: Resource (an MCP Apps view)
// showing the result of Tool (an MCP tool name) called with no arguments, or,
// with no Resource, the widget Tool returns, drawn natively.
// Title is shown above it; empty means the view draws its own header. With
// Submenu, the menu shows a row with Title, and the view opens beside the
// menu from it, like a submenu. With Bar, the item is not in the menu but
// beside it in the system menu bar, titled with the first line of Tool's
// text result, and the view opens below it.
type MenubarItem struct {
	ID       string `json:"id"`
	Title    string `json:"title,omitempty"`
	Tool     string `json:"tool"`
	Resource string `json:"resource,omitempty"`
	Submenu  bool   `json:"submenu,omitempty"`
	Bar      bool   `json:"bar,omitempty"`
}

// PagesDoc is the pages plugins add to the app's sidebar, built by
// transforms. Pages show in order.
type PagesDoc struct {
	Pages []Page `json:"pages"`
}

// Page is a page of the app: Items (an MCP tool name) lists what it holds
// as {"items": [PageItem]}, and choosing one draws the widget Tool returns
// for {"item": its ID, "tz": the device's IANA time zone}. Without Items,
// Tool's widget is the whole page. Settings, a tool too, draws the page's
// settings, behind a gear. Icon is an SF Symbol name.
type Page struct {
	ID       string       `json:"id"`
	Title    string       `json:"title"`
	Icon     string       `json:"icon,omitempty"`
	Tool     string       `json:"tool"`
	Items    string       `json:"items,omitempty"`
	Settings string       `json:"settings,omitempty"`
	Actions  []PageAction `json:"actions,omitempty"`
}

// PageAction is a button beside the page's title. It opens Popout, a URL
// or a path on the node, in a window of its own, and runs Close, a tool,
// when that window closes. Icon is an SF Symbol name.
type PageAction struct {
	Title  string `json:"title"`
	Icon   string `json:"icon,omitempty"`
	Popout string `json:"popout"`
	Close  string `json:"close,omitempty"`
}

// PageItem is one entry in a page's list, Depth levels in under the one
// before it.
type PageItem struct {
	ID    string `json:"id"`
	Title string `json:"title"`
	Depth int    `json:"depth,omitempty"`
}
