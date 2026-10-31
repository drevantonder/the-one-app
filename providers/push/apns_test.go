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
