// Package messenger is the built-in plugin that reads Facebook Messenger.
// Messenger has no API for personal accounts and its chats are end-to-end
// encrypted, so it reads messenger.com in a browser: in a Space it borrows
// (see the spaces plugin), signed in with the user's login for everything,
// where they sign in to messenger.com once:
//
//	one spaces sign_in   # open messenger.com there and sign in
package messenger

import (
	"cmp"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

var Plugin = sdk.Plugin{ID: "messenger", Name: "Messenger", Version: "0.2.0", Setup: setup}

const (
	login = "default" // the Spaces login signed in to messenger.com
	home  = "https://www.messenger.com/"
)

type chatsIn struct {
	Unread bool `json:"unread,omitempty" jsonschema:"only unread chats"`
}

type readIn struct {
	Chat string `json:"chat" jsonschema:"the chat's name, or a part of it only one recent chat has"`
}

type sendIn struct {
	Chat string `json:"chat" jsonschema:"the chat's name, or a part of it only one recent chat has; anyone else by their exact name"`
	Text string `json:"text" jsonschema:"the message"`
}

type searchIn struct {
	Query string `json:"query" jsonschema:"words to find in messages"`
}

func setup(c *sdk.Ctx) error {
	b := &browser{session: "messenger"}

	sdk.Op(c, "chats", "List recent Messenger chats: who, their last message, when, and whether unread.", sdk.Read, func(call *sdk.Call, in chatsIn) (sdk.Result, error) {
		b.mu.Lock()
		defer b.mu.Unlock()
		page, err := b.page(call)
		if err != nil {
			return sdk.Result{}, err
		}
		chats := page.chats()
		if len(chats) == 0 {
			return sdk.Result{}, errors.New("no chats found: is your login signed in to messenger.com? Sign in with `one spaces sign_in`")
		}
		var shown []Chat
		var lines []string
		for _, ch := range chats {
			if in.Unread && !ch.Unread {
				continue
			}
			shown = append(shown, ch)
			mark := ""
			if ch.Unread {
				mark = " (unread)"
			}
			lines = append(lines, fmt.Sprintf("%s%s, %s: %s", ch.Name, mark, ch.Time, ch.Last))
		}
		if len(shown) == 0 {
			return sdk.Result{Text: "No unread chats.", Data: []Chat{}}, nil
		}
		return sdk.Result{Text: strings.Join(lines, "\n"), Data: shown}, nil
	})

	sdk.Op(c, "read", "Read a recent chat's messages. An unread chat is marked unread again afterwards, though the other side may still see it as seen.", sdk.Write, func(call *sdk.Call, in readIn) (sdk.Result, error) {
		b.mu.Lock()
		defer b.mu.Unlock()
		page, err := b.page(call)
		if err != nil {
			return sdk.Result{}, err
		}
		ch, err := pick(page.chats(), in.Chat)
		if err != nil {
			return sdk.Result{}, err
		}
		if err := b.click(call, page, "link", ch.link); err != nil {
			return sdk.Result{}, err
		}
		b.dirty = true
		var msgs []Message
		deadline := time.Now().Add(8 * time.Second)
		for {
			page, err = b.snapshot(call, "Message sent")
			if err != nil {
				return sdk.Result{}, err
			}
			if msgs = parseMessages(page.nodes); len(msgs) > 0 || time.Now().After(deadline) {
				break
			}
			time.Sleep(500 * time.Millisecond)
		}
		note := ""
		if ch.Unread {
			if _, err := b.page(call); err != nil {
				note = fmt.Sprintf("\n(Couldn't mark it unread again: %v)", err)
			} else if err := b.markUnread(call, ch.Name); err != nil {
				note = fmt.Sprintf("\n(Couldn't mark it unread again: %v)", err)
			}
		}
		if len(msgs) == 0 {
			return sdk.Result{}, fmt.Errorf("no messages showed in %s", ch.Name)
		}
		var lines []string
		for _, m := range msgs {
			lines = append(lines, fmt.Sprintf("[%s] %s: %s", m.Time, m.Sender, m.Text))
		}
		return sdk.Result{Text: strings.Join(lines, "\n") + note, Data: msgs}, nil
	})

	sdk.Op(c, "send", "Send a message, as the signed-in account, to a recent chat or to a person by their exact name.", sdk.Write, func(call *sdk.Call, in sendIn) (sdk.Result, error) {
		if strings.TrimSpace(in.Text) == "" || strings.TrimSpace(in.Chat) == "" {
			return sdk.Result{}, errors.New("chat and text are required")
		}
		b.mu.Lock()
		defer b.mu.Unlock()
		name, err := b.openChat(call, in.Chat)
		if err != nil {
			return sdk.Result{}, err
		}
		var box string
		for range 16 { // the conversation loads; on a narrow page its details may cover it
			p, err := b.snapshot(call, "Write to")
			if err != nil {
				return sdk.Result{}, err
			}
			for _, r := range p.refs {
				if r.Role == "textbox" && strings.HasPrefix(r.Name, "Write to") {
					box = r.Ref
				}
			}
			if box != "" {
				break
			}
			if back, _ := b.snapshot(call, "Back"); back.ref("button", "Back") != "" {
				_ = b.click(call, back, "button", "Back")
			}
			time.Sleep(500 * time.Millisecond)
		}
		if box == "" {
			return sdk.Result{}, fmt.Errorf("no message box in the chat with %s", name)
		}
		if _, err := b.do(call, "browser_type", map[string]any{"ref": box, "text": in.Text}); err != nil {
			return sdk.Result{}, err
		}
		if _, err := b.do(call, "browser_type", map[string]any{"ref": box, "text": "\n", "mode": "keystrokes"}); err != nil {
			return sdk.Result{}, err
		}
		// Confirm it from the chat list, which is quick to read where the
		// conversation may not be.
		want := "You: " + strings.Join(strings.Fields(in.Text), " ")
		time.Sleep(time.Second)
		for range 10 {
			p, err := b.page(call)
			if err != nil {
				return sdk.Result{}, err
			}
			if ch, err := pick(p.chats(), name); err == nil && strings.Join(strings.Fields(ch.Last), " ") == want {
				return sdk.Result{Text: fmt.Sprintf("Sent to %s.", ch.Name), Data: ch}, nil
			}
			time.Sleep(500 * time.Millisecond)
		}
		return sdk.Result{}, fmt.Errorf("typed the message to %s, but it didn't show as sent: check the messenger Space", name)
	})

	sdk.Op(c, "search", "Search Messenger for messages containing words, across all chats.", sdk.Read, func(call *sdk.Call, in searchIn) (sdk.Result, error) {
		if strings.TrimSpace(in.Query) == "" {
			return sdk.Result{}, errors.New("query is required")
		}
		b.mu.Lock()
		defer b.mu.Unlock()
		page, err := b.page(call)
		if err != nil {
			return sdk.Result{}, err
		}
		b.dirty = true // the list shows results until the page is reloaded
		box := page.ref("combobox", "Search Messenger")
		if box == "" {
			return sdk.Result{}, errors.New("no search box on the page")
		}
		if _, err := b.do(call, "browser_type", map[string]any{"ref": box, "text": in.Query, "replace": true}); err != nil {
			return sdk.Result{}, err
		}
		var link string
		for range 10 {
			if page, err = b.snapshot(call, "Search messages for"); err != nil {
				return sdk.Result{}, err
			}
			if link = page.ref("link", "Search messages for "+in.Query); link != "" {
				break
			}
			time.Sleep(300 * time.Millisecond)
		}
		if link == "" {
			return sdk.Result{}, errors.New("Messenger didn't offer to search messages")
		}
		if _, err := b.do(call, "browser_click", map[string]any{"ref": link, "input_route": "dom_event"}); err != nil {
			return sdk.Result{}, err
		}
		var matches []Match
		for range 10 {
			time.Sleep(500 * time.Millisecond)
			if page, err = b.snapshot(call, ""); err != nil {
				return sdk.Result{}, err
			}
			if matches = parseMatches(page.nodes); len(matches) > 0 {
				break
			}
		}
		if len(matches) == 0 {
			return sdk.Result{Text: "Nothing found.", Data: []Match{}}, nil
		}
		var lines []string
		for _, m := range matches {
			if m.Matches > 0 {
				lines = append(lines, fmt.Sprintf("%s: %d matching messages", m.Chat, m.Matches))
			} else {
				lines = append(lines, fmt.Sprintf("%s, %s: %s", m.Chat, m.Time, m.Text))
			}
		}
		return sdk.Result{Text: strings.Join(lines, "\n"), Data: matches}, nil
	})
	return nil
}

// browser is the Space's browser, attached through the spaces plugin.
type browser struct {
	mu      sync.Mutex
	space   string // the borrowed Space; given back when idle, then borrowed again
	session string // the Space's tools group work into sessions; the browser lives in this one
	tries   int
	target  string
	tab     string
	dirty   bool // the page is off the chat list: a conversation, or search results
}

type page struct {
	url   string
	nodes []node
	parts [][]node // the page comes in parts; a row cut between two shows in both
	refs  []struct {
		Ref  string `json:"ref"`
		Role string `json:"role"`
		Name string `json:"name"`
	}
}

// chats are the chat list's rows, each once: a row cut between two parts
// of the page shows in both, each missing what the other has.
func (p page) chats() []Chat {
	var out []Chat
	at := map[string]int{}
	for _, part := range p.parts {
		for _, c := range parseChats(part) {
			i, seen := at[c.Name]
			if !seen {
				at[c.Name] = len(out)
				out = append(out, c)
				continue
			}
			link := cmp.Or(out[i].link, c.link)
			if out[i].Time == "" && c.Time != "" {
				out[i] = c
			}
			out[i].link = link
		}
	}
	return out
}

func (p page) ref(role, name string) string {
	for _, r := range p.refs {
		if r.Role == role && r.Name == name {
			return r.Ref
		}
	}
	return ""
}

// borrow takes a Space with the login, unless one is borrowed.
func (b *browser) borrow(call *sdk.Call) error {
	if b.space != "" {
		return nil
	}
	res, err := call.Invoke("spaces", "borrow", map[string]any{"for": "Reading Messenger", "login": login})
	if err != nil {
		return err
	}
	b.space, _ = res.Data.(map[string]any)["space"].(string)
	b.target, b.tab = "", ""
	if b.space == "" {
		return errors.New("couldn't borrow a Space")
	}
	return nil
}

// do runs one of the Space's tools, borrowing a Space again if the last one
// was given back.
func (b *browser) do(call *sdk.Call, tool string, in map[string]any) (map[string]any, error) {
	data, err := b.try(call, tool, in)
	if err != nil && strings.Contains(err.Error(), "no Space named") {
		b.space = ""
		if tool != "browser_prepare" {
			if err := b.open(call); err != nil {
				return nil, err
			}
		}
		data, err = b.try(call, tool, in)
	}
	return data, err
}

func (b *browser) try(call *sdk.Call, tool string, in map[string]any) (map[string]any, error) {
	if err := b.borrow(call); err != nil {
		return nil, err
	}
	in["space"], in["session"] = b.space, b.session
	if b.target != "" && tool != "browser_prepare" && tool != "list_windows" {
		in["target_id"], in["tab_id"] = b.target, b.tab
	}
	res, err := call.Invoke("spaces", tool, in)
	if err != nil {
		return nil, err
	}
	data, _ := res.Data.(map[string]any)
	if s, _ := data["status"].(string); s == "refused" {
		return nil, errors.New(res.Text)
	}
	return data, nil
}

// open starts the browser if needed and attaches to its tab.
func (b *browser) open(call *sdk.Call) error {
	b.target, b.tab = "", ""
	prepared, err := b.do(call, "browser_prepare", map[string]any{"allow_launch": true, "profile": map[string]any{"mode": "isolated_named", "name": login}})
	if err != nil && strings.Contains(err.Error(), "to this transport") && b.tries < 5 {
		// The spaces plugin restarted, so the session's name belongs to its
		// previous connection: carry on in a new one.
		b.tries++
		b.session = fmt.Sprintf("messenger-%d", time.Now().UnixNano())
		return b.open(call)
	}
	if err != nil {
		return fmt.Errorf("starting the browser in Space %s: %w", b.space, err)
	}
	pid, _ := prepared["prepared_pid"].(float64)
	windows, err := b.do(call, "list_windows", map[string]any{"pid": int(pid)})
	if err != nil {
		return err
	}
	ws, _ := windows["windows"].([]any)
	if len(ws) == 0 {
		return errors.New("the browser has no window")
	}
	w, _ := ws[0].(map[string]any)
	bound, err := b.do(call, "get_browser_state", map[string]any{"pid": int(pid), "window_id": w["window_id"]})
	if err != nil {
		return err
	}
	b.target, _ = bound["target_id"].(string)
	tabs, _ := bound["tabs"].([]any)
	for _, t := range tabs { // the active tab, or else the first
		if t, _ := t.(map[string]any); t["active"] == true || b.tab == "" {
			b.tab, _ = t["tab_id"].(string)
		}
	}
	if b.target == "" || b.tab == "" {
		return errors.New("couldn't attach to the browser")
	}
	b.dirty = true
	return nil
}

// snapshot reads the page, or the parts matching query; it reattaches once
// if the browser closed (it does after a few idle minutes).
func (b *browser) snapshot(call *sdk.Call, query string) (page, error) {
	for attempt := range 2 {
		if b.target == "" || attempt > 0 {
			if err := b.open(call); err != nil {
				return page{}, err
			}
		}
		in := map[string]any{"snapshot_format": "semantic_v2"}
		if query != "" {
			in["query"] = query
		}
		var p page
		outline := ""
		var err error
		for range 6 { // the page comes in parts
			var data map[string]any
			if data, err = b.do(call, "get_browser_state", in); err != nil {
				break
			}
			raw, _ := json.Marshal(data)
			var part struct {
				Outline string `json:"outline"`
				Refs    []struct {
					Ref  string `json:"ref"`
					Role string `json:"role"`
					Name string `json:"name"`
				} `json:"refs"`
				Page     struct{ URL string } `json:"page"`
				Snapshot struct {
					Continuation string `json:"continuation"`
				} `json:"snapshot"`
			}
			_ = json.Unmarshal(raw, &part)
			outline += part.Outline + "\n"
			p.parts = append(p.parts, parseOutline(part.Outline))
			p.refs = append(p.refs, part.Refs...)
			p.url = part.Page.URL
			if part.Snapshot.Continuation == "" {
				break
			}
			in["continuation"] = part.Snapshot.Continuation
		}
		if err == nil {
			p.nodes = parseOutline(outline)
			b.tries = 0
			return p, nil
		}
		if attempt > 0 {
			return page{}, err
		}
	}
	return page{}, errors.New("unreachable")
}

// page reads the chat list, loading it unless it is showing. Anything else
// is loaded over rather than read: the browser tools can take 20 seconds
// to read a page as deeply nested as some conversations.
func (b *browser) page(call *sdk.Call) (page, error) {
	if b.target != "" && !b.dirty {
		p, err := b.snapshot(call, "")
		if err != nil {
			return page{}, err
		}
		if p.url == home && len(p.chats()) > 0 {
			return p, nil
		}
	}
	var p page
	for range 2 { // if the list doesn't show, load it once more
		if err := b.navigate(call, home); err != nil {
			return page{}, err
		}
		b.dirty = false
		for range 40 { // a browser that just started can take a while
			time.Sleep(500 * time.Millisecond)
			var err error
			if p, err = b.snapshot(call, ""); err != nil {
				return page{}, err
			}
			if len(p.chats()) > 0 {
				return p, nil
			}
		}
	}
	return p, nil
}

// navigate loads url, reattaching once if the browser closed.
func (b *browser) navigate(call *sdk.Call, url string) error {
	if b.target == "" {
		if err := b.open(call); err != nil {
			return err
		}
	}
	if _, err := b.do(call, "browser_navigate", map[string]any{"url": url}); err == nil {
		return nil
	}
	if err := b.open(call); err != nil {
		return err
	}
	_, err := b.do(call, "browser_navigate", map[string]any{"url": url})
	return err
}

func (b *browser) click(call *sdk.Call, p page, role, name string) error {
	ref := p.ref(role, name)
	if ref == "" {
		return fmt.Errorf("%s %q isn't on the page", role, name)
	}
	_, err := b.do(call, "browser_click", map[string]any{"ref": ref, "input_route": "dom_event"})
	return err
}

// openChat opens a recent chat, or else the person whose name is exactly
// want among Messenger's search suggestions, and returns its name. Anything
// less certain is refused, so a message never goes to the wrong person.
func (b *browser) openChat(call *sdk.Call, want string) (string, error) {
	p, err := b.page(call)
	if err != nil {
		return "", err
	}
	b.dirty = true
	if ch, err := pick(p.chats(), want); err == nil {
		return ch.Name, b.click(call, p, "link", ch.link)
	} else if !strings.HasPrefix(err.Error(), "no recent chat") {
		return "", err
	}
	box := p.ref("combobox", "Search Messenger")
	if box == "" {
		return "", errors.New("no search box on the page")
	}
	if _, err := b.do(call, "browser_type", map[string]any{"ref": box, "text": want, "replace": true}); err != nil {
		return "", err
	}
	var found []string
	for range 10 {
		time.Sleep(500 * time.Millisecond)
		if p, err = b.snapshot(call, want); err != nil {
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
			_, err := b.do(call, "browser_click", map[string]any{"ref": exact[0], "input_route": "dom_event"})
			return strings.TrimSpace(want), err
		case len(exact) > 1:
			return "", fmt.Errorf("several people are called %q: send from the messenger Space instead", want)
		}
	}
	if len(found) == 0 {
		return "", fmt.Errorf("no chat or person called %q", want)
	}
	return "", fmt.Errorf("no chat or person called exactly %q; Messenger suggests: %s", want, strings.Join(found, "; "))
}

// markUnread uses the chat's menu to mark it unread.
func (b *browser) markUnread(call *sdk.Call, chat string) error {
	p, err := b.snapshot(call, menuPrefix+chat)
	if err != nil {
		return err
	}
	if err := b.click(call, p, "button", menuPrefix+chat); err != nil {
		return err
	}
	for range 10 {
		time.Sleep(300 * time.Millisecond)
		if p, err = b.snapshot(call, "Mark as unread"); err != nil {
			return err
		}
		if p.ref("menuitem", "Mark as unread") != "" {
			return b.click(call, p, "menuitem", "Mark as unread")
		}
	}
	return errors.New("its menu had no Mark as unread")
}
