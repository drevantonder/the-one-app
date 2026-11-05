package main

// Several WhatsApp accounts: each linked phone is
// a link of its own, with its own database. whatsapp.db is the first, as it
// was when there could be only one; each linked since is whatsapp-<time>.db.

import (
	"context"
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

// whatsapp is every link this device keeps, the first first.
type whatsapp struct {
	node, dir string

	mu    sync.Mutex
	links []*link

	inMu     sync.Mutex          // moves arriving here, one part at a time
	arriving map[string]*arrival // by move
	moved    map[string]string   // each move's answer here, taken or refused, by move
}

func openAll(dir, node string) (*whatsapp, error) {
	first, err := open(filepath.Join(dir, "whatsapp.db"), node)
	if err != nil {
		return nil, err
	}
	w := &whatsapp{node: node, dir: dir, links: []*link{first}, arriving: map[string]*arrival{}}
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
	w.loadMoved()
	go w.settling()
	return w, nil
}

func (w *whatsapp) run(ctx context.Context, op string, raw json.RawMessage) (any, error) {
	var in struct {
		Account, Phone, To, Name string
		From, Move, Data         string // take's and moved's
		Part, Parts              int
	}
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, err
		}
	}
	switch op {
	case "accounts":
		all, _, err := w.accounts()
		return all, err
	case "_accounts": // the app's, to keep the Vault's items: with the devices that answered
		all, answered, err := w.accounts()
		if err != nil {
			return nil, err
		}
		return map[string]any{"accounts": all, "answered": answered}, nil
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
	case "_holder": // the service's own, between the user's devices
		return w.holding(), nil
	case "_take":
		return w.take(in.From, in.Move, in.Part, in.Parts, in.Data)
	case "_moved":
		return w.movedHere(in.Move)
	case "move", "sign_out":
		l, err := w.pick(in.Account, op == "sign_out")
		if err != nil {
			return nil, err
		}
		var out string
		if op == "move" {
			out, err = w.move(l, in.To)
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
// user's other devices. With named, an account must be named when there
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

// holding is the first account linked here, as devices told each other when
// there could be only one, and every account.
func (w *whatsapp) holding() holding {
	h := holding{Moves: true}
	for _, l := range w.linked() {
		one := l.holder()
		if h.Phone == "" {
			h.Phone, h.Connected, h.State = one.Phone, one.Connected, one.State
		}
		h.All = append(h.All, one)
	}
	return h
}

// accounts lists every account linked on any of the user's devices that
// answered, this one's first, and which devices answered: one that didn't
// may still hold accounts nobody can see right now.
func (w *whatsapp) accounts() ([]map[string]any, []string, error) {
	s, err := w.links[0].devices()
	if err != nil {
		return nil, nil, err
	}
	answered := []string{s.here}
	out := []map[string]any{}
	add := func(h holding, device string) {
		a := map[string]any{"account": "+" + h.Phone, "ok": h.Connected, "device": device}
		switch {
		case h.State == handingOff:
			a["why"] = "Being moved from " + device + ": it's off until the device it goes to answers"
		case h.State == replaced:
			a["why"] = "Stopped on " + device + ": another copy of this link connected elsewhere (whatsapp_status there says more)"
		case !h.Connected:
			a["why"] = "Not connected on " + device + " right now: it reconnects by itself"
		}
		out = append(out, a)
	}
	for _, l := range w.linked() {
		add(l.holder(), s.here)
	}
	for _, name := range s.others {
		var h holding
		if err := callOn(w.node, name, "_holder", map[string]any{}, &h); err == nil {
			answered = append(answered, name)
			for _, one := range h.all() {
				add(one, name)
			}
		}
	}
	return out, answered, nil
}

func (w *whatsapp) status() (string, error) {
	all := w.linked()
	if len(all) == 0 {
		return w.links[0].status()
	}
	var lines []string
	w.mu.Lock()
	for _, l := range w.links {
		l.mu.Lock()
		if l.st.State == unlinked {
			lines = append(lines, fmt.Sprintf("WhatsApp unlinked an account here %s (from the phone, or WhatsApp itself did), so what it kept here is deleted.", l.st.At.Local().Format("2 Jan 15:04")))
		}
		l.mu.Unlock()
	}
	w.mu.Unlock()
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
