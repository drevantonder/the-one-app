package sdk

import (
	"cmp"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"html"
	"net/http"
	"net/url"
	"os"
	"slices"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/protocol"
)

// OAuthProvider describes an OAuth 2.0 authorization server.
type OAuthProvider struct {
	AuthURL   string            `json:"authUrl"`
	TokenURL  string            `json:"tokenUrl"`
	Scopes    []string          `json:"scopes,omitempty"`
	AuthQuery map[string]string `json:"authQuery,omitempty"` // extra authorization parameters
}

// Google is Google's OAuth server, asking for a refresh token, and for the
// email address that names each account.
func Google(scopes ...string) OAuthProvider {
	return OAuthProvider{
		AuthURL:   "https://accounts.google.com/o/oauth2/v2/auth",
		TokenURL:  "https://oauth2.googleapis.com/token",
		Scopes:    append([]string{"openid", "email"}, scopes...),
		AuthQuery: map[string]string{"access_type": "offline", "prompt": "consent"},
	}
}

// Accounts are a plugin's signed-in OAuth accounts, each named by its email
// address (or as the user chose). Their client and refresh tokens are the
// plugin's own log entries, so every device can use them.
type Accounts struct {
	ctx      *Ctx
	provider OAuthProvider

	mu           sync.Mutex
	clientID     string
	clientSecret string
	grants       map[string]*grant
	pending      map[string]pendingSignIn // by OAuth state
	ready        bool                     // the log has been replayed
	onChange     []func(*Call)
}

type grant struct {
	refresh string
	access  string
	expires time.Time
}

type pendingSignIn struct{ verifier, account string }

// defaultAccount names an account the service didn't name, and accounts
// signed in before they had names.
const defaultAccount = "default"

type (
	oauthClient struct {
		ClientID     string `json:"clientId"`
		ClientSecret string `json:"clientSecret,omitempty"`
	}
	oauthToken struct {
		RefreshToken string `json:"refreshToken"`
		Account      string `json:"account,omitempty"`
	}
	oauthSignOut struct {
		Account string `json:"account,omitempty"` // empty: every account
	}
	signInIn struct {
		ClientID     string `json:"clientId,omitempty" jsonschema:"OAuth client id; needed the first time"`
		ClientSecret string `json:"clientSecret,omitempty" jsonschema:"OAuth client secret, if the client has one"`
		RefreshToken string `json:"refreshToken,omitempty" jsonschema:"an existing refresh token, to skip the browser"`
		Account      string `json:"account,omitempty" jsonschema:"a name for the account; default is its email address, if the service gives one"`
	}
	accountIn struct {
		Account string `json:"account,omitempty" jsonschema:"which account, as listed by accounts; needed only when several are signed in"`
	}
	noInput struct{}
)

