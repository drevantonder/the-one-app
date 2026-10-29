// Package board is the Agent Board: the user's agents leave each other
// one-line posts in topics. Every topic is an OptMem log of its own.
package board

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
	"github.com/drevantonder/the-one-app/sdk/optmem"
)

type (
	createEntry struct {
		Topic       string `json:"topic"`
		Description string `json:"description"`
		Time        string `json:"time"`
	}
	postEntry struct {
		Topic  string `json:"topic,omitempty"`
		Text   string `json:"text"`
		Time   string `json:"time"`
		Device string `json:"device,omitempty"`
		Thread string `json:"thread,omitempty"`
		Where  string `json:"where,omitempty"`
	}
	summaryEntry struct {
		Topic  string `json:"topic,omitempty"`
		Lo     int    `json:"lo"`
		Hi     int    `json:"hi"`
		Digest string `json:"digest"`
		Text   string `json:"text"`
	}

	topicsIn struct{}
	createIn struct {
		Topic       string `json:"topic" jsonschema:"the new topic's path: lowercase segments with hyphens, joined by /, like projects/the-one-app"`
		Description string `json:"description" jsonschema:"what the topic is for, one line"`
		Force       bool   `json:"force,omitempty" jsonschema:"create it even though a similar topic exists"`
	}
	readIn struct {
		Topic string `json:"topic,omitempty" jsonschema:"the topic's path; omit for the top of the board"`
		Part  int    `json:"part,omitempty" jsonschema:"which part of a long topic to read; omit for the first"`
		At    int    `json:"at,omitempty" jsonschema:"the snapshot to read the part from, as printed by the previous part"`
	}
	postIn struct {
		Topic  string `json:"topic,omitempty" jsonschema:"the topic's path; omit for the top of the board"`
		Text   string `json:"text" jsonschema:"one line, at most 280 bytes"`
		Thread string `json:"thread,omitempty" jsonschema:"a link to your conversation, if it has one"`
		Where  string `json:"where,omitempty" jsonschema:"the folder or repo you are working in"`
	}
	napIn struct {
		Topic string `json:"topic,omitempty" jsonschema:"the topic from the compression request; omit for the top"`
		Block string `json:"block,omitempty" jsonschema:"the block id from the compression request, like 0-1; omit it and text to see the next request"`
		Text  string `json:"text,omitempty" jsonschema:"the compressed line, at most 280 bytes"`
	}
	zoomIn struct {
		Topic string `json:"topic,omitempty" jsonschema:"the topic's path; omit for the top"`
		Block string `json:"block" jsonschema:"a block id as read prints it, like 16-31"`
	}
	recallIn struct {
		Pattern string `json:"pattern" jsonschema:"case-insensitive regular expression"`
		Topic   string `json:"topic,omitempty" jsonschema:"search only this topic; omit to search every topic"`
	}
)

var Plugin = sdk.Plugin{ID: "board", Name: "Agent Board", Version: "0.1.0", Setup: setup}

const protocolText = `## Agent Board

The Agent Board is where this user's agents leave each other one-line posts, in topics like ` + "`projects/the-one-app`" + `. It is shared across their devices and outlives every session.

At the start of every session, call ` + "`board_read`" + ` with no topic to read the top of the board, and do what it says, to the end of its output.

Before working on something, read its topic with ` + "`board_read`" + `. If it has none, create one in a sensible place with ` + "`board_create`" + `; ` + "`board_topics`" + ` shows them all.

Call ` + "`board_post`" + ` (one line, at most 280 bytes) when you start something, when you decide or learn something other agents should know, and when you finish. Life-level facts about the user go at the top: omit the topic.

If a tool result asks for a compression, do it with ` + "`board_nap`" + ` before your next action.

To find an old post, search with ` + "`board_recall`" + `, or open a #a-b summary with ` + "`board_zoom`" + `.

If you are a subagent, never post: you cannot judge what is already known. When you spawn a subagent, tell it: "You are a subagent. Don't post to the board."`

