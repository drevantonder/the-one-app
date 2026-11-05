// Package signin signs a shell in, as every client does: it's an OAuth client of the install's sign-in, registered the
// first time it signs in, and sign-in and consent happen in a browser (the person's own, where their passkey works; or,
// for a throwaway network, which asks for none, anywhere) that comes back to a port on this machine (RFC 8252's
// loopback redirect). The refresh token is kept in the OS's secret store (Store); access tokens only in memory. It
// names the device it's for (`join`), so the person's devices can let it in by this sign-in.
package signin

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
)

// Store is the OS's secret store (mygo.Secrets on a computer, the Keychain on an iPhone).
type Store interface {
	Get(key string) ([]byte, error)
	Set(key string, value []byte) error
	Delete(key string) error
}

// ErrSignedOut: there's no sign-in, or sign-in refused it.
var ErrSignedOut = errors.New("Sign in again.")

// Auth is one install's sign-in.
type Auth struct {
	Address string
	Name    string // what this computer is called among the person's sign-ins
	Store   Store

	mu     sync.Mutex
	access string
	until  time.Time
	meta   *meta
}

type meta struct {
	Authorization string `json:"authorization_endpoint"`
	Token         string `json:"token_endpoint"`
	Registration  string `json:"registration_endpoint"`
	Revocation    string `json:"revocation_endpoint"`
}

var http_ = &http.Client{Timeout: 30 * time.Second}

func (a *Auth) key(k string) string { return a.Address + " " + k }

func (a *Auth) getMeta() (*meta, error) {
	if a.meta != nil {
		return a.meta, nil
	}
	r, err := http_.Get(a.Address + "/.well-known/oauth-authorization-server")
	if err != nil || r.StatusCode != 200 {
		return nil, errors.New("Can’t reach that address. Check it, and that you’re online.")
	}
	defer r.Body.Close()
	var m meta
	if err := json.NewDecoder(r.Body).Decode(&m); err != nil {
		return nil, err
	}
	a.meta = &m
	return &m, nil
}

// SignedIn says whether a refresh token is kept.
func (a *Auth) SignedIn() bool {
	b, err := a.Store.Get(a.key("refresh"))
	return err == nil && len(b) > 0
}

func (a *Auth) client(m *meta) (string, error) {
	if id, err := a.Store.Get(a.key("client")); err == nil && len(id) > 0 {
		return string(id), nil
	}
	body, _ := json.Marshal(map[string]any{
		"client_name": a.Name, "redirect_uris": []string{"http://127.0.0.1/callback"}, "token_endpoint_auth_method": "none",
		"grant_types": []string{"authorization_code", "refresh_token"}, "response_types": []string{"code"}, "application_type": "native",
	})
	r, err := http_.Post(m.Registration, "application/json", strings.NewReader(string(body)))
	if err != nil {
		return "", err
	}
	defer r.Body.Close()
	var out struct {
		ClientID string `json:"client_id"`
		Desc     string `json:"error_description"`
	}
	json.NewDecoder(r.Body).Decode(&out)
	if r.StatusCode >= 300 || out.ClientID == "" {
		return "", fmt.Errorf("this computer couldn’t sign up to sign in: %s", out.Desc)
	}
	return out.ClientID, a.Store.Set(a.key("client"), []byte(out.ClientID))
}

func b64u(b []byte) string { return base64.RawURLEncoding.EncodeToString(b) }

func random(n int) string {
	b := make([]byte, n)
	rand.Read(b)
	return b64u(b)
}

