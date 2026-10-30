package main

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
	"slices"
	"strings"
	"sync"
	"time"
)

// redirect is where Google sends the browser after sign-in, the address the
// old app used, so the same OAuth client works. Nothing answers there: the
// browser is usually on another device, so the person copies the address it
// fails to load and passes it back to sign_in.
var redirect = "http://127.0.0.1:4747/hooks/gmail/oauth"

// The OAuth client and each account's refresh token, kept in one 0600 file.
type creds struct {
	Client   oauthClient `json:"client"`
	Accounts []account   `json:"accounts"`
}

// oauthClient is the OAuth client every account signed in with.
type oauthClient struct {
	ID     string `json:"id"`
	Secret string `json:"secret,omitempty"`
}

type account struct {
	Email   string `json:"email"`
	Refresh string `json:"refresh"`
}

type accounts struct {
	path string

	mu      sync.Mutex
	creds   creds
	access  map[string]token // by email
	pending pending
}

// pending is the sign-in in progress, with the client it began with, which
// replaces the kept one only once it works.
type pending struct {
	verifier string
	client   oauthClient
}

type token struct {
	value   string
	expires time.Time
}

func load(path string) (*accounts, error) {
	a := &accounts{path: path, access: map[string]token{}}
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return a, nil
	}
	if err != nil {
		return nil, err
	}
	return a, json.Unmarshal(b, &a.creds)
}

func (a *accounts) save() error {
	b, err := json.MarshalIndent(a.creds, "", "  ")
	if err != nil {
		return err
	}
	tmp := a.path + ".tmp"
	if err := os.WriteFile(tmp, b, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, a.path)
}

// resolve finds an account by its address or a part of it only one has; the
// first signed in when none is named.
func (a *accounts) resolve(name string) (account, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	all := a.creds.Accounts
	if len(all) == 0 {
		return account{}, errors.New("no Gmail account is signed in here: gmail_sign_in signs one in")
	}
	if name == "" {
		return all[0], nil
	}
	var found []account
	for _, acc := range all {
		if strings.EqualFold(acc.Email, name) {
			return acc, nil
		}
		if strings.Contains(strings.ToLower(acc.Email), strings.ToLower(name)) {
			found = append(found, acc)
		}
	}
	if len(found) == 1 {
		return found[0], nil
	}
	return account{}, fmt.Errorf("no single account matches %q; they are: %s", name, strings.Join(a.emails(), ", "))
}

func (a *accounts) emails() []string {
	var out []string
	for _, acc := range a.creds.Accounts {
		out = append(out, acc.Email)
	}
	return out
}

// token is a current access token for acc, refreshed when it's near expiry.
func (a *accounts) token(ctx context.Context, acc account) (string, error) {
	a.mu.Lock()
	t := a.access[acc.Email]
	a.mu.Unlock()
	if time.Until(t.expires) > time.Minute {
		return t.value, nil
	}
	a.mu.Lock()
	c := a.creds.Client
	a.mu.Unlock()
	tok, err := exchange(ctx, c, url.Values{"grant_type": {"refresh_token"}, "refresh_token": {acc.Refresh}})
	if err != nil {
		return "", fmt.Errorf("%s's sign-in no longer works (%w): gmail_sign_in signs it in again", acc.Email, err)
	}
	a.mu.Lock()
	a.access[acc.Email] = token{tok.AccessToken, time.Now().Add(time.Duration(tok.ExpiresIn) * time.Second)}
	a.mu.Unlock()
	return tok.AccessToken, nil
}

type tokenReply struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	IDToken      string `json:"id_token"`
	ExpiresIn    int    `json:"expires_in"`
	Error        string `json:"error"`
	Description  string `json:"error_description"`
}

func exchange(ctx context.Context, c oauthClient, form url.Values) (tokenReply, error) {
	form.Set("client_id", c.ID)
	if c.Secret != "" {
		form.Set("client_secret", c.Secret)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "https://oauth2.googleapis.com/token", strings.NewReader(form.Encode()))
	if err != nil {
		return tokenReply{}, err
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return tokenReply{}, err
	}
	defer resp.Body.Close()
	var tok tokenReply
	if err := json.NewDecoder(resp.Body).Decode(&tok); err != nil {
		return tokenReply{}, fmt.Errorf("%s from Google", resp.Status)
	}
	if tok.Error != "" {
		return tokenReply{}, fmt.Errorf("%s: %s", tok.Error, tok.Description)
	}
	return tok, nil
}

