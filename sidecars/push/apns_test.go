package main

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"math/big"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestTokenIsSignedWithTheKey(t *testing.T) {
	dir := t.TempDir()
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	der, _ := x509.MarshalPKCS8PrivateKey(key)
	os.WriteFile(filepath.Join(dir, "AuthKey.p8"), pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der}), 0o600)
	s := &sender{dir: dir}
	if _, _, err := s.load(); err == nil || !strings.Contains(err.Error(), "isn't set up") {
		t.Fatalf("without config.json: %v", err)
	}
	os.WriteFile(filepath.Join(dir, "config.json"), []byte(`{"team_id":"TEAM123456","key_id":"KEY1234567","topic":"app.one.phone"}`), 0o600)
	c, k, err := s.load()
	if err != nil {
		t.Fatal(err)
	}
	tok, err := s.jwt(c, k)
	if err != nil {
		t.Fatal(err)
	}
	parts := strings.Split(tok, ".")
	var header map[string]string
	h, _ := base64.RawURLEncoding.DecodeString(parts[0])
	json.Unmarshal(h, &header)
	if header["kid"] != "KEY1234567" || header["alg"] != "ES256" {
		t.Fatalf("header %v", header)
	}
	sig, _ := base64.RawURLEncoding.DecodeString(parts[2])
	sum := sha256.Sum256([]byte(parts[0] + "." + parts[1]))
	if len(sig) != 64 || !ecdsa.Verify(&key.PublicKey, sum[:], new(big.Int).SetBytes(sig[:32]), new(big.Int).SetBytes(sig[32:])) {
		t.Fatal("the token's signature doesn't verify")
	}
	if again, _ := s.jwt(c, k); again != tok {
		t.Fatal("a token is kept, not made for every push")
	}
}

func TestPushesGoToTheCallersPhonesAndOpenTheSendingApp(t *testing.T) {
	// The node, saying what the install's push setting is.
	push := "push"
	node := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]any{"install": map[string]any{"settings": map[string]string{"push": push}}})
	}))
	defer node.Close()
	s := &sender{dir: t.TempDir(), node: node.URL, client: node.Client()}
	as := func(user, op, input, from string) (any, error) {
		c := &call{Op: op, Input: json.RawMessage(input), From: from}
		c.Caller.User = user
		return s.run(t.Context(), c)
	}
	if _, err := as("ann", "register", `{"token":"not hex"}`, ""); err == nil {
		t.Fatal("a token that isn't hex is refused")
	}
	if _, err := as("ann", "register", `{"token":"AB12CD34EF56AB12CD34","env":"sandbox"}`, ""); err != nil {
		t.Fatal(err)
	}
	if mine := s.mine("ann"); len(mine) != 1 || mine[0].Token != "ab12cd34ef56ab12cd34" || mine[0].Env != "sandbox" {
		t.Fatalf("ann's phones: %v", mine)
	}
	out, err := as("bob", "send", `{"title":"Hi"}`, "gmail")
	if err != nil || out.(map[string]any)["devices"] != 0 {
		t.Fatalf("bob has no phone: %v %v", out, err)
	}
	if _, err := as("ann", "send", `{"title":"Hi"}`, "gmail"); err == nil || !strings.Contains(err.Error(), "isn't set up") {
		t.Fatalf("ann's phone, without a key here: %v", err)
	}
	push = ""
	if out, err := as("ann", "send", `{"title":"Hi"}`, "gmail"); err != nil || out.(map[string]any)["accepted"] != 0 {
		t.Fatalf("push is off: %v %v", out, err)
	}
	if _, err := as("bob", "unregister", `{"token":"ab12cd34ef56ab12cd34"}`, ""); err != nil || len(s.mine("ann")) != 1 {
		t.Fatal("only its owner unregisters a phone")
	}
	got := link("gmail", sendInput{App: "other", Screen: "thread", Params: map[string]any{"id": "1"}})
	if got["app"] != "gmail" || got["screen"] != "thread" {
		t.Fatalf("the sending app is the one it opens: %v", got)
	}
	if got := link("", sendInput{Link: map[string]any{"app": "repos"}}); got["app"] != "repos" || got["screen"] != "main" {
		t.Fatalf("an old link still opens: %v", got)
	}
	if link("", sendInput{}) != nil {
		t.Fatal("an agent's push opens nothing")
	}
}
