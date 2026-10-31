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
func TestOnlyTheDomainJoinsAndSignsInAgain(t *testing.T) {
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
	// The install has Andre; Sam is in once Sam's first code is made (as if used at once).
	users := []string{"andre-id"}
	asked := 0
	node := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/v1/state":
			list := []map[string]string{}
			for _, u := range users {
				list = append(list, map[string]string{"id": u})
			}
			json.NewEncoder(w).Encode(map[string]any{"install": map[string]any{"users": list}})
		case "/v1/pair":
			asked++
			var body struct {
				NewUser map[string]string `json:"new_user"`
				User    string
			}
			json.NewDecoder(r.Body).Decode(&body)
			switch {
			case body.User != "":
				json.NewEncoder(w).Encode(map[string]string{"code": "devicecode-" + body.User, "user": body.User})
			case body.NewUser["name"] == "Sam Staff" && body.NewUser["role"] == "member":
				users = append(users, "sam-id")
				json.NewEncoder(w).Encode(map[string]string{"code": "paircode", "user": "sam-id"})
			default:
				w.WriteHeader(400)
			}
		}
	}))
	defer node.Close()
	t.Setenv("ONE_CLI", "work")

	dir := t.TempDir()
	j := &joiner{dir: dir, node: node.URL}
	site := httptest.NewServer(j.routes())
	defer site.Close()
	noFollow := &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	if r, _ := http.Get(site.URL + "/"); r.StatusCode != 404 {
		t.Fatalf("off without config: %d", r.StatusCode)
	}
	if r, _ := noFollow.Get(site.URL + "/start?app=1"); !strings.HasPrefix(r.Header.Get("location"), "work://join?error=") {
		t.Fatalf("the app hears it's off: %s", r.Header.Get("location"))
	}
	cfg, _ := json.Marshal(config{Domain: "work.org", ClientID: "client", ClientSecret: "shh", PublicURL: site.URL})
	os.WriteFile(filepath.Join(dir, "config.json"), cfg, 0o600)
	// Andre was in before it was on: an admin wrote him in.
	os.WriteFile(filepath.Join(dir, "joined.json"), []byte(`{"andre@work.org":{"name":"Andre","user":"andre-id","time":1}}`), 0o600)

	// signIn answers the page, or for the app, where it ends.
	signIn := func(as, start string) string {
		email = as
		r, err := noFollow.Get(site.URL + start)
		if err != nil {
			t.Fatal(err)
		}
		to, _ := url.Parse(r.Header.Get("location"))
		if to.Query().Get("hd") != "work.org" {
			t.Fatalf("Google is asked for the domain only: %s", to)
		}
		page, err := noFollow.Get(site.URL + "/callback?code=c&state=" + url.QueryEscape(to.Query().Get("state")))
		if err != nil {
			t.Fatal(err)
		}
		if where := page.Header.Get("location"); where != "" {
			return where
		}
		b := make([]byte, 4096)
		n, _ := page.Body.Read(b)
		return string(b[:n])
	}
	if p := signIn("sam@gmail.com", "/start"); !strings.Contains(p, "only @work.org accounts") || asked != 0 {
		t.Fatalf("another domain: %s", p)
	}
	verified = "false"
	if p := signIn("sam@work.org", "/start?app=1"); !strings.HasPrefix(p, "work://join?error=") || asked != 0 {
		t.Fatalf("unverified, from the app: %s", p)
	}
	verified = "true"
	if p := signIn("sam@work.org", "/start"); !strings.Contains(p, "paircode") || !strings.Contains(p, "Welcome, Sam Staff") {
		t.Fatalf("the domain joins: %s", p)
	}
	if p := signIn("SAM@work.org", "/start?app=1"); p != "work://join?code=devicecode-sam-id&user=Sam+Staff" {
		t.Fatalf("signing in again adds another of their devices: %s", p)
	}
	if p := signIn("andre@work.org", "/start?app=1"); !strings.HasPrefix(p, "work://join?code=devicecode-andre-id&") {
		t.Fatalf("someone in before it was on signs in as themselves: %s", p)
	}
	if asked != 3 {
		t.Fatalf("asked for %d codes", asked)
	}
}
