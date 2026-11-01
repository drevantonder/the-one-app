package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waAdv"
	"go.mau.fi/whatsmeow/types"
)

// fakeNode stands in for the node and the user's other servers: each
// server's sidecar is a link, called directly.
type fakeNode struct {
	servers map[string]*whatsapp
	url     string
}

func newFakeNode(t *testing.T, names ...string) *fakeNode {
	connect = func(*whatsmeow.Client) error { return nil }
	loggedIn = func(*whatsmeow.Client) bool { return true }
	f := &fakeNode{servers: map[string]*whatsapp{}}
	mux := http.NewServeMux()
	mux.HandleFunc("/{server}/v1/state", func(w http.ResponseWriter, r *http.Request) {
		type server struct {
			Key, Name string
			Online    bool
			Sidecars  []string
		}
		var all []server
		for _, n := range names {
			all = append(all, server{n, n, true, []string{app}})
		}
		here := r.PathValue("server")
		json.NewEncoder(w).Encode(map[string]any{"server": server{Key: here, Name: here}, "install": map[string]any{"servers": all}})
	})
	mux.HandleFunc("/{server}/v1/call", func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			Op, Server string
			Input      json.RawMessage
		}
		json.NewDecoder(r.Body).Decode(&req)
		result, err := f.servers[req.Server].run(context.Background(), req.Op, req.Input)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			json.NewEncoder(w).Encode(map[string]any{"error": err.Error()})
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"ok": result})
	})
	srv := httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	f.url = srv.URL
	for _, n := range names {
		w, err := openAll(t.TempDir(), srv.URL+"/"+n)
		must(t, err)
		f.servers[n] = w
	}
	return f
}

// linkTo stores a made-up linked device in l, as pairing would.
func linkTo(t *testing.T, l *link, number string) {
	device := l.container.NewDevice()
	device.ID = &types.JID{User: number, Device: 7, Server: types.DefaultUserServer}
	device.Account = &waAdv.ADVSignedDeviceIdentity{Details: []byte{1}, AccountSignature: make([]byte, 64), AccountSignatureKey: make([]byte, 32), DeviceSignature: make([]byte, 64)}
	must(t, device.Save(context.Background()))
	must(t, l.fresh())
}

func run(t *testing.T, w *whatsapp, op string, input any) (string, error) {
	t.Helper()
	raw, _ := json.Marshal(input)
	out, err := w.run(context.Background(), op, raw)
	s, _ := out.(string)
	return s, err
}

// link links a new account on w, as pairing it would.
func link2(t *testing.T, w *whatsapp, number string) *link {
	l, err := w.spare()
	must(t, err)
	linkTo(t, l, number)
	return l
}

func TestPairingPhone(t *testing.T) {
	for in, want := range map[string]string{"+61 400 000 000": "61400000000", "+1 (555) 010-0000": "15550100000"} {
		got, err := pairingPhone(in)
		if err != nil || got != want {
			t.Errorf("pairingPhone(%q) = %q, %v; want %q", in, got, err, want)
		}
	}
	for _, in := range []string{"0400 000 000", "61400000000", "+0400000000", "+61", "Andre"} {
		if got, err := pairingPhone(in); err == nil {
			t.Errorf("pairingPhone(%q) = %q, want an error", in, got)
		}
	}
}

func TestSeveralAccounts(t *testing.T) {
	f := newFakeNode(t, "linux", "personal")
	linux := f.servers["linux"]
	first := link2(t, linux, "61400000000")
	if first != linux.links[0] {
		t.Fatal("the first account isn't whatsapp.db")
	}
	second := link2(t, linux, "61411111111")
	if second == first || filepath.Base(second.path) == "whatsapp.db" {
		t.Fatalf("the second account went in %s", second.path)
	}
	link2(t, f.servers["personal"], "61422222222")

	got, err := linux.accounts()
	must(t, err)
	var all []string
	for _, a := range got {
		all = append(all, fmt.Sprint(a["account"], "@", a["server"]))
	}
	if strings.Join(all, " ") != "+61400000000@linux +61411111111@linux +61422222222@personal" {
		t.Fatalf("accounts: %v", all)
	}
	if l, err := linux.pick("+61 411 111 111", false); err != nil || l != second {
		t.Fatalf("pick: %v", err)
	}
	var away elsewhere
	if _, err := linux.pick("+61422222222", false); !errors.As(err, &away) {
		t.Fatalf("an account on another server: %v, want elsewhere", err)
	}
	if _, err := run(t, linux, "sign_out", nil); err == nil || !strings.Contains(err.Error(), "give account") {
		t.Fatalf("sign_out of one of several, unnamed: %v", err)
	}

	// The next start opens both, and forgets a pairing that never finished.
	unfinished, err := linux.spare()
	must(t, err)
	for _, l := range linux.links {
		l.unload()
	}
	again, err := openAll(linux.dir, linux.node)
	must(t, err)
	if len(again.links) != 2 || again.links[1].holder().Phone != "61411111111" {
		t.Fatalf("opened %d links", len(again.links))
	}
	if _, err := os.Stat(unfinished.path); err == nil {
		t.Fatal("kept an unfinished pairing")
	}
}