func setup(c *sdk.Ctx) error {
	b := New()

	c.Entry("create", sdk.Available)
	c.Entry("post", sdk.Available)
	c.Entry("summary", sdk.Available)
	c.OnReset(func() { b = New() })
	sdk.Handle(c, "create", func(_ sdk.Entry, in createEntry) error {
		b.Create(in.Topic, in.Description, in.Time)
		return nil
	})
	sdk.Handle(c, "post", func(e sdk.Entry, in postEntry) error {
		b.Post(e.ID, in)
		return nil
	})
	sdk.Handle(c, "summary", func(_ sdk.Entry, in summaryEntry) error {
		b.ensure(in.Topic).log.PutSummary(optmem.Block{Lo: in.Lo, Hi: in.Hi}, in.Digest, in.Text)
		return nil
	})

	sdk.Op(c, "topics", "List every topic as a tree: its description, its summary or latest post, and when it was last active.", sdk.Read, func(_ *sdk.Call, _ topicsIn) (sdk.Result, error) {
		return sdk.Text(b.Topics()), nil
	})

	sdk.Op(c, "create", "Create a topic under an existing one (the top always exists), with a one-line description.", sdk.Write, func(call *sdk.Call, in createIn) (sdk.Result, error) {
		path, err := b.CanCreate(in.Topic, in.Description, in.Force)
		if err != nil {
			return sdk.Result{}, err
		}
		if _, err := call.Append("create", createEntry{Topic: path, Description: strings.TrimSpace(in.Description), Time: time.Now().Format(time.RFC3339)}); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text(fmt.Sprintf("Created %s. Post to it with board_post.", path)), nil
	})

	sdk.Op(c, "read", "Read a topic: its posts, older ones summarized, then its subtopics. Omit the topic to read the top; do that first in every session and do what it says, to the end of its output.", sdk.Read, func(_ *sdk.Call, in readIn) (sdk.Result, error) {
		s, err := b.Read(in.Topic, in.Part, in.At)
		return sdk.Text(s), err
	})

	sdk.Op(c, "post", "Post one line, at most 280 bytes, to a topic; omit the topic to post at the top.", sdk.Write, func(call *sdk.Call, in postIn) (sdk.Result, error) {
		t, err := b.find(in.Topic)
		if err != nil {
			return sdk.Result{}, err
		}
		text, err := t.log.Words.Check(in.Text)
		if err != nil {
			return sdk.Result{}, err
		}
		p := postEntry{Topic: t.path, Text: text, Time: time.Now().Format(time.RFC3339), Device: deviceName(call),
			Thread: strings.TrimSpace(in.Thread), Where: strings.TrimSpace(in.Where)}
		if _, err := call.Append("post", p); err != nil {
			return sdk.Result{}, err
		}
		out := fmt.Sprintf("Posted as #%d in %s.", t.log.T()-1, label(t.path))
		if nap := b.NextNap(t.path); nap != "" {
			out += "\n\n" + nap
		}
		return sdk.Text(out), nil
	})

	sdk.Op(c, "nap", "Answer a compression request; with no block or text, show the next one.", sdk.Write, func(call *sdk.Call, in napIn) (sdk.Result, error) {
		t, err := b.find(in.Topic)
		if err != nil {
			return sdk.Result{}, err
		}
		var out []string
		if in.Block != "" || in.Text != "" {
			blk, err := optmem.ParseBlock(in.Block)
			if err != nil {
				return sdk.Result{}, err
			}
			todo := t.log.Pending(1)
			switch {
			case len(todo) == 0:
				out = append(out, fmt.Sprintf("Nothing left to compress in %s.", label(t.path)))
			case blk != todo[0]:
				if _, ok := t.log.Summary(blk); ok {
					out = append(out, fmt.Sprintf("%s is already settled.", blk))
					break
				}
				return sdk.Result{}, fmt.Errorf("wrong block %s: blocks are compressed in order, and the next in %s is %s", blk, label(t.path), todo[0])
			default:
				text, err := t.log.Words.Check(in.Text)
				if err != nil {
					return sdk.Result{}, err
				}
				if _, err := call.Append("summary", summaryEntry{Topic: t.path, Lo: blk.Lo, Hi: blk.Hi, Digest: t.log.Digest(blk), Text: text}); err != nil {
					return sdk.Result{}, err
				}
				out = append(out, fmt.Sprintf("%s saved.", blk))
			}
		}
		if nap := b.NextNap(t.path); nap != "" {
			out = append(out, nap)
		} else {
			out = append(out, "Nothing left to compress.")
		}
		return sdk.Text(strings.Join(out, "\n\n")), nil
	})

	sdk.Op(c, "zoom", "Open a #a-b summary in a topic into its two halves, down to the raw posts.", sdk.Read, func(_ *sdk.Call, in zoomIn) (sdk.Result, error) {
		t, err := b.find(in.Topic)
		if err != nil {
			return sdk.Result{}, err
		}
		blk, err := optmem.ParseBlock(in.Block)
		if err != nil {
			return sdk.Result{}, err
		}
		s, err := t.log.Zoom(blk)
		return sdk.Text(s), err
	})

	sdk.Op(c, "recall", "Search posts, word for word, with a case-insensitive regular expression: in one topic, or in every topic.", sdk.Read, func(_ *sdk.Call, in recallIn) (sdk.Result, error) {
		s, err := b.Recall(in.Pattern, in.Topic)
		return sdk.Text(s), err
	})

	c.Instructions(func(text string) string { return text + "\n\n" + protocolText })
	return nil
}

// deviceName is this device's name as the user knows it.
func deviceName(call *sdk.Call) string {
	if res, err := call.Invoke("one", "devices", struct{}{}); err == nil {
		var data struct {
			Devices []struct {
				Name string `json:"name"`
				Self bool   `json:"self"`
			} `json:"devices"`
		}
		raw, _ := json.Marshal(res.Data)
		if json.Unmarshal(raw, &data) == nil {
			for _, d := range data.Devices {
				if d.Self && d.Name != "" {
					return d.Name
				}
			}
		}
	}
	host, _ := os.Hostname()
	host, _, _ = strings.Cut(host, ".")
	return strings.ToLower(host)
}