// OAuth adds sign_in, sign_out and accounts operations to the plugin and
// returns its accounts, whose Token is used to call the service.
func OAuth(c *Ctx, p OAuthProvider) *Accounts {
	a := &Accounts{ctx: c, provider: p, grants: map[string]*grant{}, pending: map[string]pendingSignIn{}}
	c.Entry("oauth.client", Available)
	c.Entry("oauth.token", Available)
	c.Entry("oauth.signout", Available)
	c.OnReset(func() {
		a.mu.Lock()
		a.clientID, a.clientSecret, a.grants, a.ready = "", "", map[string]*grant{}, false
		a.mu.Unlock()
	})
	c.OnReady(func(*Call) {
		a.mu.Lock()
		a.ready = true
		a.mu.Unlock()
	})
	Handle(c, "oauth.client", func(_ Entry, in oauthClient) error {
		a.mu.Lock()
		a.clientID, a.clientSecret = in.ClientID, in.ClientSecret
		a.mu.Unlock()
		return nil
	})
	Handle(c, "oauth.token", func(_ Entry, in oauthToken) error {
		name := cmp.Or(in.Account, defaultAccount)
		a.mu.Lock()
		_, had := a.grants[name]
		a.grants[name] = &grant{refresh: in.RefreshToken}
		a.mu.Unlock()
		if !had {
			a.changed()
		}
		return nil
	})
	Handle(c, "oauth.signout", func(_ Entry, in oauthSignOut) error {
		a.mu.Lock()
		before := len(a.grants)
		if in.Account == "" {
			a.grants = map[string]*grant{}
		} else {
			delete(a.grants, in.Account)
		}
		removed := len(a.grants) != before
		a.mu.Unlock()
		if removed {
			a.changed()
		}
		return nil
	})

	Op(c, "sign_in", "Connect an account; signing in to another adds it. Returns a link to open in a browser on this device; or pass an existing refresh token.", Write, func(call *Call, in signInIn) (Result, error) {
		if in.ClientID != "" {
			if _, err := call.Append("oauth.client", oauthClient{in.ClientID, in.ClientSecret}); err != nil {
				return Result{}, err
			}
		}
		a.mu.Lock()
		clientID := a.clientID
		a.mu.Unlock()
		if clientID == "" {
			return Result{}, errors.New("no OAuth client yet: pass clientId (and clientSecret) from the service's developer console")
		}
		if in.RefreshToken != "" {
			tok, err := a.exchange(call.Context, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {in.RefreshToken}})
			if err != nil {
				return Result{}, fmt.Errorf("that refresh token doesn't work: %w", err)
			}
			name := a.name(in.Account, tok)
			if _, err := call.Append("oauth.token", oauthToken{in.RefreshToken, name}); err != nil {
				return Result{}, err
			}
			return Text("Signed in as " + name + "."), nil
		}
		state, verifier := random(), random()
		challenge := sha256.Sum256([]byte(verifier))
		a.mu.Lock()
		a.pending[state] = pendingSignIn{verifier, in.Account}
		a.mu.Unlock()
		q := url.Values{
			"client_id":             {clientID},
			"redirect_uri":          {a.redirectURI()},
			"response_type":         {"code"},
			"scope":                 {strings.Join(a.provider.Scopes, " ")},
			"state":                 {state},
			"code_challenge":        {base64.RawURLEncoding.EncodeToString(challenge[:])},
			"code_challenge_method": {"S256"},
		}
		for k, v := range a.provider.AuthQuery {
			q.Set(k, v)
		}
		return Text("Open this link in a browser on this device to sign in:\n" + a.provider.AuthURL + "?" + q.Encode()), nil
	})
	Op(c, "sign_out", "Disconnect an account on every device.", Write, func(call *Call, in accountIn) (Result, error) {
		name, err := a.Resolve(in.Account)
		if err != nil {
			return Result{}, err
		}
		if _, err := call.Append("oauth.signout", oauthSignOut{name}); err != nil {
			return Result{}, err
		}
		return Text("Signed out " + name + "."), nil
	})
	Op(c, "accounts", "List the connected accounts, and whether each still works.", Read, func(call *Call, _ noInput) (Result, error) {
		names := a.Names()
		if len(names) == 0 {
			return Text("Not signed in. Use sign_in."), nil
		}
		var lines []string
		for _, name := range names {
			if _, err := a.Token(call.Context, name); err != nil {
				lines = append(lines, err.Error())
				continue
			}
			lines = append(lines, name+": signed in")
		}
		return Text(strings.Join(lines, "\n")), nil
	})
	c.Hook("oauth", func(call *Call, req protocol.HookRequest) (protocol.HookResponse, error) {
		page := func(status int, msg string) (protocol.HookResponse, error) {
			return protocol.HookResponse{Status: status, ContentType: "text/html; charset=utf-8",
				Body: "<!doctype html><title>" + c.id + "</title><p style='font:16px system-ui;margin:3em'>" + html.EscapeString(msg) + "</p>"}, nil
		}
		if e := req.Query["error"]; e != "" {
			return page(400, "Sign-in failed: "+e)
		}
		a.mu.Lock()
		p, ok := a.pending[req.Query["state"]]
		delete(a.pending, req.Query["state"])
		a.mu.Unlock()
		if !ok {
			return page(400, "This sign-in link has expired. Start again.")
		}
		tok, err := a.exchange(call.Context, url.Values{
			"grant_type": {"authorization_code"}, "code": {req.Query["code"]},
			"redirect_uri": {a.redirectURI()}, "code_verifier": {p.verifier},
		})
		if err != nil {
			return page(400, "Sign-in failed: "+err.Error())
		}
		if tok.RefreshToken == "" {
			return page(400, "The service didn't return a refresh token, so the connection wouldn't last.")
		}
		name := a.name(p.account, tok)
		if _, err := call.Append("oauth.token", oauthToken{tok.RefreshToken, name}); err != nil {
			return page(500, err.Error())
		}
		return page(200, "Connected "+name+" to "+c.id+". You can close this tab.")
	})
	return a
}

// OnChange runs whenever an account is added or removed, on any device,
// once the log has been replayed.
func (a *Accounts) OnChange(fn func(*Call)) { a.onChange = append(a.onChange, fn) }

func (a *Accounts) changed() {
	a.mu.Lock()
	ready, fns := a.ready, a.onChange
	a.mu.Unlock()
	if !ready {
		return
	}
	// Entries are applied one at a time; reacting may take a while.
	go func() {
		for _, fn := range fns {
			fn(&Call{Context: context.Background(), Caller: Caller{Actor: "oauth"}, ctx: a.ctx})
		}
	}()
}

