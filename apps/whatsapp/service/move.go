package main

// Moving the link: this service on the device that holds it hands its
// database (the link's keys and the messages kept) to the same service on
// another of the user's devices, in calls through the nodes, end-to-end
// encrypted between them. Nothing is written anywhere else, so once the
// other device has it and this one deletes its copy, there is one copy; the
// link's state (state.go) keeps it off here until the other device answered.

import (
	"bytes"
	"compress/gzip"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/store/sqlstore"
)

// partSize is how much of the database goes in one call, well inside what a
// call may carry, even through older nodes (2 MB).
var partSize = 1 << 20

type holding struct {
	Phone     string    `json:"phone,omitempty"` // the linked number, if linked there
	Connected bool      `json:"connected,omitempty"`
	State     string    `json:"state,omitempty"` // handing-off or replaced: held there, but off
	All       []holding `json:"all,omitempty"`
	Moves     bool      `json:"moves,omitempty"` // it answers _moved, so it can take a link
}

// all is every link a device holds, from a device before several too.
func (h holding) all() []holding {
	if h.All != nil {
		return h.All
	}
	if h.Phone != "" {
		return []holding{{Phone: h.Phone, Connected: h.Connected, State: h.State}}
	}
	return nil
}

func (h holding) has(number string) bool {
	return slices.ContainsFunc(h.all(), func(o holding) bool { return o.Phone == number })
}

func (l *link) holder() holding {
	l.mu.Lock()
	defer l.mu.Unlock()
	if id := l.client.Store.ID; id != nil {
		return holding{Phone: id.User, Connected: loggedIn(l.client), State: l.st.State}
	}
	return holding{}
}

type devices struct {
	here   string
	others []string
}

func (l *link) devices() (devices, error) {
	var state struct {
		Device  struct{ Key, Name string }
		Network struct {
			Devices []struct {
				Key, Name string
				Online    bool
				Services  []string
			}
		}
	}
	body, err := request(&http.Client{Timeout: 5 * time.Second, Transport: toNode}, l.node+"/v1/state", nil)
	if err == nil {
		err = json.Unmarshal(body, &state)
	}
	if err != nil {
		return devices{}, fmt.Errorf("asking this device's node: %w", err)
	}
	s := devices{here: state.Device.Name}
	for _, o := range state.Network.Devices {
		if o.Key != state.Device.Key && o.Online && slices.Contains(o.Services, app) {
			s.others = append(s.others, o.Name)
		}
	}
	return s, nil
}

func (l *link) linkedElsewhere(s devices) (device, phone string) {
	for _, name := range s.others {
		var h holding
		if err := callOn(l.node, name, "_holder", map[string]any{}, &h); err == nil && h.Phone != "" {
			return name, h.Phone
		}
	}
	return "", ""
}