// SignIn signs in: `open` shows the authorization URL in a browser (the system's, or a window's), which comes back to
// this machine's loopback port; `join` is the device it's for (client.DeviceJoin).
func (a *Auth) SignIn(ctx context.Context, join string, open func(url string) error) error {
	m, err := a.getMeta()
	if err != nil {
		return err
	}
	id, err := a.client(m)
	if err != nil {
		return err
	}
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	defer ln.Close()
	redirect := fmt.Sprintf("http://%s/callback", ln.Addr())
	verifier, state := random(32), random(16)
	sum := sha256.Sum256([]byte(verifier))
	q := url.Values{
		"response_type": {"code"}, "client_id": {id}, "redirect_uri": {redirect}, "scope": {"openid profile offline_access"},
		"code_challenge": {b64u(sum[:])}, "code_challenge_method": {"S256"}, "state": {state}, "prompt": {"consent"},
	}
	if join != "" {
		q.Set("join", join)
	}
	back := make(chan url.Values, 1)
	srv := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/callback" {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("content-type", "text/html; charset=utf-8")
		fmt.Fprint(w, `<!doctype html><meta charset=utf-8><meta name=color-scheme content="light dark"><body style="font:15px system-ui;display:grid;place-items:center;height:90vh;color:GrayText"><p>You’re signed in. Go back to the app.</p>`)
		select {
		case back <- r.URL.Query():
		default:
		}
	})}
	go srv.Serve(ln)
	defer srv.Close()
	if err := open(m.Authorization + "?" + q.Encode()); err != nil {
		return err
	}
	var got url.Values
	select {
	case got = <-back:
	case <-ctx.Done():
		return ctx.Err()
	}
	if e := got.Get("error"); e != "" {
		if e == "access_denied" {
			return errors.New("You didn’t let this computer in.")
		}
		return errors.New(firstOf(got.Get("error_description"), e))
	}
	if got.Get("state") != state || got.Get("code") == "" {
		return errors.New("That sign-in didn’t start here. Sign in again.")
	}
	return a.grant(m, url.Values{"grant_type": {"authorization_code"}, "code": {got.Get("code")}, "redirect_uri": {redirect}, "client_id": {id}, "code_verifier": {verifier}})
}

func firstOf(s ...string) string {
	for _, v := range s {
		if v != "" {
			return v
		}
	}
	return ""
}

func (a *Auth) grant(m *meta, fields url.Values) error {
	r, err := http_.PostForm(m.Token, fields)
	if err != nil {
		return err
	}
	defer r.Body.Close()
	var out struct {
		Access  string  `json:"access_token"`
		Refresh string  `json:"refresh_token"`
		Expires float64 `json:"expires_in"`
		Error   string  `json:"error"`
		Desc    string  `json:"error_description"`
	}
	json.NewDecoder(r.Body).Decode(&out)
	if r.StatusCode >= 300 || out.Access == "" {
		why := firstOf(out.Desc, out.Error, "Signing in didn’t work.")
		if r.StatusCode == 400 || r.StatusCode == 401 {
			return fmt.Errorf("%w %s", ErrSignedOut, why)
		}
		return errors.New(why)
	}
	if out.Expires == 0 {
		out.Expires = 3600
	}
	a.access, a.until = out.Access, time.Now().Add(time.Duration(out.Expires)*time.Second)
	// Each refresh token works once: the new one is kept before anything else can ask.
	if out.Refresh != "" {
		return a.Store.Set(a.key("refresh"), []byte(out.Refresh))
	}
	return nil
}

// Token is an access token good for a minute more, new from the refresh token if it's older.
func (a *Auth) Token() (string, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.access != "" && time.Until(a.until) > time.Minute {
		return a.access, nil
	}
	refresh, err := a.Store.Get(a.key("refresh"))
	if err != nil || len(refresh) == 0 {
		return "", ErrSignedOut
	}
	m, err := a.getMeta()
	if err != nil {
		return "", err
	}
	id, _ := a.Store.Get(a.key("client"))
	if err := a.grant(m, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {string(refresh)}, "client_id": {string(id)}}); err != nil {
		if errors.Is(err, ErrSignedOut) {
			a.Store.Delete(a.key("refresh"))
		}
		return "", err
	}
	return a.access, nil
}

// Subject is who the access token is for: its `sub`.
func Subject(token string) string {
	parts := strings.Split(token, ".")
	if len(parts) < 2 {
		return ""
	}
	b, err := base64.RawURLEncoding.DecodeString(strings.TrimRight(parts[1], "="))
	if err != nil {
		return ""
	}
	var c struct{ Sub string }
	json.Unmarshal(b, &c)
	return c.Sub
}

// SignOut revokes the refresh token, so it opens nothing again, and forgets it here.
func (a *Auth) SignOut() {
	a.mu.Lock()
	defer a.mu.Unlock()
	refresh, _ := a.Store.Get(a.key("refresh"))
	a.Store.Delete(a.key("refresh"))
	a.access = ""
	if m, err := a.getMeta(); err == nil && m.Revocation != "" && len(refresh) > 0 {
		id, _ := a.Store.Get(a.key("client"))
		http_.PostForm(m.Revocation, url.Values{"token": {string(refresh)}, "token_type_hint": {"refresh_token"}, "client_id": {string(id)}})
	}
}