// name is what a new sign-in is called: the user's choice, or the email
// address the service gave, or the default.
func (a *Accounts) name(chosen string, tok tokenResponse) string {
	if chosen != "" {
		return chosen
	}
	if email := idTokenEmail(tok.IDToken); email != "" {
		return email
	}
	return defaultAccount
}

// idTokenEmail reads the email claim of an OpenID Connect ID token. It came
// straight from the token endpoint, so it isn't verified again here.
func idTokenEmail(token string) string {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return ""
	}
	raw, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return ""
	}
	var claims struct {
		Email string `json:"email"`
	}
	_ = json.Unmarshal(raw, &claims)
	return claims.Email
}

// Names lists the signed-in accounts.
func (a *Accounts) Names() []string {
	a.mu.Lock()
	defer a.mu.Unlock()
	names := make([]string, 0, len(a.grants))
	for name := range a.grants {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

// Resolve names the account to use: the one given, or the only one.
func (a *Accounts) Resolve(name string) (string, error) {
	names := a.Names()
	switch {
	case len(names) == 0:
		return "", errors.New("not signed in: use sign_in first")
	case name == "" && len(names) == 1:
		return names[0], nil
	case name == "":
		return "", fmt.Errorf("several accounts are signed in; pass account: %s", strings.Join(names, ", "))
	case !slices.Contains(names, name):
		return "", fmt.Errorf("no account %q is signed in; they are: %s", name, strings.Join(names, ", "))
	}
	return name, nil
}

// Token returns a current access token for an account (or the only one),
// refreshing it when needed.
func (a *Accounts) Token(ctx context.Context, account string) (string, error) {
	name, err := a.Resolve(account)
	if err != nil {
		return "", err
	}
	a.mu.Lock()
	g := a.grants[name]
	if g.access != "" && time.Until(g.expires) > time.Minute {
		defer a.mu.Unlock()
		return g.access, nil
	}
	refresh := g.refresh
	a.mu.Unlock()
	tok, err := a.exchange(ctx, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {refresh}})
	if err != nil {
		return "", fmt.Errorf("%s's token no longer works (%w); use sign_in again", name, err)
	}
	a.mu.Lock()
	g.access, g.expires = tok.AccessToken, time.Now().Add(time.Duration(tok.ExpiresIn)*time.Second)
	a.mu.Unlock()
	return tok.AccessToken, nil
}

// withAccount adds the account argument to an operation's input schema when
// several accounts are signed in, unless it has its own.
func (a *Accounts) withAccount(schema map[string]any) map[string]any {
	if a == nil || len(a.Names()) < 2 {
		return schema
	}
	props, _ := schema["properties"].(map[string]any)
	if props == nil {
		props = map[string]any{}
	}
	if _, own := props["account"]; own {
		return schema
	}
	props["account"] = map[string]any{"type": "string", "enum": a.Names(), "description": "Which account to use."}
	schema["properties"] = props
	return schema
}

// takeAccount removes the account argument from an operation's input,
// unless the operation has its own.
func takeAccount(input map[string]any, own bool) string {
	if own {
		return ""
	}
	name, _ := input["account"].(string)
	delete(input, "account")
	return name
}

type tokenResponse struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	IDToken      string `json:"id_token"`
	ExpiresIn    int    `json:"expires_in"`
	Error        string `json:"error"`
	Description  string `json:"error_description"`
}

func (a *Accounts) exchange(ctx context.Context, form url.Values) (tokenResponse, error) {
	a.mu.Lock()
	form.Set("client_id", a.clientID)
	if a.clientSecret != "" {
		form.Set("client_secret", a.clientSecret)
	}
	a.mu.Unlock()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, a.provider.TokenURL, strings.NewReader(form.Encode()))
	if err != nil {
		return tokenResponse{}, err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Accept", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return tokenResponse{}, err
	}
	defer resp.Body.Close()
	var tok tokenResponse
	if err := json.NewDecoder(resp.Body).Decode(&tok); err != nil {
		return tokenResponse{}, fmt.Errorf("token endpoint: %s", resp.Status)
	}
	if tok.Error != "" || tok.AccessToken == "" {
		return tok, fmt.Errorf("%s %s", tok.Error, tok.Description)
	}
	return tok, nil
}

// redirectURI is this device's node, which passes the request to the hook.
func (a *Accounts) redirectURI() string {
	addr := os.Getenv("ONE_ADDR")
	if addr == "" {
		addr = "127.0.0.1:4747"
	}
	return "http://" + addr + "/hooks/" + a.ctx.id + "/oauth"
}

func random() string {
	b := make([]byte, 24)
	_, _ = rand.Read(b)
	return base64.RawURLEncoding.EncodeToString(b)
}
