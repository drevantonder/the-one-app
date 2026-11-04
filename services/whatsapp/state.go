package main

// A link's state, beside its database as <db>.state: none while it's linked
// (or a spare), else what keeps it from connecting, which a restart keeps.
//
//	linked ──move──▶ handing-off(to, move) ──the target took it──▶ handed off: deleted here
//	                               └────────the target refused it──▶ linked again
//	linked ──another copy connected──▶ replaced ──one of the user's devices holds it──▶ deleted here
//	linked ──the phone unlinked it──▶ unlinked: deleted here, as sign_out does, and noted

import (
	"encoding/json"
	"errors"
	"log"
	"os"
	"time"

	"go.mau.fi/whatsmeow"
)

const (
	handingOff = "handing-off"
	replaced   = "replaced"
	unlinked   = "unlinked"
)

type linkState struct {
	State string    `json:"state,omitempty"`
	To    string    `json:"to,omitempty"`   // handing-off: the device it goes to
	Move  string    `json:"move,omitempty"` // handing-off: the move's id, which the target answers for
	At    time.Time `json:"at,omitzero"`
}

// connects says whether a link in this state connects to WhatsApp.
func (s linkState) connects() bool { return s.State == "" || s.State == unlinked }

func loadState(path string) linkState {
	var s linkState
	if b, err := os.ReadFile(path + ".state"); err == nil {
		if err := json.Unmarshal(b, &s); err != nil {
			log.Printf("%s.state: %v", path, err)
		}
	}
	return s
}

// setState keeps the link's state, on disk before in memory, with l.mu held.
func (l *link) setState(s linkState) error {
	if err := writeDurably(l.path+".state", s, s.State == ""); err != nil {
		return err
	}
	l.st = s
	return nil
}

// writeDurably writes v as JSON to path, synced, by renaming a new file over
// it; with none, it deletes it.
func writeDurably(path string, v any, none bool) error {
	if none {
		if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		return nil
	}
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	tmp := path + ".part"
	f, err := os.OpenFile(tmp, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return err
	}
	_, err = f.Write(b)
	if err := errors.Join(err, f.Sync(), f.Close()); err != nil {
		os.Remove(tmp)
		return err
	}
	return os.Rename(tmp, path)
}

// unlinked deletes what the link kept once the phone (or WhatsApp) unlinked
// it, which deleted its keys, so a link made again starts empty, as after
// sign_out; c is the client WhatsApp unlinked, ignored once it's not the
// link's.
func (l *link) unlinked(c *whatsmeow.Client) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.client != c {
		return
	}
	l.qr, l.code = "", ""
	err := l.forget()
	if err == nil {
		err = l.setState(linkState{State: unlinked, At: time.Now().UTC()})
	}
	if err != nil {
		log.Printf("deleting what an unlinked link kept: %v", err)
	}
}

// replaced keeps the link from connecting again once another copy of it
// connected, which stopped this one: two copies would take turns.
func (l *link) replaced(c *whatsmeow.Client) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.client != c || l.client.Store.ID == nil || l.st.State != "" {
		return
	}
	if err := l.setState(linkState{State: replaced, At: time.Now().UTC()}); err != nil {
		log.Printf("keeping that this link was replaced: %v", err)
	}
}