// move hands the link to the device to: it's off here from the moment it's
// handing off, which is kept, until the target says it took it (deleted here)
// or refused it (linked here again). An answer lost on the way is asked for
// again, at once and then every minute, the link staying off here meanwhile,
// so it's never connected from two devices.
func (w *whatsapp) move(l *link, to string) (string, error) {
	to = strings.TrimSpace(to)
	if to == "" {
		return "", errors.New("give to: the device to move WhatsApp to")
	}
	s, err := l.devices()
	if err != nil {
		return "", err
	}
	if strings.EqualFold(to, s.here) {
		return "", fmt.Errorf("WhatsApp is linked here, on %s, already", s.here)
	}
	mine := l.holder().Phone
	var there holding
	if err := callOn(l.node, to, "_holder", map[string]any{}, &there); err != nil {
		return "", fmt.Errorf("not moved: %s didn't answer (it needs WhatsApp installed and to be online): %w", to, err)
	}
	if mine != "" && there.has(mine) {
		return "", fmt.Errorf("not moved: %s is linked to WhatsApp already, as +%s", to, mine)
	}
	if !there.Moves {
		return "", fmt.Errorf("not moved: WhatsApp on %s is older than here, and can't say whether it took it: update it there first", to)
	}

	l.mu.Lock()
	if l.client.Store.ID == nil {
		l.mu.Unlock()
		return "", errors.New("WhatsApp isn't linked here")
	}
	if l.st.State != "" {
		l.mu.Unlock()
		return "", fmt.Errorf("not moved: it's %s here (whatsapp_status says more)", l.st.State)
	}
	number := l.client.Store.ID.User
	id := rand.Text()
	if err := l.setState(linkState{State: handingOff, To: to, Move: id, At: time.Now().UTC()}); err != nil {
		l.mu.Unlock()
		return "", fmt.Errorf("not moved: %w", err)
	}
	l.client.Disconnect() // so the link is never connected from two devices
	session, err := snapshot(l.db, filepath.Dir(l.path))
	if err != nil { // nothing was sent
		if serr := l.setState(linkState{}); serr != nil {
			err = errors.Join(err, serr)
		} else {
			l.reconnect()
		}
		l.mu.Unlock()
		return "", fmt.Errorf("not moved: %w", err)
	}
	l.mu.Unlock()
	data := base64.StdEncoding.EncodeToString(session)
	parts := (len(data) + partSize - 1) / partSize
	for i := 0; i < parts && err == nil; i++ {
		part := data[i*partSize : min((i+1)*partSize, len(data))]
		err = callOn(l.node, to, "_take", map[string]any{"from": s.here, "move": id, "part": i, "parts": parts, "data": part}, &there)
	}
	if err == nil && !there.has(number) {
		err = fmt.Errorf("%s answered without it", to)
	}
	if err == nil {
		l.mu.Lock()
		err = l.handedOff()
		l.mu.Unlock()
		w.drop(l)
		if err != nil {
			return "", fmt.Errorf("moved to %s, but deleting it here failed: %w", to, err)
		}
	} else {
		switch w.settle(l) {
		case "":
			return "", fmt.Errorf("not moved, and WhatsApp stays off here until %s answers whether it took it, which this device asks it every minute: %w", to, err)
		case "refused":
			return "", fmt.Errorf("not moved, still linked here: %w", err)
		}
		there = holding{Connected: false}
	}
	state := "connected"
	if !there.Connected {
		state = "connecting"
	}
	return fmt.Sprintf("Moved WhatsApp from %s to %s, as +%s, with its messages; it's %s there and gone from %s. Messages sent meanwhile arrive there.",
		s.here, to, number, state, s.here), nil
}

// With l.mu held.
func (l *link) handedOff() error {
	if l.st.State != handingOff {
		return nil
	}
	log.Printf("handed the link to %s", l.st.To)
	return l.forget()
}

// settle finishes a link that's handing off or was replaced, if it can now:
// "taken" or "refused" for a handoff the target answered, "deleted" for a
// replaced link one of the user's devices holds, "" if it's as it was.
func (w *whatsapp) settle(l *link) string {
	l.mu.Lock()
	st := l.st
	number := ""
	if id := l.client.Store.ID; id != nil {
		number = id.User
	}
	l.mu.Unlock()
	switch st.State {
	case handingOff:
		var answer struct{ Taken bool }
		if err := callOn(l.node, st.To, "_moved", map[string]any{"move": st.Move}, &answer); err != nil {
			log.Printf("asking %s whether it took the link: %v", st.To, err)
			return ""
		}
		l.mu.Lock()
		defer func() {
			l.mu.Unlock()
			w.drop(l)
		}()
		if l.st.Move != st.Move {
			return ""
		}
		if answer.Taken {
			if err := l.handedOff(); err != nil {
				log.Printf("deleting the link handed to %s: %v", st.To, err)
				return ""
			}
			return "taken"
		}
		if err := l.setState(linkState{}); err != nil {
			log.Printf("%v", err)
			return ""
		}
		log.Printf("%s didn't take the link: connecting here again", st.To)
		l.reconnect()
		return "refused"
	case replaced:
		s, err := l.devices()
		if err != nil {
			return ""
		}
		for _, name := range s.others {
			var h holding
			if callOn(l.node, name, "_holder", map[string]any{}, &h) != nil {
				continue
			}
			for _, one := range h.all() {
				if one.Phone != number || one.State != "" {
					continue
				}
				l.mu.Lock()
				err := error(nil)
				if l.st.State == replaced {
					log.Printf("%s holds this link, whose copy here was replaced: deleting it here", name)
					err = l.forget()
				}
				l.mu.Unlock()
				w.drop(l)
				if err != nil {
					log.Printf("deleting a replaced link: %v", err)
					return ""
				}
				return "deleted"
			}
		}
	}
	return ""
}