// signIn starts a sign-in, returning the link to open, or, given the address
// the browser was sent to (or just its code), finishes it.
func (a *accounts) signIn(ctx context.Context, clientID, clientSecret, code string) (string, error) {
	if code == "" {
		a.mu.Lock()
		c := a.creds.Client
		a.mu.Unlock()
		if clientID != "" {
			c = oauthClient{clientID, clientSecret}
		}
		if c.ID == "" {
			return "", errors.New("no OAuth client yet: pass client_id and client_secret of a Desktop app client from Google Cloud Console (APIs & Services > Credentials), with the Gmail API enabled")
		}
		return a.start(c), nil
	}
	if u, err := url.Parse(code); err == nil && u.Query().Get("code") != "" {
		code = u.Query().Get("code")
	}
	a.mu.Lock()
	p := a.pending
	a.mu.Unlock()
	if p.verifier == "" {
		return "", errors.New("no sign-in is in progress: call gmail_sign_in without code first")
	}
	tok, err := exchange(ctx, p.client, url.Values{"grant_type": {"authorization_code"}, "code": {code},
		"redirect_uri": {redirect}, "code_verifier": {p.verifier}})
	if err != nil {
		return "", fmt.Errorf("sign-in failed: %w", err)
	}
	if tok.RefreshToken == "" {
		return "", errors.New("Google didn't give a refresh token, so the sign-in wouldn't last; start again")
	}
	email := idTokenEmail(tok.IDToken)
	if email == "" {
		return "", errors.New("Google didn't say which account signed in; start again")
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	a.pending = pending{}
	done := "Signed in as " + email + "."
	if p.client != a.creds.Client && len(a.creds.Accounts) > 0 { // tokens belong to their client
		done += " The OAuth client changed, so the other accounts must sign in again."
		a.creds.Accounts, a.access = nil, map[string]token{}
	}
	a.creds.Client = p.client
	acc := account{email, tok.RefreshToken}
	if i := slices.IndexFunc(a.creds.Accounts, func(x account) bool { return x.Email == email }); i >= 0 {
		a.creds.Accounts[i] = acc
	} else {
		a.creds.Accounts = append(a.creds.Accounts, acc)
	}
	a.access[email] = token{tok.AccessToken, time.Now().Add(time.Duration(tok.ExpiresIn) * time.Second)}
	if err := a.save(); err != nil {
		return "", err
	}
	return done, nil
}

func (a *accounts) start(c oauthClient) string {
	b := make([]byte, 32)
	_, _ = rand.Read(b)
	verifier := base64.RawURLEncoding.EncodeToString(b)
	challenge := sha256.Sum256([]byte(verifier))
	a.mu.Lock()
	a.pending = pending{verifier, c}
	a.mu.Unlock()
	q := url.Values{
		"client_id":             {c.ID},
		"redirect_uri":          {redirect},
		"response_type":         {"code"},
		"scope":                 {"openid email https://www.googleapis.com/auth/gmail.modify"},
		"code_challenge":        {base64.RawURLEncoding.EncodeToString(challenge[:])},
		"code_challenge_method": {"S256"},
		"access_type":           {"offline"},
		"prompt":                {"consent"},
	}
	return "Open this link in a browser on any device and sign in. The browser then fails to load a 127.0.0.1 page: " +
		"copy that page's whole address and pass it to gmail_sign_in as code.\n\nhttps://accounts.google.com/o/oauth2/v2/auth?" + q.Encode()
}

func (a *accounts) signOut(name string) (string, error) {
	acc, err := a.resolve(name)
	if err != nil {
		return "", err
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	a.creds.Accounts = slices.DeleteFunc(a.creds.Accounts, func(x account) bool { return x.Email == acc.Email })
	delete(a.access, acc.Email)
	if err := a.save(); err != nil {
		return "", err
	}
	return "Signed out " + acc.Email + " here. To revoke access too, remove it at myaccount.google.com/permissions.", nil
}

// idTokenEmail reads the email claim of the ID token Google's token endpoint
// just gave, so it isn't verified again.
func idTokenEmail(t string) string {
	parts := strings.Split(t, ".")
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
