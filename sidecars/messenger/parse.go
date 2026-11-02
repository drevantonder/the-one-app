package main

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"
)

// The device's browser describes a page as an outline, one node per line:
//
//	- row "Kim Unread message: Lunch? 2 hours ago More options for Kim"
//	  - link "Kim Unread message: Lunch? 2 hours ago" [focusable=true]
//	- statictext "Kim"
//
// Messenger's chat list, a conversation and search results are read from it.

type node struct {
	role, label string
}

var outlineLine = regexp.MustCompile(`^\s*- ([a-z]+)(?: "((?:[^"\\]|\\.)*)")?`)

func parseOutline(outline string) []node {
	var out []node
	for _, l := range strings.Split(outline, "\n") {
		m := outlineLine.FindStringSubmatch(l)
		if m == nil {
			continue
		}
		label := m[2]
		if strings.Contains(label, `\`) {
			if u, err := strconv.Unquote(`"` + label + `"`); err == nil {
				label = u
			}
		}
		out = append(out, node{role: m[1], label: label})
	}
	return out
}

// Chat is a row of the chat list.
type Chat struct {
	Name   string `json:"name"`
	Last   string `json:"last,omitempty"` // the last message, as the list shows it
	Time   string `json:"time,omitempty"`
	Unread bool   `json:"unread,omitempty"`
	Group  bool   `json:"group,omitempty"`
	link   string // the row's link, by its name among the page's refs
}

// Message is one message of the open conversation.
type Message struct {
	Sender string `json:"sender"` // "You" for the account's own
	Time   string `json:"time"`
	Text   string `json:"text"`
}

// Match is a chat with messages a search found.
type Match struct {
	Chat    string `json:"chat"`
	Matches int    `json:"matches,omitempty"` // when the chat has several
	Text    string `json:"text,omitempty"`    // the message, when it has one
	Time    string `json:"time,omitempty"`
}

const menuPrefix = "More options for "

// row is one row of a list: its link, its time, and its texts before the time.
type row struct {
	label, link, time string
	texts             []string
}

func rows(nodes []node) []row {
	var out []row
	var cur *row
	timed := false
	for _, n := range nodes {
		switch {
		case n.role == "row":
			out = append(out, row{label: n.label})
			cur, timed = &out[len(out)-1], false
		case cur == nil:
		case n.role == "link" && cur.link == "":
			cur.link = n.label
		case timed:
		case n.role == "abbr":
			cur.time, timed = n.label, true
		case n.role == "statictext" || n.role == "image":
			label, _, _ := strings.Cut(n.label, ". To get missing image descriptions")
			if t := strings.TrimSpace(label); t != "" {
				cur.texts = append(cur.texts, t)
			}
		}
	}
	return out
}

func parseChats(nodes []node) []Chat {
	var out []Chat
	for _, r := range rows(nodes) {
		i := strings.LastIndex(r.label, menuPrefix)
		if i < 0 {
			continue
		}
		c := Chat{Name: r.label[i+len(menuPrefix):], Time: r.time, link: r.link, Group: strings.HasPrefix(r.label, "Group chat: ")}
		var texts []string
		for _, t := range r.texts {
			switch {
			case t == "Unread message:":
				c.Unread = true
			case t == "Active now", strings.HasPrefix(t, "Active ") && strings.HasSuffix(t, " ago"):
			default:
				texts = append(texts, t)
			}
		}
		c.Last = strings.Join(withoutName(texts, c.Name), " ")
		out = append(out, c)
	}
	return out
}

// squash compares names whatever their spacing and emoji variants.
func squash(s string) string {
	return strings.Map(func(r rune) rune {
		if r == ' ' || r == ' ' || r == '️' {
			return -1
		}
		return r
	}, s)
}

// withoutName drops the leading texts that spell the chat's name: a group's
// emoji come as images between its words.
func withoutName(texts []string, name string) []string {
	want, got := squash(name), ""
	for i, t := range texts {
		if got == want {
			return texts[i:]
		}
		if next := got + squash(t); strings.HasPrefix(want, next) {
			got = next
			continue
		}
		break
	}
	if got == want {
		return nil
	}
	if len(texts) > 0 {
		return texts[1:] // the name didn't line up: it's the first text
	}
	return texts
}

var messageLabel = regexp.MustCompile(`(?s)^Enter, Message sent (.+?) by ([^:]+?)(?:: (.*))?$`)

func parseMessages(nodes []node) []Message {
	var out []Message
	for _, n := range nodes {
		m := messageLabel.FindStringSubmatch(n.label)
		if n.role != "button" || m == nil {
			continue
		}
		text := strings.TrimSpace(m[3])
		if text == "" {
			text = "(attachment)"
		}
		out = append(out, Message{Sender: m[2], Time: m[1], Text: text})
	}
	return out
}

var matchCount = regexp.MustCompile(`^(\d+) matched messages?$`)

func parseMatches(nodes []node) []Match {
	var out []Match
	for _, r := range rows(nodes) {
		if len(r.texts) == 0 || strings.Contains(r.label, menuPrefix) {
			continue
		}
		m := Match{Chat: r.texts[0], Time: r.time}
		rest := strings.Join(r.texts[1:], " ")
		if n := matchCount.FindStringSubmatch(rest); n != nil {
			m.Matches, _ = strconv.Atoi(n[1])
		} else {
			m.Text = rest
		}
		out = append(out, m)
	}
	return out
}

// pick finds the chat called want: by its whole name, or by a part of it only
// one chat has.
func pick(chats []Chat, want string) (Chat, error) {
	w := strings.ToLower(strings.TrimSpace(want))
	var partial []Chat
	for _, c := range chats {
		n := strings.ToLower(c.Name)
		if n == w {
			return c, nil
		}
		if strings.Contains(n, w) {
			partial = append(partial, c)
		}
	}
	if len(partial) == 1 {
		return partial[0], nil
	}
	names := func(cs []Chat) string {
		var out []string
		for _, c := range cs {
			out = append(out, c.Name)
		}
		return strings.Join(out, "; ")
	}
	if len(partial) > 1 {
		return Chat{}, fmt.Errorf("%q matches several chats: %s", want, names(partial))
	}
	return Chat{}, fmt.Errorf("%w %q; recent chats: %s", errNoChat, want, names(chats))
}
