// Package protocol defines the messages exchanged between the core and plugin
// processes. Plugins speak JSON-RPC 2.0 over stdin/stdout, one message per line.
package protocol

import "encoding/json"

// Version is the plugin protocol version. Bump it on breaking changes.
const Version = 1

// Core → plugin methods.
const (
	MethodDescribe  = "describe"
	MethodReset     = "reset"
	MethodApply     = "apply"
	MethodCall      = "call"
	MethodTransform = "transform"
	MethodRun       = "run"
)

// Plugin → core methods.
const (
	MethodAppend = "append"
	MethodLog    = "log"
)

// Transform names a plugin can register.
const (
	TransformWake         = "wake"
	TransformInstructions = "instructions"
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
	Version    string      `json:"version"`
	Entries    []EntryType `json:"entries"`
	Operations []Operation `json:"operations"`
	Transforms []string    `json:"transforms"`
	Schedules  []Schedule  `json:"schedules,omitempty"`
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

type Result struct {
	Text string `json:"text,omitempty"`
	Data any    `json:"data,omitempty"`
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

// WakeDoc is what the wake transforms build, starting from empty.
type WakeDoc struct {
	Sections []Section `json:"sections"`
}

type Section struct {
	Plugin string `json:"plugin"`
	Title  string `json:"title"`
	Body   string `json:"body"`
}

// WakeArgs page through a long wake.
type WakeArgs struct {
	Part int   `json:"part,omitempty"`
	At   int64 `json:"at,omitempty"`
}

// InstructionsDoc is the MCP server instructions, built by transforms.
type InstructionsDoc struct {
	Text string `json:"text"`
}
