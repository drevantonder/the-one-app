// Package protocol defines the messages exchanged between the core and plugin
// processes. Plugins speak JSON-RPC 2.0 over stdin/stdout, one message per line.
package protocol

import (
	"encoding/json"
	"time"
)

// Version is the plugin protocol version. Bump it on breaking changes.
const Version = 4

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
)

// Transform names a plugin can register.
const (
	TransformInstructions = "instructions"
	TransformMenubar      = "menubar"
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
}

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
// showing the result of Tool (an MCP tool name) called with no arguments.
// Title is shown above it; empty means the view draws its own header.
type MenubarItem struct {
	ID       string `json:"id"`
	Title    string `json:"title,omitempty"`
	Tool     string `json:"tool"`
	Resource string `json:"resource"`
}
