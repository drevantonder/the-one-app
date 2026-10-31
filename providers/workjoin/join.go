package main

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"html/template"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

// Google's endpoints; tests point them elsewhere.
var (
	authURL      = "https://accounts.google.com/o/oauth2/v2/auth"
	tokenURL     = "https://oauth2.googleapis.com/token"
	tokeninfoURL = "https://oauth2.googleapis.com/tokeninfo"
)

type config struct {
	Domain       string `json:"domain"`
	ClientID     string `json:"client_id"`
	ClientSecret string `json:"client_secret"`
	PublicURL    string `json:"public_url"`
	Listen       string `json:"listen"`
	Group        string `json:"group"`
}

type joiner struct {
	dir  string
	node string

	mu      sync.Mutex
	pending map[string]pending // by OAuth state
}

type pending struct {
	verifier string
	made     time.Time
}

// joined is who joined through it: by email, the name and when.
type joined map[string]struct {
	Name string `json:"name"`
	Time int64  `json:"time"`
}

func (j *joiner) config() (config, error) {
	var c config
	b, err := os.ReadFile(filepath.Join(j.dir, "config.json"))
	if err != nil {
		return c, fmt.Errorf("no %s, so nobody joins with a work email", filepath.Join(j.dir, "config.json"))
	}
	if err := json.Unmarshal(b, &c); err != nil {
		return c, err
	}
	if c.Domain == "" || c.ClientID == "" || c.ClientSecret == "" || c.PublicURL == "" {
		return c, errors.New("config.json needs domain, client_id, client_secret and public_url")
	}
	c.Listen = cmpOr(c.Listen, "127.0.0.1:8790")
	c.PublicURL = strings.TrimRight(c.PublicURL, "/")
	return c, nil
}

