package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"
)

var errNoChat = errors.New("no recent chat called")

func (b *browser) run(op string, raw json.RawMessage) (any, error) {
	var in struct {
		Unread bool   `json:"unread"`
		Chat   string `json:"chat"`
		Text   string `json:"text"`
		Query  string `json:"query"`
	}
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, fmt.Errorf("bad input: %w", err)
		}
	}
	if op == "sign_in" {
		return b.signIn()
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	switch op {
	case "chats":
		return b.chats(in.Unread)
	case "read":
		return b.read(in.Chat)
	case "search":
		return b.search(in.Query)
	case "send":
		return b.send(in.Chat, in.Text)
	}
	return nil, fmt.Errorf("no operation %s", op)
}

func (b *browser) chats(unread bool) (any, error) {
	p, err := b.list()
	if err != nil {
		return nil, err
	}
	var lines []string
	for _, c := range p.chats() {
		if unread && !c.Unread {
			continue
		}
		mark := ""
		if c.Unread {
			mark = " (unread)"
		}
		lines = append(lines, fmt.Sprintf("%s%s, %s: %s", c.Name, mark, c.Time, c.Last))
	}
	if len(lines) == 0 {
		return "No unread chats.", nil
	}
	return strings.Join(lines, "\n"), nil
}

func (b *browser) read(chat string) (any, error) {
	if strings.TrimSpace(chat) == "" {
		return nil, errors.New("which chat?")
	}
	p, err := b.list()
	if err != nil {
		return nil, err
	}
	c, err := pick(p.chats(), chat)
	if err != nil {
		return nil, err
	}
	b.dirty = true
	if err := b.click(p, "link", c.link); err != nil {
		return nil, err
	}
	var msgs []Message
	for deadline := time.Now().Add(10 * time.Second); len(msgs) == 0 && time.Now().Before(deadline); {
		time.Sleep(500 * time.Millisecond)
		p, err := b.snapshot("Message sent")
		if err != nil {
			return nil, err
		}
		msgs = parseMessages(p.nodes)
	}
	note := ""
	if c.Unread {
		if _, err := b.list(); err != nil {
			note = fmt.Sprintf("\n(Couldn't mark it unread again: %v)", err)
		} else if err := b.markUnread(c.Name); err != nil {
			note = fmt.Sprintf("\n(Couldn't mark it unread again: %v)", err)
		} else {
			note = "\n(Marked unread again.)"
		}
	}
	if len(msgs) == 0 {
		return nil, fmt.Errorf("no messages showed in %s%s", c.Name, note)
	}
	var lines []string
	for _, m := range msgs {
		lines = append(lines, fmt.Sprintf("[%s] %s: %s", m.Time, m.Sender, m.Text))
	}
	return strings.Join(lines, "\n") + note, nil
}

func (b *browser) search(query string) (any, error) {
	if strings.TrimSpace(query) == "" {
		return nil, errors.New("what to search for?")
	}
	p, err := b.list()
	if err != nil {
		return nil, err
	}
	b.dirty = true // the list shows results until it's loaded again
	box := p.ref("combobox", "Search Messenger")
	if box == "" {
		return nil, errors.New("no search box on the page")
	}
	if _, err := b.tool("browser_type", map[string]any{"ref": box, "text": query, "replace": true}); err != nil {
		return nil, err
	}
	var link string
	for range 10 {
		if p, err = b.snapshot("Search messages for"); err != nil {
			return nil, err
		}
		if link = p.ref("link", "Search messages for "+query); link != "" {
			break
		}
		time.Sleep(300 * time.Millisecond)
	}
	if link == "" {
		return nil, errors.New("Messenger didn't offer to search messages")
	}
	if _, err := b.tool("browser_click", map[string]any{"ref": link, "input_route": "dom_event"}); err != nil {
		return nil, err
	}
	var found []Match
	for range 10 {
		time.Sleep(500 * time.Millisecond)
		if p, err = b.snapshot(""); err != nil {
			return nil, err
		}
		if found = parseMatches(p.nodes); len(found) > 0 {
			break
		}
	}
	if len(found) == 0 {
		return "Nothing found.", nil
	}
	var lines []string
	for _, m := range found {
		if m.Matches > 0 {
			lines = append(lines, fmt.Sprintf("%s: %d matching messages", m.Chat, m.Matches))
		} else {
			lines = append(lines, fmt.Sprintf("%s, %s: %s", m.Chat, m.Time, m.Text))
		}
	}
	return strings.Join(lines, "\n"), nil
}