func TestUnlinkedSendsCallsElsewhere(t *testing.T) {
	f := newFakeNode(t, "personal")
	for _, op := range []string{"chats", "read", "search", "send", "move", "sign_out"} {
		var away elsewhere
		if _, err := run(t, f.servers["personal"], op, map[string]any{"chat": "x", "query": "x", "text": "x", "to": "x"}); !errors.As(err, &away) {
			t.Errorf("%s while unlinked: %v, want elsewhere", op, err)
		}
	}
}

func TestMoveHandsOverTheLinkAndMessages(t *testing.T) {
	partSize = 1000 // several parts
	t.Cleanup(func() { partSize = 1 << 20 })
	f := newFakeNode(t, "linux", "personal")
	linux, personal := f.servers["linux"], f.servers["personal"]
	from, to := linux.links[0], personal.links[0]
	linkTo(t, from, "61400000000")
	must(t, put(from.db, stored{Chat: "1@s.whatsapp.net", ID: "a", Sender: "1@s.whatsapp.net", Time: time.Unix(1e9, 0), Text: "kept"}, false))

	out, err := run(t, linux, "move", map[string]any{"to": "personal"})
	if err != nil || !strings.HasPrefix(out, "Moved WhatsApp from linux to personal, as +61400000000") {
		t.Fatalf("move: %q, %v", out, err)
	}
	if h := to.holder(); h.Phone != "61400000000" {
		t.Fatalf("personal holds %+v", h)
	}
	if ms, _ := messages(to.db, "", "", 10, 0); len(ms) != 1 || ms[0].Text != "kept" {
		t.Fatalf("personal kept %v", texts(ms))
	}
	if h := from.holder(); h.Phone != "" {
		t.Fatalf("linux still holds %+v", h)
	}
	if ms, _ := messages(from.db, "", "", 10, 0); len(ms) != 0 {
		t.Fatalf("linux still keeps %v", texts(ms))
	}
	if left, _ := filepath.Glob(filepath.Join(filepath.Dir(from.path), "moving-*")); len(left) > 0 {
		t.Fatalf("left behind %v", left)
	}

	// Back again, and not onto a server where that number is linked already.
	if _, err := run(t, personal, "move", map[string]any{"to": "linux"}); err != nil {
		t.Fatal(err)
	}
	link2(t, personal, "61400000000")
	if _, err := run(t, linux, "move", map[string]any{"to": "personal"}); err == nil || !strings.Contains(err.Error(), "personal is linked to WhatsApp already, as +61400000000") {
		t.Fatalf("move onto a server linked as that number: %v", err)
	}
	if h := from.holder(); h.Phone != "61400000000" {
		t.Fatalf("a refused move unlinked linux: %+v", h)
	}

	// Another account moves beside one linked there, and its database goes from here.
	second := link2(t, linux, "61411111111")
	if _, err := run(t, linux, "move", map[string]any{"to": "personal", "account": "+61411111111"}); err != nil {
		t.Fatal(err)
	}
	if h := personal.holding(); len(h.All) != 2 || h.All[1].Phone != "61411111111" {
		t.Fatalf("personal holds %+v", h)
	}
	if len(linux.links) != 1 {
		t.Fatalf("linux kept %d links", len(linux.links))
	}
	if _, err := os.Stat(second.path); err == nil {
		t.Fatal("the moved account's database is still on linux")
	}
}

func TestTakeRefusesWhatHoldsNoLink(t *testing.T) {
	f := newFakeNode(t, "personal")
	l := f.servers["personal"].links[0]
	session, err := snapshot(l.db, t.TempDir())
	must(t, err)
	if _, err := l.take("linux", 0, 1, encode(session)); err == nil || !strings.Contains(err.Error(), "holds no link") {
		t.Fatalf("take: %v", err)
	}
	if _, err := l.take("linux", 1, 2, "x"); err == nil {
		t.Fatal("took a part before the first")
	}
}

func encode(b []byte) string { return base64.StdEncoding.EncodeToString(b) }