func cmpOr(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

func (j *joiner) run(_ context.Context, op string) (any, error) {
	if op != "status" {
		return nil, fmt.Errorf("workjoin has no operation %s", op)
	}
	c, err := j.config()
	if err != nil {
		return map[string]any{"on": false, "why": err.Error()}, nil
	}
	people := j.joined()
	names := []string{}
	for email, p := range people {
		names = append(names, p.Name+" <"+email+">")
	}
	return map[string]any{"on": true, "domain": c.Domain, "url": c.PublicURL, "joined": names}, nil
}

func (j *joiner) joined() joined {
	out := joined{}
	if b, err := os.ReadFile(filepath.Join(j.dir, "joined.json")); err == nil {
		_ = json.Unmarshal(b, &out)
	}
	return out
}

func (j *joiner) routes() http.Handler {
	m := http.NewServeMux()
	m.HandleFunc("GET /{$}", j.home)
	m.HandleFunc("GET /start", j.start)
	m.HandleFunc("GET /callback", j.callback)
	return m
}

var page = template.Must(template.New("page").Parse(`<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Join</title>
<style>body{font:17px/1.4 -apple-system,system-ui,sans-serif;max-width:520px;margin:12vh auto;padding:0 20px;color:#111}
a.button{display:inline-block;background:#1a73e8;color:#fff;padding:10px 18px;border-radius:8px;text-decoration:none;font-weight:600}
code{display:block;word-break:break-all;background:#f2f2f7;padding:12px;border-radius:8px;font-size:14px}.muted{color:#666;font-size:15px}
.error{color:#c00}</style></head><body>{{if .Error}}<h1>That didn't work</h1><p class="error">{{.Error}}</p><p><a href="/">Try again</a></p>
{{else if .Code}}<h1>Welcome, {{.Name}}</h1><p>On your iPhone, with the app installed, open this link:</p><p><a class="button" href="{{.Link}}">Join</a></p>
<p class="muted">Or on a computer with the app, run:</p><code>{{.CLI}} join {{.Code}}</code><p class="muted">The code works once, for a week. Don't share it.</p>
{{else}}<h1>Join with your work email</h1><p>Sign in with your <b>@{{.Domain}}</b> Google account to join.</p><p><a class="button" href="/start">Sign in with Google</a></p>{{end}}
</body></html>`))

type view struct {
	Domain, Error, Code, Name, Link, CLI string
}

func (j *joiner) show(w http.ResponseWriter, v view) {
	w.Header().Set("content-type", "text/html; charset=utf-8")
	_ = page.Execute(w, v)
}

func (j *joiner) home(w http.ResponseWriter, _ *http.Request) {
	c, err := j.config()
	if err != nil {
		http.Error(w, "joining with a work email is off", http.StatusNotFound)
		return
	}
	j.show(w, view{Domain: c.Domain})
}

func random() string {
	b := make([]byte, 32)
	_, _ = rand.Read(b)
	return base64.RawURLEncoding.EncodeToString(b)
}

// start sends the person to Google, limited to the domain, with PKCE.
func (j *joiner) start(w http.ResponseWriter, r *http.Request) {
	c, err := j.config()
	if err != nil {
		http.Error(w, "joining with a work email is off", http.StatusNotFound)
		return
	}
	state, verifier := random(), random()
	j.mu.Lock()
	if j.pending == nil {
		j.pending = map[string]pending{}
	}
	for s, p := range j.pending {
		if time.Since(p.made) > 10*time.Minute {
			delete(j.pending, s)
		}
	}
	j.pending[state] = pending{verifier: verifier, made: time.Now()}
	j.mu.Unlock()
	sum := sha256.Sum256([]byte(verifier))
	q := url.Values{
		"client_id": {c.ClientID}, "redirect_uri": {c.PublicURL + "/callback"}, "response_type": {"code"},
		"scope": {"openid email profile"}, "hd": {c.Domain}, "state": {state}, "prompt": {"select_account"},
		"code_challenge": {base64.RawURLEncoding.EncodeToString(sum[:])}, "code_challenge_method": {"S256"},
	}
	http.Redirect(w, r, authURL+"?"+q.Encode(), http.StatusFound)
}

// callback checks Google's answer and, for someone on the domain, makes them a
// pairing code as a new member.
func (j *joiner) callback(w http.ResponseWriter, r *http.Request) {
	c, err := j.config()
	if err != nil {
		http.Error(w, "joining with a work email is off", http.StatusNotFound)
		return
	}
	j.mu.Lock()
	p, ok := j.pending[r.URL.Query().Get("state")]
	delete(j.pending, r.URL.Query().Get("state"))
	j.mu.Unlock()
	if !ok {
		j.show(w, view{Error: "That sign-in took too long or was already used."})
		return
	}
	email, name, err := verify(c, r.URL.Query().Get("code"), p.verifier)
	if err != nil {
		j.show(w, view{Error: err.Error()})
		return
	}
	code, err := j.admit(c, email, name)
	if err != nil {
		j.show(w, view{Error: err.Error()})
		return
	}
	cli := cmpOr(os.Getenv("ONE_CLI"), "one")
	link := cli + "://join?" + url.Values{"code": {code}, "name": {name + "'s iPhone"}}.Encode()
	j.show(w, view{Code: code, Name: name, Link: link, CLI: cli})
}

// verify exchanges the code for an ID token and checks it's a verified address
// on the domain, for this client.
func verify(c config, code, verifier string) (email, name string, err error) {
	resp, err := http.PostForm(tokenURL, url.Values{
		"code": {code}, "client_id": {c.ClientID}, "client_secret": {c.ClientSecret},
		"redirect_uri": {c.PublicURL + "/callback"}, "grant_type": {"authorization_code"}, "code_verifier": {verifier},
	})
	if err != nil {
		return "", "", err
	}
	defer resp.Body.Close()
	var tok struct {
		IDToken string `json:"id_token"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&tok); err != nil || tok.IDToken == "" {
		return "", "", errors.New("Google didn't sign you in")
	}
	info, err := http.Get(tokeninfoURL + "?" + url.Values{"id_token": {tok.IDToken}}.Encode())
	if err != nil {
		return "", "", err
	}
	defer info.Body.Close()
	var claims struct {
		Aud, Email, Hd, Name string
		Verified             string `json:"email_verified"`
	}
	if info.StatusCode != http.StatusOK || json.NewDecoder(info.Body).Decode(&claims) != nil {
		return "", "", errors.New("Google's answer didn't check out")
	}
	switch {
	case claims.Aud != c.ClientID:
		return "", "", errors.New("that sign-in was for another app")
	case claims.Verified != "true":
		return "", "", errors.New("Google hasn't verified that address")
	case claims.Hd != c.Domain || !strings.HasSuffix(strings.ToLower(claims.Email), "@"+strings.ToLower(c.Domain)):
		return "", "", fmt.Errorf("only @%s accounts join here; you signed in as %s", c.Domain, claims.Email)
	}
	return strings.ToLower(claims.Email), cmpOr(claims.Name, strings.Split(claims.Email, "@")[0]), nil
}

// admit asks the node, as its local user (an admin), for a new member's
// pairing code, once per address; and invites them to the group once they're in.
func (j *joiner) admit(c config, email, name string) (string, error) {
	j.mu.Lock()
	defer j.mu.Unlock()
	people := j.joined()
	if p, ok := people[email]; ok {
		return "", fmt.Errorf("%s already joined as %s: add another device from one you have (Settings, Devices, Add)", email, p.Name)
	}
	var out struct{ Code string }
	if err := post(j.node+"/v1/pair", map[string]any{"new_user": map[string]any{"name": name, "role": "member"}}, &out); err != nil {
		return "", fmt.Errorf("the install wouldn't make you a code: %w", err)
	}
	people[email] = struct {
		Name string `json:"name"`
		Time int64  `json:"time"`
	}{name, time.Now().UnixMilli()}
	b, _ := json.MarshalIndent(people, "", "  ")
	if err := os.WriteFile(filepath.Join(j.dir, "joined.json"), b, 0o600); err != nil {
		return "", err
	}
	if c.Group != "" {
		go j.invite(c.Group, name)
	}
	return out.Code, nil
}

// invite waits up to a week for the new user to join, then invites them to
// the group they land in.
func (j *joiner) invite(group, name string) {
	for range 7 * 24 * 60 {
		time.Sleep(time.Minute)
		if err := post(j.node+"/v1/invite", map[string]any{"group": group, "user": name, "role": "member"}, nil); err == nil {
			return
		}
	}
}
