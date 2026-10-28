// Package store is the node's append-only log. Every device appends to its own
// log; entries from all devices sort the same way everywhere, by hybrid
// logical clock, then device, then sequence number.
package store

import (
	"crypto/rand"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"sync"
	"time"

	_ "modernc.org/sqlite"

	"github.com/drevantonder/the-one-app/protocol"
)

const schema = `
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS entries (
	device  TEXT    NOT NULL,
	seq     INTEGER NOT NULL,
	hlc     INTEGER NOT NULL,
	plugin  TEXT    NOT NULL,
	type    TEXT    NOT NULL,
	payload BLOB    NOT NULL,
	actor   TEXT    NOT NULL DEFAULT '',
	PRIMARY KEY (device, seq)
);
CREATE INDEX IF NOT EXISTS entries_order ON entries (plugin, hlc, device, seq);
`

type Store struct {
	db     *sql.DB
	device string

	mu      sync.Mutex
	seq     int64
	lastHLC uint64
	changed chan struct{} // closed and replaced whenever the log grows
}

func Open(path string) (*Store, error) {
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)&_pragma=synchronous(NORMAL)")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return nil, err
	}
	s := &Store{db: db, changed: make(chan struct{})}
	if err := s.load(); err != nil {
		db.Close()
		return nil, err
	}
	return s, nil
}

func (s *Store) load() error {
	err := s.db.QueryRow(`SELECT value FROM meta WHERE key = 'device'`).Scan(&s.device)
	if err == sql.ErrNoRows {
		b := make([]byte, 6)
		if _, err := rand.Read(b); err != nil {
			return err
		}
		s.device = hex.EncodeToString(b)
		if _, err := s.db.Exec(`INSERT INTO meta (key, value) VALUES ('device', ?)`, s.device); err != nil {
			return err
		}
	} else if err != nil {
		return err
	}
	if err := s.db.QueryRow(`SELECT COALESCE(MAX(seq), 0) FROM entries WHERE device = ?`, s.device).Scan(&s.seq); err != nil {
		return err
	}
	var last int64
	if err := s.db.QueryRow(`SELECT COALESCE(MAX(hlc), 0) FROM entries`).Scan(&last); err != nil {
		return err
	}
	s.lastHLC = uint64(last)
	return nil
}

func (s *Store) Device() string { return s.device }
func (s *Store) Close() error   { return s.db.Close() }

// tick advances the hybrid logical clock: wall-clock milliseconds in the high
// bits, a counter in the low 16 bits, never going backwards.
func (s *Store) tick() uint64 {
	now := uint64(time.Now().UnixMilli()) << 16
	if now > s.lastHLC {
		s.lastHLC = now
	} else {
		s.lastHLC++
	}
	return s.lastHLC
}