func (w *whatsapp) settling() {
	for {
		w.mu.Lock()
		all := slices.Clone(w.links)
		w.mu.Unlock()
		for _, l := range all {
			l.mu.Lock()
			waiting := !l.st.connects()
			l.mu.Unlock()
			if waiting {
				w.settle(l)
			}
		}
		time.Sleep(time.Minute)
	}
}

func (l *link) reconnect() {
	if err := connect(l.client); err != nil {
		log.Printf("connecting again: %v", err)
	}
}

func snapshot(db *sql.DB, dir string) ([]byte, error) {
	f, err := os.CreateTemp(dir, "moving-*.db")
	if err != nil {
		return nil, err
	}
	path := f.Name()
	f.Close()
	os.Remove(path) // VACUUM INTO makes it
	defer os.Remove(path)
	if _, err := db.Exec(`VACUUM INTO ?`, path); err != nil {
		return nil, err
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var b bytes.Buffer
	z := gzip.NewWriter(&b)
	if _, err := z.Write(raw); err != nil {
		return nil, err
	}
	if err := z.Close(); err != nil {
		return nil, err
	}
	return b.Bytes(), nil
}

type arrival struct {
	file string
	next int
}

// take receives the link another device moves here, part by part, and links
// this device with it, as a link of its own, once it has every part. Once
// it's here, or _moved said it isn't, the move's answer never changes: one
// device holds the link.
func (w *whatsapp) take(from, move string, part, parts int, data string) (holding, error) {
	if move == "" {
		return holding{}, fmt.Errorf("a move from %s names no move", from)
	}
	w.inMu.Lock()
	locked := true
	defer func() {
		if locked {
			w.inMu.Unlock()
		}
	}()
	if done := w.moved[move]; done != "" {
		return holding{}, fmt.Errorf("that move from %s is %s here already", from, done)
	}
	a := w.arriving[move]
	if part == 0 {
		if a != nil {
			os.Remove(a.file)
		}
		h := sha256.Sum256([]byte(move))
		a = &arrival{file: filepath.Join(w.dir, "incoming-"+hex.EncodeToString(h[:8]))}
		w.arriving[move] = a
		os.Remove(a.file)
	}
	if a == nil || part != a.next {
		return holding{}, fmt.Errorf("part %d of the link from %s came out of order", part, from)
	}
	f, err := os.OpenFile(a.file, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o600)
	if err != nil {
		return holding{}, err
	}
	_, err = f.WriteString(data)
	if err := errors.Join(err, f.Close()); err != nil {
		return holding{}, err
	}
	a.next++
	if part < parts-1 {
		return holding{}, nil
	}
	delete(w.arriving, move)
	defer os.Remove(a.file)

	received := a.file + ".db"
	defer os.Remove(received)
	if err := unpack(a.file, received); err != nil {
		return holding{}, fmt.Errorf("the link from %s didn't arrive whole: %w", from, err)
	}
	if err := hasLink(received); err != nil {
		return holding{}, fmt.Errorf("what came from %s: %w", from, err)
	}
	// The move's id goes in the database before it's a link here, so the
	// answer to _moved is in the same file as the link: a crash between
	// leaves neither or both.
	if err := stamp(received, move); err != nil {
		return holding{}, err
	}
	path := filepath.Join(w.dir, fmt.Sprintf("whatsapp-%d.db", time.Now().UnixNano()))
	if err := os.Rename(received, path); err != nil {
		return holding{}, err
	}
	w.moved[move] = "taken"
	if err := w.saveMoved(); err != nil {
		log.Printf("%v", err) // the link's database says so too
	}
	l, err := open(path, w.node)
	if err != nil {
		return holding{}, err
	}
	w.mu.Lock()
	w.links = append(w.links, l)
	w.mu.Unlock()
	w.inMu.Unlock()
	locked = false
	for start := time.Now(); !loggedIn(l.client) && time.Since(start) < 10*time.Second; {
		time.Sleep(200 * time.Millisecond)
	}
	h := l.holder()
	log.Printf("took the link from %s, as +%s", from, h.Phone)
	return holding{Phone: h.Phone, Connected: h.Connected, All: []holding{h}}, nil
}

// movedHere answers whether the move named came here: once it says not, it
// refuses any part of it still on its way.
func (w *whatsapp) movedHere(move string) (map[string]bool, error) {
	if move == "" {
		return nil, errors.New("which move?")
	}
	w.inMu.Lock()
	defer w.inMu.Unlock()
	if w.moved[move] == "taken" {
		return map[string]bool{"taken": true}, nil
	}
	w.moved[move] = "refused"
	if err := w.saveMoved(); err != nil {
		delete(w.moved, move)
		return nil, err
	}
	if a := w.arriving[move]; a != nil {
		os.Remove(a.file)
		delete(w.arriving, move)
	}
	return map[string]bool{"taken": false}, nil
}

func (w *whatsapp) movedFile() string { return filepath.Join(w.dir, "moves.json") }

// saveMoved keeps each move's answer, with w.inMu held.
func (w *whatsapp) saveMoved() error {
	return writeDurably(w.movedFile(), w.moved, false)
}

func (w *whatsapp) loadMoved() {
	w.moved = map[string]string{}
	if b, err := os.ReadFile(w.movedFile()); err == nil {
		json.Unmarshal(b, &w.moved)
	}
	for _, l := range w.links {
		rows, err := l.db.Query(`SELECT id FROM moved_in`)
		if err != nil {
			continue // a link that never moved
		}
		for rows.Next() {
			var id string
			if rows.Scan(&id) == nil {
				w.moved[id] = "taken"
			}
		}
		rows.Close()
	}
	old, _ := filepath.Glob(filepath.Join(w.dir, "incoming-*"))
	for _, f := range old {
		os.Remove(f)
	}
}

func stamp(path, move string) error {
	db, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		return err
	}
	_, err = db.Exec(`CREATE TABLE IF NOT EXISTS moved_in (id TEXT PRIMARY KEY); INSERT OR IGNORE INTO moved_in (id) VALUES (?)`, move)
	return errors.Join(err, db.Close())
}

func unpack(from, to string) error {
	in, err := os.Open(from)
	if err != nil {
		return err
	}
	defer in.Close()
	z, err := gzip.NewReader(base64.NewDecoder(base64.StdEncoding, in))
	if err != nil {
		return err
	}
	out, err := os.OpenFile(to, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return err
	}
	_, err = io.Copy(out, z)
	return errors.Join(err, out.Close())
}

func hasLink(path string) error {
	db, err := sql.Open("sqlite", "file:"+path+"?mode=ro")
	if err != nil {
		return err
	}
	defer db.Close()
	device, err := sqlstore.NewWithDB(db, "sqlite", logger{"whatsmeow"}).GetFirstDevice(context.Background())
	if err != nil {
		return err
	}
	if device.ID == nil {
		return errors.New("it holds no link")
	}
	return nil
}
