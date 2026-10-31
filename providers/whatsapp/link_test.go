package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waAdv"
	"go.mau.fi/whatsmeow/types"
)

// fakeNode stands in for the node and the user's other servers: each
// server's provider is a link, called directly.
type fakeNode struct {
	links map[string]*link
	url   string
}

func newFakeNode(t *testing.T, names ...string) *fakeNode {
	connect = func(*whatsmeow.Client) error { return nil }
	loggedIn = func(*whatsmeow.Client) bool { return true }
	f := &fakeNode{links: map[string]*link{}}
	mux := http.NewServeMux()
	mux.HandleFunc("/{server}/v1/state", func(w http.ResponseWriter, r *http.Request) {
		type server struct {
			Key, Name string
			Online    bool
			Drivers   []string
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
		result, err := f.links[req.Server].run(context.Background(), req.Op, req.Input)
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
		l, err := open(filepath.Join(t.TempDir(), "whatsapp.db"), srv.URL+"/"+n)
		must(t, err)
		f.links[n] = l
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

func run(t *testing.T, l *link, op string, input any) (string, error) {
	t.Helper()
	raw, _ := json.Marshal(input)
	out, err := l.run(context.Background(), op, raw)
	s, _ := out.(string)
	return s, err
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

func TestSignInWhileLinkedSaysWhere(t *testing.T) {
	f := newFakeNode(t, "linux", "personal")
	linkTo(t, f.links["linux"], "61400000000")
	out, err := run(t, f.links["linux"], "sign_in", map[string]any{"phone": "+61 400 000 000"})
	if err != nil || out != "Already linked here, on linux, as +61400000000." {
		t.Fatalf("sign_in here: %q, %v", out, err)
	}
	_, err = run(t, f.links["personal"], "sign_in", map[string]any{})
	if err == nil || !strings.Contains(err.Error(), "WhatsApp is linked on linux, as +61400000000") {
		t.Fatalf("sign_in elsewhere: %v", err)
	}
	out, _ = run(t, f.links["personal"], "status", nil)
	if !strings.Contains(out, "Not linked here, on personal: WhatsApp is linked on linux, as +61400000000.") {
		t.Fatalf("status elsewhere: %q", out)
	}
}

func TestUnlinkedSendsCallsElsewhere(t *testing.T) {
	f := newFakeNode(t, "personal")
	for _, op := range []string{"chats", "read", "search", "send", "move", "sign_out"} {
		var away elsewhere
		if _, err := run(t, f.links["personal"], op, map[string]any{"chat": "x", "query": "x", "text": "x", "to": "x"}); !errors.As(err, &away) {
			t.Errorf("%s while unlinked: %v, want elsewhere", op, err)
		}
	}
}

func TestMoveHandsOverTheLinkAndMessages(t *testing.T) {
	partSize = 1000 // several parts
	t.Cleanup(func() { partSize = 1 << 20 })
	f := newFakeNode(t, "linux", "personal")
	from, to := f.links["linux"], f.links["personal"]
	linkTo(t, from, "61400000000")
	must(t, put(from.db, stored{Chat: "1@s.whatsapp.net", ID: "a", Sender: "1@s.whatsapp.net", Time: time.Unix(1e9, 0), Text: "kept"}, false))

	out, err := run(t, from, "move", map[string]any{"to": "personal"})
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

	// Back again, and not onto a server that's linked.
	if _, err := run(t, to, "move", map[string]any{"to": "linux"}); err != nil {
		t.Fatal(err)
	}
	linkTo(t, to, "61411111111")
	if _, err := run(t, to, "move", map[string]any{"to": "linux"}); err == nil || !strings.Contains(err.Error(), "linux is linked to WhatsApp already") {
		t.Fatalf("move onto a linked server: %v", err)
	}
	if h := to.holder(); h.Phone != "61411111111" {
		t.Fatalf("a refused move unlinked personal: %+v", h)
	}
}

func TestTakeRefusesWhatHoldsNoLink(t *testing.T) {
	f := newFakeNode(t, "personal")
	l := f.links["personal"]
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