// Append writes a new entry to this device's log.
func (s *Store) Append(plugin, typ string, payload json.RawMessage, actor string) (protocol.Entry, error) {
	if len(payload) == 0 {
		payload = json.RawMessage("null")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	seq, hlc := s.seq+1, s.tick()
	if _, err := s.db.Exec(`INSERT INTO entries (device, seq, hlc, plugin, type, payload, actor) VALUES (?, ?, ?, ?, ?, ?, ?)`,
		s.device, seq, int64(hlc), plugin, typ, []byte(payload), actor); err != nil {
		return protocol.Entry{}, err
	}
	s.seq = seq
	s.notify()
	return entry(s.device, seq, hlc, plugin, typ, payload, actor), nil
}

// notify wakes everyone waiting on Changed. Callers hold s.mu.
func (s *Store) notify() {
	close(s.changed)
	s.changed = make(chan struct{})
}

// Changed returns a channel that is closed the next time the log grows.
func (s *Store) Changed() <-chan struct{} {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.changed
}

// Vector is how far this node has each device's log: the highest sequence
// number it holds per device. Logs are always received in order, so
// everything below that number is present too.
func (s *Store) Vector() (map[string]int64, error) {
	rows, err := s.db.Query(`SELECT device, MAX(seq) FROM entries GROUP BY device`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	v := map[string]int64{}
	for rows.Next() {
		var d string
		var n int64
		if err := rows.Scan(&d, &n); err != nil {
			return nil, err
		}
		v[d] = n
	}
	return v, rows.Err()
}

// After returns entries this node has beyond the given vector, from every
// device (so nodes relay each other's logs), in per-device sequence order.
func (s *Store) After(vector map[string]int64, limit int) ([]protocol.Entry, error) {
	mine, err := s.Vector()
	if err != nil {
		return nil, err
	}
	var out []protocol.Entry
	for device, have := range mine {
		from := vector[device]
		if have <= from {
			continue
		}
		batch, err := s.DeviceEntries(device, from, limit-len(out))
		if err != nil {
			return nil, err
		}
		out = append(out, batch...)
		if len(out) >= limit {
			break
		}
	}
	return out, nil
}

// DeviceEntries returns one device's entries after a sequence number.
func (s *Store) DeviceEntries(device string, after int64, limit int) ([]protocol.Entry, error) {
	rows, err := s.db.Query(`SELECT seq, hlc, plugin, type, payload, actor FROM entries WHERE device = ? AND seq > ? ORDER BY seq LIMIT ?`, device, after, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []protocol.Entry
	for rows.Next() {
		var (
			plugin, typ, actor string
			seq, hlc           int64
			payload            []byte
		)
		if err := rows.Scan(&seq, &hlc, &plugin, &typ, &payload, &actor); err != nil {
			return nil, err
		}
		out = append(out, entry(device, seq, uint64(hlc), plugin, typ, payload, actor))
	}
	return out, rows.Err()
}

// Ingest stores entries received from another node or the bucket. Entries
// already held are skipped; an entry that would leave a gap in its device's
// log is skipped too (it arrives again later, in order). It returns the
// entries that were new.
func (s *Store) Ingest(entries []protocol.Entry) ([]protocol.Entry, error) {
	if len(entries) == 0 {
		return nil, nil
	}
	entries = slices.Clone(entries)
	slices.SortStableFunc(entries, func(a, b protocol.Entry) int {
		if c := strings.Compare(a.Device, b.Device); c != 0 {
			return c
		}
		return int(a.Seq - b.Seq)
	})
	vec, err := s.Vector()
	if err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	tx, err := s.db.Begin()
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	var added []protocol.Entry
	for _, e := range entries {
		if e.Device == s.device || e.Seq != vec[e.Device]+1 {
			continue
		}
		payload := []byte(e.Payload)
		if len(payload) == 0 {
			payload = []byte("null")
		}
		if _, err := tx.Exec(`INSERT INTO entries (device, seq, hlc, plugin, type, payload, actor) VALUES (?, ?, ?, ?, ?, ?, ?)`,
			e.Device, e.Seq, int64(e.HLC), e.Plugin, e.Type, payload, e.Actor); err != nil {
			return nil, err
		}
		vec[e.Device] = e.Seq
		if e.HLC > s.lastHLC {
			s.lastHLC = e.HLC // our next entry sorts after everything we've seen
		}
		added = append(added, entry(e.Device, e.Seq, e.HLC, e.Plugin, e.Type, payload, e.Actor))
	}
	if err := tx.Commit(); err != nil {
		return nil, err
	}
	if len(added) > 0 {
		s.notify()
	}
	return added, nil
}

// Entries returns a plugin's entries in log order.
func (s *Store) Entries(plugin string) ([]protocol.Entry, error) {
	rows, err := s.db.Query(`SELECT device, seq, hlc, type, payload, actor FROM entries WHERE plugin = ? ORDER BY hlc, device, seq`, plugin)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []protocol.Entry
	for rows.Next() {
		var (
			device, typ, actor string
			seq, hlc           int64
			payload            []byte
		)
		if err := rows.Scan(&device, &seq, &hlc, &typ, &payload, &actor); err != nil {
			return nil, err
		}
		out = append(out, entry(device, seq, uint64(hlc), plugin, typ, payload, actor))
	}
	return out, rows.Err()
}

// Meta reads a device-local setting.
func (s *Store) Meta(key string) (string, error) {
	var v string
	err := s.db.QueryRow(`SELECT value FROM meta WHERE key = ?`, key).Scan(&v)
	if err == sql.ErrNoRows {
		return "", nil
	}
	return v, err
}

// SetMeta writes a device-local setting (never synced).
func (s *Store) SetMeta(key, value string) error {
	_, err := s.db.Exec(`INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value`, key, value)
	return err
}

func entry(device string, seq int64, hlc uint64, plugin, typ string, payload []byte, actor string) protocol.Entry {
	return protocol.Entry{
		ID:      fmt.Sprintf("%s:%d", device, seq),
		Device:  device,
		Seq:     seq,
		HLC:     hlc,
		Plugin:  plugin,
		Type:    typ,
		Payload: json.RawMessage(payload),
		Actor:   actor,
		Time:    time.UnixMilli(int64(hlc >> 16)).UTC().Format(time.RFC3339),
	}
}
