package sdk

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"os"
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

// Google is Google's OAuth server, asking for a refresh token.
func Google(scopes ...string) OAuthProvider {
	return OAuthProvider{
		AuthURL:   "https://accounts.google.com/o/oauth2/v2/auth",
		TokenURL:  "https://oauth2.googleapis.com/token",
		Scopes:    scopes,
		AuthQuery: map[string]string{"access_type": "offline", "prompt": "consent"},
	}
}

// Account is a signed-in OAuth account. Its client and refresh token are the
// plugin's own log entries, so every device can use it.
type Account struct {
	ctx      *Ctx
	provider OAuthProvider

	mu           sync.Mutex
	clientID     string
	clientSecret string
	refresh      string
	access       string
	expires      time.Time
	pending      map[string]string // state → PKCE verifier
	onSignIn     []func(*Call)
}

type (
	oauthClient struct {
		ClientID     string `json:"clientId"`
		ClientSecret string `json:"clientSecret,omitempty"`
	}
	oauthToken struct {
		RefreshToken string `json:"refreshToken"`
	}
	signInIn struct {
		ClientID     string `json:"clientId,omitempty" jsonschema:"OAuth client id; needed the first time"`
		ClientSecret string `json:"clientSecret,omitempty" jsonschema:"OAuth client secret, if the client has one"`
		RefreshToken string `json:"refreshToken,omitempty" jsonschema:"an existing refresh token, to skip the browser"`
	}
	noInput struct{}
)

// OAuth adds sign_in, sign_out and account operations to the plugin and
// returns the account, whose Token is used to call the service.
func OAuth(c *Ctx, p OAuthProvider) *Account {
	a := &Account{ctx: c, provider: p, pending: map[string]string{}}
	c.Entry("oauth.client", Available)
	c.Entry("oauth.token", Available)
	c.Entry("oauth.signout", Available)
	c.OnReset(func() {
		a.mu.Lock()
		a.clientID, a.clientSecret, a.refresh, a.access = "", "", "", ""
		a.mu.Unlock()
	})
	Handle(c, "oauth.client", func(_ Entry, in oauthClient) error {
		a.mu.Lock()
		a.clientID, a.clientSecret = in.ClientID, in.ClientSecret
		a.mu.Unlock()
		return nil
	})
	Handle(c, "oauth.token", func(_ Entry, in oauthToken) error {
		a.mu.Lock()
		a.refresh, a.access = in.RefreshToken, ""
		a.mu.Unlock()
		return nil
	})
	Handle(c, "oauth.signout", func(Entry, struct{}) error {
		a.mu.Lock()
		a.refresh, a.access = "", ""
		a.mu.Unlock()
		return nil
	})

	Op(c, "sign_in", "Connect the account. Returns a link to open in a browser on this device; or pass an existing refresh token.", Write, func(call *Call, in signInIn) (Result, error) {
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
			if _, err := a.exchange(call.Context, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {in.RefreshToken}}); err != nil {
				return Result{}, fmt.Errorf("that refresh token doesn't work: %w", err)
			}
			if _, err := call.Append("oauth.token", oauthToken{in.RefreshToken}); err != nil {
				return Result{}, err
			}
			a.signedIn(call)
			return Text("Signed in."), nil
		}
		state, verifier := random(), random()
		challenge := sha256.Sum256([]byte(verifier))
		a.mu.Lock()
		a.pending[state] = verifier
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
	Op(c, "sign_out", "Disconnect the account on every device.", Write, func(call *Call, _ noInput) (Result, error) {
		if _, err := call.Append("oauth.signout", struct{}{}); err != nil {
			return Result{}, err
		}
		return Text("Signed out."), nil
	})
	Op(c, "account", "Show whether the account is connected.", Read, func(call *Call, _ noInput) (Result, error) {
		if !a.SignedIn() {
			return Text("Not signed in. Use sign_in."), nil
		}
		if _, err := a.Token(call.Context); err != nil {
			return Text("Signed in, but the token no longer works (" + err.Error() + "). Use sign_in again."), nil
		}
		return Text("Signed in."), nil
	})
	c.Hook("oauth", func(call *Call, req protocol.HookRequest) (protocol.HookResponse, error) {
		page := func(status int, msg string) (protocol.HookResponse, error) {
			return protocol.HookResponse{Status: status, ContentType: "text/html; charset=utf-8",
				Body: "<!doctype html><title>" + c.id + "</title><p style='font:16px system-ui;margin:3em'>" + msg + "</p>"}, nil
		}
		if e := req.Query["error"]; e != "" {
			return page(400, "Sign-in failed: "+e)
		}
		a.mu.Lock()
		verifier, ok := a.pending[req.Query["state"]]
		delete(a.pending, req.Query["state"])
		a.mu.Unlock()
		if !ok {
			return page(400, "This sign-in link has expired. Start again.")
		}
		tok, err := a.exchange(call.Context, url.Values{
			"grant_type": {"authorization_code"}, "code": {req.Query["code"]},
			"redirect_uri": {a.redirectURI()}, "code_verifier": {verifier},
		})
		if err != nil {
			return page(400, "Sign-in failed: "+err.Error())
		}
		if tok.RefreshToken == "" {
			return page(400, "The service didn't return a refresh token, so the connection wouldn't last.")
		}
		if _, err := call.Append("oauth.token", oauthToken{tok.RefreshToken}); err != nil {
			return page(500, err.Error())
		}
		a.signedIn(call)
		return page(200, "Connected "+c.id+". You can close this tab.")
	})
	return a
}

// OnSignIn runs after the account connects.
func (a *Account) OnSignIn(fn func(*Call)) { a.onSignIn = append(a.onSignIn, fn) }

func (a *Account) signedIn(call *Call) {
	for _, fn := range a.onSignIn {
		fn(call)
	}
}

// SignedIn reports whether a refresh token is held.
func (a *Account) SignedIn() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.refresh != ""
}

// Token returns a current access token, refreshing it when needed.
func (a *Account) Token(ctx context.Context) (string, error) {
	a.mu.Lock()
	if a.access != "" && time.Until(a.expires) > time.Minute {
		defer a.mu.Unlock()
		return a.access, nil
	}
	refresh := a.refresh
	a.mu.Unlock()
	if refresh == "" {
		return "", errors.New("not signed in")
	}
	tok, err := a.exchange(ctx, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {refresh}})
	if err != nil {
		return "", err
	}
	return tok.AccessToken, nil
}

type tokenResponse struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresIn    int    `json:"expires_in"`
	Error        string `json:"error"`
	Description  string `json:"error_description"`
}

func (a *Account) exchange(ctx context.Context, form url.Values) (tokenResponse, error) {
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
	a.mu.Lock()
	a.access, a.expires = tok.AccessToken, time.Now().Add(time.Duration(tok.ExpiresIn)*time.Second)
	a.mu.Unlock()
	return tok, nil
}

// redirectURI is this device's node, which passes the request to the hook.
func (a *Account) redirectURI() string {
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
