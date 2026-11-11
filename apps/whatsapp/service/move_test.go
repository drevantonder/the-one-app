package main

// Moving a link between two devices, each with a an app runtime of its own that passes
// calls to the other's service, as the real app runtimes do; WhatsApp is never
// reached. A link must end on exactly one device, whatever answer is lost.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"sync"
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waAdv"
	"go.mau.fi/whatsmeow/types"
)

type fakeRuntimes struct {
	mu       sync.Mutex
	services map[string]*whatsapp
	// lose is asked before and after each call: true loses it there (before:
	// it never runs; after: it runs, but its answer is lost).
	lose func(function string, in map[string]any, after bool) bool
}

func (f *fakeRuntimes) runtime(t *testing.T, name string) string {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/api/state":
			json.NewEncoder(w).Encode(map[string]any{"device": map[string]any{"key": name, "name": name},
				"network": map[string]any{"devices": []map[string]any{
					{"key": "a", "name": "a", "online": true, "services": []string{app}},
					{"key": "b", "name": "b", "online": true, "services": []string{app}},
				}}})
		case "/api/call":
			var c struct {
				Function string `json:"function"`
				Device   string
				Input    map[string]any
			}
			json.NewDecoder(r.Body).Decode(&c)
			f.mu.Lock()
			s, lose := f.services[c.Device], f.lose
			f.mu.Unlock()
			if s == nil || lose != nil && lose(c.Function, c.Input, false) {
				http.Error(w, `{"error":"unreachable"}`, http.StatusBadGateway)
				return
			}
			raw, _ := json.Marshal(c.Input)
			out, err := s.run(context.Background(), c.Function, raw)
			if lose != nil && lose(c.Function, c.Input, true) {
				http.Error(w, `{"error":"answer lost"}`, http.StatusBadGateway)
				return
			}
			if err != nil {
				w.WriteHeader(http.StatusInternalServerError)
				json.NewEncoder(w).Encode(map[string]any{"error": err.Error()})
				return
			}
			json.NewEncoder(w).Encode(map[string]any{"ok": out})
		}
	}))
	t.Cleanup(srv.Close)
	return srv.URL
}

var connects = map[string]int{} // by number, how often a link connected

func offline(t *testing.T) {
	var mu sync.Mutex
	connect = func(c *whatsmeow.Client) error {
		if c.Store.ID != nil {
			mu.Lock()
			connects[c.Store.ID.User]++
			mu.Unlock()
		}
		return nil
	}
	loggedIn = func(c *whatsmeow.Client) bool { return c.Store.ID != nil }
	partSize = 1 << 10 // several parts
	t.Cleanup(func() { clear(connects) })
}

