package main

// Several WhatsApp accounts (SPEC.md "Connectors"): each linked phone is
// a link of its own, with its own database. whatsapp.db is the first, as it
// was when there could be only one; each linked since is whatsapp-<time>.db.

import (
	"context"
	_ "embed"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"log"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/skip2/go-qrcode"
)

//go:embed settings.html
var settingsHTML string

// whatsapp is every link this server keeps, the first first.
type whatsapp struct {
	node, dir string

	mu    sync.Mutex
	links []*link
}

func openAll(dir, node string) (*whatsapp, error) {
	first, err := open(filepath.Join(dir, "whatsapp.db"), node)
	if err != nil {
		return nil, err
	}
	w := &whatsapp{node: node, dir: dir, links: []*link{first}}
	more, _ := filepath.Glob(filepath.Join(dir, "whatsapp-*.db"))
	slices.Sort(more)
	for _, path := range more {
		l, err := open(path, node)
		if err != nil {
			log.Printf("%s: %v", path, err)
			continue
		}
		if l.holder().Phone == "" { // a pairing that never finished
			if err := l.remove(); err != nil {
				log.Printf("%s: %v", path, err)
			}
			continue
		}
		w.links = append(w.links, l)
	}
	return w, nil
}

func (w *whatsapp) run(ctx context.Context, op string, raw json.RawMessage) (any, error) {
	var in struct {
		Account, Phone, To, Name string
		From, Data               string // take's
		Part, Parts              int
	}
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, err
		}
	}
	switch op {
	case "_screen":
		if in.Name != "settings" {
			return nil, fmt.Errorf("WhatsApp has no screen %q", in.Name)
		}
		return settingsHTML, nil
	case "accounts":
		return w.accounts()
	case "status":
		return w.status()
	case "sign_in":
		l, err := w.spare()
		if err != nil {
			return nil, err
		}
		return l.signIn(ctx, in.Phone)
	case "_pairing": // the settings screen's QR code and phone code, while one is pairing
		return w.pairing(), nil
	case "_holder": // the sidecar's own, between the user's servers
		return w.holding(), nil
	case "_take":
		l, err := w.spare()
		if err != nil {
			return nil, err
		}
		return l.take(in.From, in.Part, in.Parts, in.Data)
	case "move", "sign_out":
		l, err := w.pick(in.Account, op == "sign_out")
		if err != nil {
			return nil, err
		}
		var out string
		if op == "move" {
			out, err = l.move(in.To)
		} else {
			out, err = l.signOut(ctx)
		}
		if err == nil {
			w.drop(l)
		}
		return out, err
	}
	l, err := w.pick(in.Account, false)
	if err != nil {
		return nil, err
	}
	return l.run(ctx, op, raw)
}

// linked are the links paired with a phone, the first first.
func (w *whatsapp) linked() []*link {
	w.mu.Lock()
	defer w.mu.Unlock()
	var out []*link
	for _, l := range w.links {
		if l.holder().Phone != "" {
			out = append(out, l)
		}
	}
	return out
}

// pick is the account named, by its number, or the first when none is
// named; elsewhere when it isn't linked here, so the call goes on to the
// user's other servers. With named, an account must be named when there
// are several, as for signing one out.
func (w *whatsapp) pick(account string, named bool) (*link, error) {
	all := w.linked()
	if len(all) == 0 {
		return nil, elsewhere("WhatsApp isn't linked here: whatsapp_sign_in links it")
	}
	want := digits(account)
	if want == "" {
		if named && len(all) > 1 {
			var numbers []string
			for _, l := range all {
				numbers = append(numbers, "+"+l.holder().Phone)
			}
			return nil, fmt.Errorf("give account: which of %s", strings.Join(numbers, ", "))
		}
		return all[0], nil
	}
	for _, l := range all {
		if p := l.holder().Phone; p == want || strings.HasSuffix(p, want) && len(want) >= 6 {
			return l, nil
		}
	}
	return nil, elsewhere(fmt.Sprintf("WhatsApp isn't linked here as %s", account))
}

// spare is a link to pair or to take a moved link into: the first that
// isn't linked, else a new one.
func (w *whatsapp) spare() (*link, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	for _, l := range w.links {
		if l.holder().Phone == "" {
			return l, nil
		}
	}
	l, err := open(filepath.Join(w.dir, fmt.Sprintf("whatsapp-%d.db", time.Now().UnixNano())), w.node)
	if err != nil {
		return nil, err
	}
	w.links = append(w.links, l)
	return l, nil
}

// drop deletes a link that's no longer linked, but for the first, whose
// database stays to pair again.
func (w *whatsapp) drop(l *link) {
	w.mu.Lock()
	defer w.mu.Unlock()
	i := slices.Index(w.links, l)
	if i <= 0 || l.holder().Phone != "" {
		return
	}
	w.links = slices.Delete(w.links, i, i+1)
	if err := l.remove(); err != nil {
		log.Printf("deleting %s: %v", l.path, err)
	}
}

// holding is the first account linked here, as servers told each other when
// there could be only one, and every account.
func (w *whatsapp) holding() holding {
	var h holding
	for _, l := range w.linked() {
		one := l.holder()
		if h.Phone == "" {
			h.Phone, h.Connected = one.Phone, one.Connected
		}
		h.All = append(h.All, one)
	}
	return h
}

// accounts lists every account linked on any of the user's servers that
// answered, this one's first.
func (w *whatsapp) accounts() ([]map[string]any, error) {
	s, err := w.links[0].servers()
	if err != nil {
		return nil, err
	}
	out := []map[string]any{}
	add := func(h holding, server string) {
		a := map[string]any{"account": "+" + h.Phone, "ok": h.Connected, "server": server}
		if !h.Connected {
			a["why"] = "Not connected on " + server + " right now: it reconnects by itself, unless the phone unlinked it"
		}
		out = append(out, a)
	}
	for _, l := range w.linked() {
		add(l.holder(), s.here)
	}
	for _, name := range s.others {
		var h holding
		if err := callOn(w.node, name, "_holder", map[string]any{}, &h); err == nil {
			for _, one := range h.all() {
				add(one, name)
			}
		}
	}
	return out, nil
}

func (w *whatsapp) status() (string, error) {
	all := w.linked()
	if len(all) == 0 {
		return w.links[0].status()
	}
	var lines []string
	for _, l := range all {
		line, err := l.status()
		if err != nil {
			return "", err
		}
		lines = append(lines, line)
	}
	return strings.Join(lines, "\n"), nil
}

// pairing is the QR code of the link pairing now, as a picture, and the code
// to type on the phone, if it was asked for one.
func (w *whatsapp) pairing() map[string]any {
	w.mu.Lock()
	defer w.mu.Unlock()
	for _, l := range w.links {
		l.mu.Lock()
		qr, code := l.qr, l.code
		l.mu.Unlock()
		if qr == "" {
			continue
		}
		png, err := qrcode.Encode(qr, qrcode.Medium, 512)
		if err != nil {
			return map[string]any{"error": err.Error()}
		}
		return map[string]any{"qr": "data:image/png;base64," + base64.StdEncoding.EncodeToString(png), "code": code}
	}
	return map[string]any{}
}

func digits(s string) string {
	return strings.Map(func(r rune) rune {
		if r >= '0' && r <= '9' {
			return r
		}
		return -1
	}, s)
}
