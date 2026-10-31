package main

// Moving the link: this provider on the server that holds it hands its
// database (the link's keys and the messages kept) to the same provider on
// another of the user's servers, in calls through the nodes, end-to-end
// encrypted between them. Nothing is written anywhere else, so once the
// other server has it and this one deletes its copy, there is one copy.

import (
	"bytes"
	"compress/gzip"
	"context"
	"database/sql"
	"encoding/base64"
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

// holding is what one server's provider tells another about its links: the
// first, as when there could be only one, and all of them.
type holding struct {
	Phone     string    `json:"phone,omitempty"` // the linked number, if linked there
	Connected bool      `json:"connected,omitempty"`
	All       []holding `json:"all,omitempty"`
}

// all is every link a server holds, from a server before several too.
func (h holding) all() []holding {
	if h.All != nil {
		return h.All
	}
	if h.Phone != "" {
		return []holding{{Phone: h.Phone, Connected: h.Connected}}
	}
	return nil
}

// has says whether a server holds a link as number.
func (h holding) has(number string) bool {
	return slices.ContainsFunc(h.all(), func(o holding) bool { return o.Phone == number })
}

func (l *link) holder() holding {
	l.mu.Lock()
	defer l.mu.Unlock()
	if id := l.client.Store.ID; id != nil {
		return holding{Phone: id.User, Connected: loggedIn(l.client)}
	}
	return holding{}
}

// servers is this server's name, and the user's other servers that run
// WhatsApp and are online.
type servers struct {
	here   string
	others []string
}

func (l *link) servers() (servers, error) {
	var state struct {
		Server  struct{ Key, Name string }
		Install struct {
			Servers []struct {
				Key, Name string
				Online    bool
				Drivers   []string
			}
		}
	}
	body, err := request(&http.Client{Timeout: 5 * time.Second}, l.node+"/v1/state", nil)
	if err == nil {
		err = json.Unmarshal(body, &state)
	}
	if err != nil {
		return servers{}, fmt.Errorf("asking this server's node: %w", err)
	}
	s := servers{here: state.Server.Name}
	for _, o := range state.Install.Servers {
		if o.Key != state.Server.Key && o.Online && slices.Contains(o.Drivers, app) {
			s.others = append(s.others, o.Name)
		}
	}
	return s, nil
}

// linkedElsewhere is the other server that holds the link, and as which
// number; "" if none of those that answered does.
func (l *link) linkedElsewhere(s servers) (server, phone string) {
	for _, name := range s.others {
		var h holding
		if err := callOn(l.node, name, "_holder", map[string]any{}, &h); err == nil && h.Phone != "" {
			return name, h.Phone
		}
	}
	return "", ""
}

func (l *link) move(to string) (string, error) {
	to = strings.TrimSpace(to)
	if to == "" {
		return "", errors.New("give to: the server to move WhatsApp to")
	}
	s, err := l.servers()
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

	l.mu.Lock()
	defer l.mu.Unlock()
	if l.client.Store.ID == nil {
		return "", errors.New("WhatsApp isn't linked here")
	}
	number := l.client.Store.ID.User
	l.client.Disconnect() // so the link is never connected from two servers
	session, err := snapshot(l.db, filepath.Dir(l.path))
	if err != nil {
		l.reconnect()
		return "", fmt.Errorf("not moved: %w", err)
	}
	data := base64.StdEncoding.EncodeToString(session)
	parts := (len(data) + partSize - 1) / partSize
	for i := 0; i < parts && err == nil; i++ {
		part := data[i*partSize : min((i+1)*partSize, len(data))]
		err = callOn(l.node, to, "_take", map[string]any{"from": s.here, "part": i, "parts": parts, "data": part}, &there)
	}
	if err != nil {
		// Its answer may be what was lost: ask before connecting here again.
		if callOn(l.node, to, "_holder", map[string]any{}, &there) != nil || !there.has(number) {
			l.reconnect()
			return "", fmt.Errorf("not moved, still linked here: %w", err)
		}
	}
	if err := l.forget(); err != nil {
		return "", fmt.Errorf("moved to %s, but deleting it here failed: %w", to, err)
	}
	state := "connected"
	if !there.Connected {
		state = "connecting"
	}
	return fmt.Sprintf("Moved WhatsApp from %s to %s, as +%s, with its messages; it's %s there and gone from %s. Messages sent meanwhile arrive there.",
		s.here, to, number, state, s.here), nil
}

func (l *link) reconnect() {
	if err := connect(l.client); err != nil {
		log.Printf("connecting again: %v", err)
	}
}

// snapshot is the whole database, consistent and compressed.
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

// take receives the link another server moves here, part by part, and
// links this server with it once it has every part.
func (l *link) take(from string, part, parts int, data string) (holding, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if id := l.client.Store.ID; id != nil {
		return holding{}, fmt.Errorf("WhatsApp is linked here already, as +%s", id.User)
	}
	incoming := l.path + ".incoming"
	flags := os.O_WRONLY | os.O_APPEND
	if part == 0 {
		flags |= os.O_CREATE | os.O_TRUNC
	}
	f, err := os.OpenFile(incoming, flags, 0o600)
	if err != nil {
		return holding{}, fmt.Errorf("part %d of the link from %s came before the first", part, from)
	}
	_, err = f.WriteString(data)
	if err := errors.Join(err, f.Close()); err != nil || part < parts-1 {
		return holding{}, err
	}
	defer os.Remove(incoming)

	received := l.path + ".received"
	defer os.Remove(received)
	if err := unpack(incoming, received); err != nil {
		return holding{}, fmt.Errorf("the link from %s didn't arrive whole: %w", from, err)
	}
	if err := hasLink(received); err != nil {
		return holding{}, fmt.Errorf("what came from %s: %w", from, err)
	}
	l.unload()
	for _, suffix := range []string{"-wal", "-shm"} {
		os.Remove(l.path + suffix)
	}
	err = os.Rename(received, l.path)
	l.qr = ""
	if err := errors.Join(err, l.load()); err != nil {
		return holding{}, err
	}
	if err := connect(l.client); err != nil {
		log.Printf("connecting: %v", err)
	}
	for start := time.Now(); !loggedIn(l.client) && time.Since(start) < 10*time.Second; {
		time.Sleep(200 * time.Millisecond)
	}
	log.Printf("took the link from %s, as +%s", from, l.client.Store.ID.User)
	return holding{Phone: l.client.Store.ID.User, Connected: loggedIn(l.client)}, nil
}

// unpack decodes and decompresses the parts in from into the database to.
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

// hasLink checks that the database at path holds a linked device.
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
