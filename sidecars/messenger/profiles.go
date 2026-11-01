package main

// Messenger's accounts are the agent computers' browser profiles it's linked to:
// each profile signed in to messenger.com is one
// account, read through a browser of its own.

import (
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

//go:embed settings.html
var settingsHTML string

// messenger is the profiles it's linked to, kept in its folder, and a
// browser for each.
type messenger struct {
	node string
	path string // profiles.json

	mu       sync.Mutex
	linked   []string          // the profiles, the default first
	out      map[string]string // why a profile needs signing in again, as its last read found
	browsers map[string]*browser
}

type saved struct {
	Linked []string          `json:"linked"`
	Out    map[string]string `json:"signed_out,omitempty"`
}

// load reads which profiles are linked; with none saved yet, the browser
// profile called default is the first account.
func load(node, dir string) (*messenger, error) {
	m := &messenger{node: node, path: filepath.Join(dir, "profiles.json"), linked: []string{"default"}, out: map[string]string{}, browsers: map[string]*browser{}}
	raw, err := os.ReadFile(m.path)
	if errors.Is(err, os.ErrNotExist) {
		return m, nil
	}
	if err != nil {
		return nil, err
	}
	var s saved
	if err := json.Unmarshal(raw, &s); err != nil {
		return nil, fmt.Errorf("%s: %w", m.path, err)
	}
	m.linked = s.Linked
	if s.Out != nil {
		m.out = s.Out
	}
	return m, nil
}

// save writes the profiles; run under mu.
func (m *messenger) save() error {
	raw, _ := json.MarshalIndent(saved{Linked: m.linked, Out: m.out}, "", "  ")
	if err := os.MkdirAll(filepath.Dir(m.path), 0o700); err != nil {
		return err
	}
	tmp := m.path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, m.path)
}

// browser is the profile's browser, or the default's when none is named.
func (m *messenger) browser(account string) (*browser, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if len(m.linked) == 0 {
		return nil, errors.New("Messenger isn't linked to a browser profile yet: messenger_sign_in links one")
	}
	profile := m.linked[0]
	if account != "" {
		i := slices.IndexFunc(m.linked, func(p string) bool { return strings.EqualFold(p, account) })
		if i < 0 {
			return nil, fmt.Errorf("Messenger isn't linked to a profile %q; it's linked to %s", account, strings.Join(m.linked, ", "))
		}
		profile = m.linked[i]
	}
	b := m.browsers[profile]
	if b == nil {
		b = &browser{node: m.node, profile: profile}
		m.browsers[profile] = b
	}
	return b, nil
}

// found keeps whether a read found the profile signed in.
func (m *messenger) found(profile string, err error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	was := m.out[profile]
	switch {
	case errors.Is(err, errNotSignedIn):
		m.out[profile] = "Messenger signed this profile out: sign in again"
	case err == nil:
		delete(m.out, profile)
	}
	if was != m.out[profile] {
		m.save()
	}
}

func (m *messenger) keepGivingBack() {
	for range time.Tick(time.Minute) {
		m.mu.Lock()
		all := make([]*browser, 0, len(m.browsers))
		for _, b := range m.browsers {
			all = append(all, b)
		}
		m.mu.Unlock()
		for _, b := range all {
			b.mu.Lock()
			b.giveBack()
			b.mu.Unlock()
		}
	}
}

func (m *messenger) run(op string, raw json.RawMessage) (any, error) {
	var in struct {
		Account, Profile, Name string
	}
	if len(raw) > 0 && string(raw) != "null" {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, fmt.Errorf("bad input: %w", err)
		}
	}
	switch op {
	case "_screen":
		if in.Name != "settings" {
			return nil, fmt.Errorf("Messenger has no screen %q", in.Name)
		}
		return settingsHTML, nil
	case "accounts":
		m.mu.Lock()
		defer m.mu.Unlock()
		out := []map[string]any{}
		for _, p := range m.linked {
			a := map[string]any{"account": p, "ok": m.out[p] == ""}
			if why := m.out[p]; why != "" {
				a["why"] = why
			}
			out = append(out, a)
		}
		return out, nil
	case "sign_in":
		return m.signIn(strings.TrimSpace(cmpOr(in.Profile, in.Account)))
	case "sign_out":
		return m.signOut(in.Account)
	case "_profiles": // the agent computers' profiles, for the settings screen to pick one
		b := &browser{node: m.node}
		return b.computers("profiles", map[string]any{})
	case "_save": // the settings screen's Done: saves the profile signed in
		b := &browser{node: m.node}
		if _, err := b.computers("save_profile", map[string]any{"profile": in.Profile}); err != nil {
			return nil, err
		}
		m.found(in.Profile, nil)
		return fmt.Sprintf("Saved the %s profile.", in.Profile), nil
	}
	b, err := m.browser(in.Account)
	if err != nil {
		return nil, err
	}
	result, err := b.run(op, raw)
	if op != "send" || err == nil {
		m.found(b.profile, err)
	}
	return result, err
}

// signIn links Messenger to a browser profile (made if new) and opens
// messenger.com in it, for the person to sign in.
func (m *messenger) signIn(profile string) (any, error) {
	m.mu.Lock()
	if profile == "" {
		profile = "default"
		if len(m.linked) > 0 {
			profile = m.linked[0]
		}
	}
	m.mu.Unlock()
	b := &browser{node: m.node}
	got, err := b.computers("sign_in", map[string]any{"profile": profile, "url": home})
	if err != nil && strings.Contains(err.Error(), "No browser profile") {
		// A new one: added to the Vault first.
		if _, err = b.computers("add_profile", map[string]any{"profile": profile}); err == nil {
			got, err = b.computers("sign_in", map[string]any{"profile": profile, "url": home})
		}
	}
	if err != nil {
		return nil, err
	}
	r, _ := got.(map[string]any)
	url, _ := r["url"].(string)
	m.mu.Lock()
	if !slices.Contains(m.linked, profile) {
		m.linked = append(m.linked, profile)
	}
	delete(m.out, profile)
	err = m.save()
	m.mu.Unlock()
	return map[string]any{
		"profile": profile, "url": url, "computer": r["computer"], "server": r["server"],
		"text": fmt.Sprintf("Give the person this link: %s\nThey sign in to Messenger there, in the %s browser profile (tick Keep me signed in), then press Done on Messenger's settings, or Done Signing In on the profile in the Vault; it's saved by itself after 30 minutes otherwise.", url, profile),
	}, err
}

// signOut unlinks a profile: Messenger stops reading it. The profile itself,
// and its sign-ins to other sites, stay.
func (m *messenger) signOut(account string) (string, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	i := slices.IndexFunc(m.linked, func(p string) bool { return strings.EqualFold(p, account) })
	if i < 0 {
		return "", fmt.Errorf("Messenger isn't linked to a profile %q", account)
	}
	profile := m.linked[i]
	m.linked = slices.Delete(m.linked, i, i+1)
	delete(m.out, profile)
	if b := m.browsers[profile]; b != nil {
		b.mu.Lock()
		if b.computer != "" {
			b.computers("give_back", map[string]any{"computer": b.computer})
		}
		b.mu.Unlock()
		delete(m.browsers, profile)
	}
	return fmt.Sprintf("Messenger no longer reads the %s profile. The profile and its other sign-ins stay, on the Agent Computers page.", profile), m.save()
}
