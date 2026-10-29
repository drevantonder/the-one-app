// Package memory is the built-in memory plugin, a rewrite of OptMem.
package memory

import (
	"fmt"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

type (
	noteEntry struct {
		Text string `json:"text"`
		Date string `json:"date"`
	}
	summaryEntry struct {
		Lo     int    `json:"lo"`
		Hi     int    `json:"hi"`
		Digest string `json:"digest"`
		Text   string `json:"text"`
	}
	forgetEntry struct {
		Lo int `json:"lo"`
		Hi int `json:"hi"`
	}

	noteIn struct {
		Text string `json:"text" jsonschema:"one line, at most 280 bytes"`
	}
	napIn struct {
		Block string `json:"block,omitempty" jsonschema:"the block id from the compression request, like 0-1; omit both fields to see the next request"`
		Text  string `json:"text,omitempty" jsonschema:"the compressed line, at most 280 bytes"`
	}
	recallIn struct {
		Pattern string `json:"pattern" jsonschema:"case-insensitive regular expression"`
	}
	blockIn struct {
		Block string `json:"block" jsonschema:"a block id as wake prints it, like 16-31"`
	}
	wakeIn struct {
		Part int `json:"part,omitempty" jsonschema:"which part of a long memory to read; omit for the first"`
		At   int `json:"at,omitempty" jsonschema:"the snapshot to read the part from, as printed by the previous part"`
	}
)

var Plugin = sdk.Plugin{ID: "memory", Name: "Memory", Version: "0.1.0", Setup: setup}

const protocolText = `## Memory

Before anything else, in every session, call ` + "`memory_wake`" + ` and do what it says, to the end of its output.

Your memory lives in TheOneApp, shared across this user's devices. It outlives every session, compaction, model and vendor change. Without it you do not know who you are, or what was decided and tried.

While working, call ` + "`memory_note`" + ` (one line, at most 280 bytes) whenever you learn something new or something worth keeping happens: a task worth real effort, a fact or insight the user teaches you, anything you learn about their life (even indirectly), any event of lasting effect. Do not note redundant memories.

If a tool result asks for a compression, do it with ` + "`memory_nap`" + ` before your next action.

To find an old memory, search with ` + "`memory_recall`" + `, or open a #a-b summary with ` + "`memory_zoom`" + `.

If you are a subagent, never write memories: you cannot judge what is already known. When you spawn a subagent, tell it: "You are a subagent. Don't write memories."`

func setup(c *sdk.Ctx) error {
	m := New()

	c.Entry("note", sdk.Available)
	c.Entry("summary", sdk.Available)
	c.Entry("forget", sdk.Available)
	c.OnReset(func() { m = New() })
	sdk.Handle(c, "note", func(e sdk.Entry, in noteEntry) error {
		m.Add(e.ID, in.Date, in.Text)
		return nil
	})
	sdk.Handle(c, "summary", func(_ sdk.Entry, in summaryEntry) error {
		m.PutSummary(block{in.Lo, in.Hi}, in.Digest, in.Text)
		return nil
	})
	sdk.Handle(c, "forget", func(_ sdk.Entry, in forgetEntry) error {
		m.Forget(block{in.Lo, in.Hi})
		return nil
	})

	sdk.Op(c, "note", "Record one memory: one line, at most 280 bytes.", sdk.Write, func(call *sdk.Call, in noteIn) (sdk.Result, error) {
		text, err := CheckText(in.Text)
		if err != nil {
			return sdk.Result{}, err
		}
		if _, err := call.Append("note", noteEntry{Text: text, Date: time.Now().Format("2006-01-02")}); err != nil {
			return sdk.Result{}, err
		}
		out := fmt.Sprintf("Saved as #%d.", m.T()-1)
		if nap := m.NextNap(); nap != "" {
			out += "\n\n" + nap
		}
		return sdk.Text(out), nil
	})

	sdk.Op(c, "nap", "Answer a compression request; with no arguments, show the next one.", sdk.Write, func(call *sdk.Call, in napIn) (sdk.Result, error) {
		var out []string
		if in.Block != "" || in.Text != "" {
			b, err := ParseBlock(in.Block)
			if err != nil {
				return sdk.Result{}, err
			}
			todo := m.Pending(1)
			switch {
			case len(todo) == 0:
				return sdk.Text("Nothing left to compress."), nil
			case b != todo[0]:
				if _, ok := m.Summary(b); ok {
					out = append(out, fmt.Sprintf("%s is already settled.", b))
					break
				}
				return sdk.Result{}, fmt.Errorf("wrong block %s: blocks are compressed in order, and the next is %s", b, todo[0])
			default:
				text, err := CheckText(in.Text)
				if err != nil {
					return sdk.Result{}, err
				}
				if _, err := call.Append("summary", summaryEntry{Lo: b.Lo, Hi: b.Hi, Digest: m.Digest(b), Text: text}); err != nil {
					return sdk.Result{}, err
				}
				out = append(out, fmt.Sprintf("%s saved.", b))
			}
		}
		if nap := m.NextNap(); nap != "" {
			out = append(out, nap)
		} else {
			out = append(out, "Nothing left to compress.")
		}
		return sdk.Text(strings.Join(out, "\n\n")), nil
	})

	sdk.Op(c, "recall", "Search every memory, word for word, with a case-insensitive regular expression.", sdk.Read, func(_ *sdk.Call, in recallIn) (sdk.Result, error) {
		s, err := m.Recall(in.Pattern)
		return sdk.Text(s), err
	})

	sdk.Op(c, "zoom", "Open a #a-b summary into its two halves, down to the raw memories.", sdk.Read, func(_ *sdk.Call, in blockIn) (sdk.Result, error) {
		b, err := ParseBlock(in.Block)
		if err != nil {
			return sdk.Result{}, err
		}
		s, err := m.Zoom(b)
		return sdk.Text(s), err
	})

	sdk.Op(c, "forget", "Drop a wrong summary and every summary built on it; they are asked for again. Memories themselves are never lost.", sdk.Write, func(call *sdk.Call, in blockIn) (sdk.Result, error) {
		b, err := ParseBlock(in.Block)
		if err != nil {
			return sdk.Result{}, err
		}
		if _, ok := m.Summary(b); !ok {
			return sdk.Result{}, fmt.Errorf("no summary at %s", b)
		}
		if _, err := call.Append("forget", forgetEntry{Lo: b.Lo, Hi: b.Hi}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Forgot %s and the summaries built on it. Call memory_nap to redo them.", b)), nil
	})

	sdk.Op(c, "wake", "Read your memory. Call this first in every session and do what it says, to the end of its output.", sdk.Read, func(_ *sdk.Call, in wakeIn) (sdk.Result, error) {
		s, err := m.Wake(in.Part, in.At)
		return sdk.Text(s), err
	})
	c.Instructions(func(text string) string { return text + "\n\n" + protocolText })
	return nil
}