func (b *browser) send(chat, text string) (any, error) {
	if strings.TrimSpace(text) == "" || strings.TrimSpace(chat) == "" {
		return nil, errors.New("chat and text are required")
	}
	name, err := b.openChat(chat)
	if err != nil {
		return nil, err
	}
	var box string
	for range 16 { // the conversation loads; on a narrow page its details may cover it
		p, err := b.snapshot("Write to")
		if err != nil {
			return nil, err
		}
		for _, r := range p.refs {
			if r.Role == "textbox" && strings.HasPrefix(r.Name, "Write to") {
				box = r.Ref
			}
		}
		if box != "" {
			break
		}
		if p.ref("button", "Back") != "" {
			b.click(p, "button", "Back")
		}
		time.Sleep(500 * time.Millisecond)
	}
	if box == "" {
		return nil, fmt.Errorf("no message box in the chat with %s", name)
	}
	if _, err := b.tool("browser_type", map[string]any{"ref": box, "text": text}); err != nil {
		return nil, err
	}
	if _, err := b.tool("browser_type", map[string]any{"ref": box, "text": "\n", "mode": "keystrokes"}); err != nil {
		return nil, err
	}
	// Confirm it from the chat list, which is quick to read where the
	// conversation may not be.
	want := "You: " + strings.Join(strings.Fields(text), " ")
	time.Sleep(time.Second)
	for range 10 {
		p, err := b.list()
		if err != nil {
			return nil, err
		}
		if c, err := pick(p.chats(), name); err == nil && strings.Join(strings.Fields(c.Last), " ") == want {
			return fmt.Sprintf("Sent to %s.", c.Name), nil
		}
		time.Sleep(500 * time.Millisecond)
		b.dirty = true
	}
	return nil, fmt.Errorf("typed the message to %s, but it didn't show as sent: look at the Desktops page", name)
}

// openChat opens a recent chat, or else the one person Messenger suggests
// whose name is exactly want, and says its name. Anything less certain is
// refused, so a message never goes to the wrong person.
func (b *browser) openChat(want string) (string, error) {
	p, err := b.list()
	if err != nil {
		return "", err
	}
	b.dirty = true
	c, err := pick(p.chats(), want)
	if err == nil {
		return c.Name, b.click(p, "link", c.link)
	}
	if !errors.Is(err, errNoChat) {
		return "", err
	}
	box := p.ref("combobox", "Search Messenger")
	if box == "" {
		return "", errors.New("no search box on the page")
	}
	if _, err := b.tool("browser_type", map[string]any{"ref": box, "text": want, "replace": true}); err != nil {
		return "", err
	}
	var found []string
	for range 10 {
		time.Sleep(500 * time.Millisecond)
		if p, err = b.snapshot(want); err != nil {
			return "", err
		}
		found = found[:0]
		var exact []string
		for _, r := range p.refs {
			if r.Role != "link" || strings.HasPrefix(r.Name, "Search messages for") {
				continue
			}
			found = append(found, r.Name)
			if strings.EqualFold(r.Name, strings.TrimSpace(want)) {
				exact = append(exact, r.Ref)
			}
		}
		switch {
		case len(exact) == 1:
			_, err := b.tool("browser_click", map[string]any{"ref": exact[0], "input_route": "dom_event"})
			return strings.TrimSpace(want), err
		case len(exact) > 1:
			return "", fmt.Errorf("several people are called %q: send from the desktop yourself", want)
		}
	}
	if len(found) == 0 {
		return "", fmt.Errorf("no chat or person called %q", want)
	}
	return "", fmt.Errorf("no chat or person called exactly %q; Messenger suggests: %s", want, strings.Join(found, "; "))
}

// signIn opens messenger.com on the saved login, for the person to sign in.
func (b *browser) signIn() (any, error) {
	got, err := b.desktops("sign_in", map[string]any{"login": login, "url": home})
	if err != nil {
		return nil, err
	}
	m, _ := got.(map[string]any)
	url, _ := m["url"].(string)
	return fmt.Sprintf("Give the person this link: %s\nThey sign in to Messenger there (tick Keep me signed in), then press Save login on the Desktops page; it's saved by itself after 30 minutes otherwise.", url), nil
}
