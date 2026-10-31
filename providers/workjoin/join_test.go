package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A fake Google that signs in whoever `email` says, and a fake node that makes pairing codes.
func TestOnlyTheDomainJoinsOnceEach(t *testing.T) {
	email, verified := "", "true"
	google := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/token":
			r.ParseForm()
			if r.Form.Get("code_verifier") == "" || r.Form.Get("client_secret") != "shh" {
				w.WriteHeader(400)
				return
			}
			json.NewEncoder(w).Encode(map[string]string{"id_token": "tok"})
		case "/tokeninfo":
			hd := strings.SplitN(email, "@", 2)[1]
			json.NewEncoder(w).Encode(map[string]string{"aud": "client", "email": email, "hd": hd, "name": "Sam Staff", "email_verified": verified})
		}
	}))
	defer google.Close()
	tokenURL, tokeninfoURL = google.URL+"/token", google.URL+"/tokeninfo"
	asked := 0
	node := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/v1/pair" {
			asked++
			var body map[string]map[string]string
			json.NewDecoder(r.Body).Decode(&body)
			if body["new_user"]["name"] != "Sam Staff" || body["new_user"]["role"] != "member" {
				w.WriteHeader(400)
				return
			}
			json.NewEncoder(w).Encode(map[string]string{"code": "paircode"})
		}
	}))
	defer node.Close()

	dir := t.TempDir()
	j := &joiner{dir: dir, node: node.URL}
	site := httptest.NewServer(j.routes())
	defer site.Close()
	if r, _ := http.Get(site.URL + "/"); r.StatusCode != 404 {
		t.Fatalf("off without config: %d", r.StatusCode)
	}
	cfg, _ := json.Marshal(config{Domain: "work.org", ClientID: "client", ClientSecret: "shh", PublicURL: site.URL})
	os.WriteFile(filepath.Join(dir, "config.json"), cfg, 0o600)

	signIn := func(as string) string {
		email = as
		noFollow := &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
		r, err := noFollow.Get(site.URL + "/start")
		if err != nil {
			t.Fatal(err)
		}
		to, _ := url.Parse(r.Header.Get("location"))
		if to.Query().Get("hd") != "work.org" {
			t.Fatalf("Google is asked for the domain only: %s", to)
		}
		page, err := http.Get(site.URL + "/callback?code=c&state=" + url.QueryEscape(to.Query().Get("state")))
		if err != nil {
			t.Fatal(err)
		}
		b := make([]byte, 4096)
		n, _ := page.Body.Read(b)
		return string(b[:n])
	}
	if p := signIn("sam@gmail.com"); !strings.Contains(p, "only @work.org accounts") || asked != 0 {
		t.Fatalf("another domain: %s", p)
	}
	verified = "false"
	if p := signIn("sam@work.org"); !strings.Contains(p, "verified") || asked != 0 {
		t.Fatalf("unverified: %s", p)
	}
	verified = "true"
	if p := signIn("sam@work.org"); !strings.Contains(p, "paircode") || !strings.Contains(p, "Welcome, Sam Staff") {
		t.Fatalf("the domain joins: %s", p)
	}
	if p := signIn("SAM@work.org"); !strings.Contains(p, "already joined") || asked != 1 {
		t.Fatalf("once each: %s", p)
	}
}