func linkAs(t *testing.T, l *link, number string) {
	l.client.Store.ID = &types.JID{User: number, Device: 7, Server: types.DefaultUserServer}
	l.client.Store.Account = &waAdv.ADVSignedDeviceIdentity{Details: []byte{1}, AccountSignature: make([]byte, 64), AccountSignatureKey: make([]byte, 32), DeviceSignature: make([]byte, 64)}
	if err := l.client.Store.Save(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err := put(l.db, stored{Chat: "1@s.whatsapp.net", ID: "m1", Sender: "1@s.whatsapp.net", Time: time.Now(), Text: "hello from " + number}, false); err != nil {
		t.Fatal(err)
	}
}

func devicesAB(t *testing.T, f *fakeRuntimes) (a, b *whatsapp, dirA string) {
	dirA, dirB := t.TempDir(), t.TempDir()
	var err error
	if a, err = openAll(dirA, f.runtime(t, "a")); err != nil {
		t.Fatal(err)
	}
	if b, err = openAll(dirB, f.runtime(t, "b")); err != nil {
		t.Fatal(err)
	}
	f.services = map[string]*whatsapp{"a": a, "b": b}
	return a, b, dirA
}

func holds(w *whatsapp, number string) bool { return w.holding().has(number) }

func kept(t *testing.T, w *whatsapp, number string) int {
	for _, l := range w.linked() {
		if l.holder().Phone == number {
			var n int
			l.db.QueryRow(`SELECT count(*) FROM messages`).Scan(&n)
			return n
		}
	}
	return -1
}

func TestMoveLeavesOneCopy(t *testing.T) {
	offline(t)
	f := &fakeRuntimes{}
	a, b, _ := devicesAB(t, f)
	linkAs(t, a.links[0], "61400000001")
	if _, err := a.run(context.Background(), "move", json.RawMessage(`{"to":"b"}`)); err != nil {
		t.Fatal(err)
	}
	if holds(a, "61400000001") || !holds(b, "61400000001") || kept(t, b, "61400000001") != 1 {
		t.Fatalf("after a move: a %v, b %v", a.holding(), b.holding())
	}
}

// The target took it, but its answer, and then the first asking after it, are
// lost, and the source restarts: it stays off until it hears, then deletes.
func TestMoveAnswerLostThenRestart(t *testing.T) {
	offline(t)
	f := &fakeRuntimes{}
	a, b, dirA := devicesAB(t, f)
	linkAs(t, a.links[0], "61400000002")
	lost := 0
	f.lose = func(function string, in map[string]any, after bool) bool {
		switch {
		case function == "_take" && after && in["part"] == in["parts"].(float64)-1:
			return true
		case function == "_moved" && !after && lost < 1:
			lost++
			return true
		}
		return false
	}
	if _, err := a.run(context.Background(), "move", json.RawMessage(`{"to":"b"}`)); err == nil {
		t.Fatal("a move whose answer was lost said it moved")
	}
	if !holds(b, "61400000002") || a.links[0].holder().State != handingOff {
		t.Fatalf("a %v, b %v", a.holding(), b.holding())
	}
	before := connects["61400000002"]
	// A restart: the state on disk keeps it off.
	a2, err := openAll(dirA, a.runtime)
	if err != nil {
		t.Fatal(err)
	}
	f.services["a"] = a2
	if connects["61400000002"] != before {
		t.Fatal("a link handing off connected after a restart")
	}
	// It asks again at once on starting, and then every minute.
	for start := time.Now(); holds(a2, "61400000002") && time.Since(start) < 5*time.Second; {
		time.Sleep(50 * time.Millisecond)
	}
	if holds(a2, "61400000002") || !holds(b, "61400000002") {
		t.Fatalf("a %v, b %v", a2.holding(), b.holding())
	}
	if _, err := os.Stat(a2.links[0].path + ".state"); err == nil {
		t.Fatal("a's state is left after it handed off")
	}
}

// The target never got it: asked, it refuses it for good, and the source is
// linked again; a part still on its way is refused.
func TestMoveRefusedRollsBack(t *testing.T) {
	offline(t)
	f := &fakeRuntimes{}
	a, b, _ := devicesAB(t, f)
	linkAs(t, a.links[0], "61400000003")
	var move string
	f.lose = func(function string, in map[string]any, after bool) bool {
		if function == "_take" && !after && in["part"] == float64(1) {
			move = in["move"].(string)
			return true
		}
		return false
	}
	if _, err := a.run(context.Background(), "move", json.RawMessage(`{"to":"b"}`)); err == nil {
		t.Fatal("a move that failed said it moved")
	}
	if h := a.links[0].holder(); h.Phone != "61400000003" || h.State != "" || holds(b, "61400000003") {
		t.Fatalf("a %v, b %v", a.holding(), b.holding())
	}
	if _, err := b.take("a", move, 1, 2, "late"); err == nil {
		t.Fatal("a part of a refused move was taken")
	}
}

// Two devices moving links to one at once: each arrives whole, as its own.
func TestTwoMovesToOne(t *testing.T) {
	offline(t)
	f := &fakeRuntimes{}
	a, b, _ := devicesAB(t, f)
	linkAs(t, a.links[0], "61400000004")
	l2, err := a.spare()
	if err != nil {
		t.Fatal(err)
	}
	linkAs(t, l2, "61400000005")
	var wg sync.WaitGroup
	for _, number := range []string{"61400000004", "61400000005"} {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, err := a.run(context.Background(), "move", json.RawMessage(`{"to":"b","account":"`+number+`"}`)); err != nil {
				t.Error(err)
			}
		}()
	}
	wg.Wait()
	for _, number := range []string{"61400000004", "61400000005"} {
		if holds(a, number) || kept(t, b, number) != 1 {
			t.Fatalf("%s: a %v, b %v", number, a.holding(), b.holding())
		}
	}
}

// The phone unlinking it deletes what it kept, so the next link starts empty.
func TestUnlinkedFromThePhoneWipes(t *testing.T) {
	offline(t)
	f := &fakeRuntimes{}
	a, _, _ := devicesAB(t, f)
	l := a.links[0]
	linkAs(t, l, "61400000006")
	l.unlinked(l.client)
	var n int
	l.db.QueryRow(`SELECT count(*) FROM messages`).Scan(&n)
	if n != 0 || l.holder().Phone != "" || loadState(l.path).State != unlinked {
		t.Fatalf("after unlinking: %d messages, %v, %v", n, l.holder(), loadState(l.path))
	}
}
