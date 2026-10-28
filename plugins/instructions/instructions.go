// Package instructions is the built-in plugin holding the user's standing
// instructions for their agents. Every edit is kept; the latest one wins.
package instructions

import (
	"fmt"
	"strings"

	"github.com/drevantonder/the-one-app/sdk"
)

type revision struct {
	Entry string
	Time  string
	Text  string
}

type setIn struct {
	Text string `json:"text" jsonschema:"the complete new instructions; replaces the current ones"`
}

type restoreIn struct {
	Revision int `json:"revision" jsonschema:"revision number, as listed by history"`
}

type noIn struct{}

var Plugin = sdk.Plugin{ID: "instructions", Version: "0.1.0", Setup: setup}

func setup(c *sdk.Ctx) error {
	var revs []revision
	current := func() string {
		if len(revs) == 0 {
			return ""
		}
		return revs[len(revs)-1].Text
	}

	c.Entry("set", sdk.Available)
	c.OnReset(func() { revs = nil })
	sdk.Handle(c, "set", func(e sdk.Entry, in setIn) error {
		revs = append(revs, revision{Entry: e.ID, Time: e.Time, Text: in.Text})
		return nil
	})

	sdk.Op(c, "get", "Show the user's current instructions.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
		if current() == "" {
			return sdk.Text("No instructions yet. Set them with instructions_set."), nil
		}
		return sdk.Text(current()), nil
	})
	sdk.Op(c, "set", "Replace the user's instructions with new text. Only do this when the user asks; earlier versions stay in history.", sdk.Write, func(call *sdk.Call, in setIn) (sdk.Result, error) {
		text := strings.TrimSpace(in.Text)
		if text == "" {
			return sdk.Result{}, fmt.Errorf("instructions cannot be empty")
		}
		if text == current() {
			return sdk.Text("Unchanged."), nil
		}
		if _, err := call.Append("set", setIn{Text: text}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Saved as revision %d.", len(revs))), nil
	})
	sdk.Op(c, "history", "List every revision of the instructions, newest first.", sdk.Read, func(*sdk.Call, noIn) (sdk.Result, error) {
		if len(revs) == 0 {
			return sdk.Text("No revisions yet."), nil
		}
		var b strings.Builder
		for i := len(revs) - 1; i >= 0; i-- {
			first := strings.SplitN(revs[i].Text, "\n", 2)[0]
			if len(first) > 80 {
				first = first[:80] + "…"
			}
			fmt.Fprintf(&b, "%d  %s  %s\n", i+1, revs[i].Time, first)
		}
		return sdk.Text(strings.TrimRight(b.String(), "\n")), nil
	})
	sdk.Op(c, "restore", "Make an earlier revision current again (as a new revision).", sdk.Write, func(call *sdk.Call, in restoreIn) (sdk.Result, error) {
		if in.Revision < 1 || in.Revision > len(revs) {
			return sdk.Result{}, fmt.Errorf("no revision %d; there are %d", in.Revision, len(revs))
		}
		text := revs[in.Revision-1].Text
		if text == current() {
			return sdk.Text("That revision is already current."), nil
		}
		if _, err := call.Append("set", setIn{Text: text}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Restored revision %d as revision %d.", in.Revision, len(revs))), nil
	})

	c.Instructions(func(text string) string {
		if current() == "" {
			return text
		}
		return text + "\n\n## This user's instructions\n\n" + current()
	})
	return nil
}
