package main

import (
	"bytes"
	"cmp"
	"context"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"math/big"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
)

// config is what the Vault's Apple push item says: the Apple developer team,
// the APNs key's id, and the app's bundle id that pushes go to (its topic).
type config struct {
	TeamID string `json:"team_id"`
	KeyID  string `json:"key_id"`
	Topic  string `json:"topic"`
}

// apns is the host the key is filled in for: the item's sites are Apple's
// push hosts, and the token made with it goes only there.
const apns = "api.push.apple.com"

type sender struct {
	dir    string
	node   string
	client *http.Client

	mu    sync.Mutex
	token string
	made  time.Time
}

type device struct {
	Token    string `json:"token"`
	Platform string `json:"platform"`
	Env      string `json:"env"`
	// Whose phone it is, and what it's called, once registered here.
	User string `json:"user,omitempty"`
	Name string `json:"name,omitempty"`
	Time int64  `json:"time,omitempty"`
}

type sendInput struct {
	Devices []device       `json:"devices"`
	Title   string         `json:"title"`
	Body    string         `json:"body"`
	App     string         `json:"app"`
	Screen  string         `json:"screen"`
	Params  map[string]any `json:"params"`
	Link    map[string]any `json:"link"`
	Badge   *int           `json:"badge"`
}

var hexToken = regexp.MustCompile(`^[0-9a-f]{16,200}$`)

func (s *sender) run(ctx context.Context, c *call) (any, error) {
	input := c.Input
	switch c.Op {
	case "register":
		var d device
		if err := json.Unmarshal(input, &d); err != nil {
			return nil, err
		}
		d.Token = strings.ToLower(d.Token)
		if !hexToken.MatchString(d.Token) {
			return nil, errors.New("a device token is hex")
		}
		d.User, d.Time = c.Caller.User, time.Now().UnixMilli()
		d.Platform, d.Env, d.Name = cmp.Or(d.Platform, "ios"), cmp.Or(d.Env, "production"), cmp.Or(d.Name, c.Caller.Name)
		return "Registered for push.", s.change(func(all map[string]device) { all[d.Token] = d })
	case "unregister":
		var d device
		if err := json.Unmarshal(input, &d); err != nil {
			return nil, err
		}
		return "Unregistered.", s.change(func(all map[string]device) {
			if all[strings.ToLower(d.Token)].User == c.Caller.User {
				delete(all, strings.ToLower(d.Token))
			}
		})
	case "devices":
		mine := []map[string]any{}
		for _, d := range s.mine(c.Caller.User) {
			mine = append(mine, map[string]any{"token": d.Token[:8] + "…", "platform": d.Platform, "env": d.Env, "name": d.Name, "time": d.Time})
		}
		return mine, nil
	case "status":
		cfg, _, err := s.load(c.ID)
		if err != nil {
			return map[string]any{"ready": false, "why": err.Error()}, nil
		}
		return map[string]any{"ready": true, "team_id": cfg.TeamID, "key_id": cfg.KeyID, "topic": cfg.Topic}, nil
	case "send":
		var in sendInput
		if err := json.Unmarshal(input, &in); err != nil {
			return nil, err
		}
		if in.Title == "" {
			return nil, errors.New("give it a title")
		}
		// Given devices are the old way, from before phones registered here.
		registered := len(in.Devices) == 0
		if registered {
			in.Devices = s.mine(c.Caller.User)
		}
		if len(in.Devices) == 0 {
			return map[string]any{"accepted": 0, "devices": 0, "why": "no phone of yours is registered for push"}, nil
		}
		if !s.on() {
			return map[string]any{"accepted": 0, "devices": len(in.Devices), "why": "push is off in this install's settings"}, nil
		}
		out, err := s.send(ctx, c.ID, in, link(c.From, in))
		if err == nil && registered && len(out["gone"].([]string)) > 0 {
			gone := out["gone"].([]string)
			err = s.change(func(all map[string]device) {
				for _, t := range gone {
					delete(all, t)
				}
			})
		}
		return out, err
	}
	return nil, fmt.Errorf("push has no operation %s", c.Op)
}

// on says whether the install's push setting names this sidecar; an admin
// turns push off in Settings by emptying it.
func (s *sender) on() bool {
	resp, err := s.client.Get(s.node + "/v1/state")
	if err != nil {
		return true
	}
	defer resp.Body.Close()
	var state struct {
		Install *struct {
			Settings map[string]string `json:"settings"`
		} `json:"install"`
	}
	if json.NewDecoder(resp.Body).Decode(&state) != nil || state.Install == nil || state.Install.Settings == nil {
		return true
	}
	push, set := state.Install.Settings["push"]
	return !set || push == app
}

// link is what a tap opens: the screen of the app that sent the push (as the
// node says), or else the one the push names.
func link(from string, in sendInput) map[string]any {
	app, screen, params := cmp.Or(from, in.App), in.Screen, in.Params
	if in.Link != nil {
		l, _ := in.Link["app"].(string)
		sc, _ := in.Link["screen"].(string)
		p, _ := in.Link["params"].(map[string]any)
		app, screen = cmp.Or(app, l), cmp.Or(screen, sc)
		if params == nil {
			params = p
		}
	}
	if app == "" {
		return nil
	}
	if params == nil {
		params = map[string]any{}
	}
	return map[string]any{"app": app, "screen": cmp.Or(screen, "main"), "params": params}
}

// mine is a user's registered phones.
func (s *sender) mine(user string) []device {
	s.mu.Lock()
	defer s.mu.Unlock()
	var out []device
	for _, d := range s.devices() {
		if d.User == user {
			out = append(out, d)
		}
	}
	return out
}

// change changes the registered phones and keeps them in devices.json.
func (s *sender) change(f func(map[string]device)) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	all := s.devices()
	f(all)
	b, _ := json.MarshalIndent(all, "", "  ")
	tmp := filepath.Join(s.dir, "devices.json.new")
	if err := os.WriteFile(tmp, b, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, filepath.Join(s.dir, "devices.json"))
}

// devices reads devices.json; s.mu is held.
func (s *sender) devices() map[string]device {
	all := map[string]device{}
	b, _ := os.ReadFile(filepath.Join(s.dir, "devices.json"))
	_ = json.Unmarshal(b, &all)
	return all
}

// load reads the config and the APNs key from the Vault: the fields of an
// item switched on for push (on Everyone, or the caller's own), which the node
// fills in for this call, the key only for Apple's push hosts.
func (s *sender) load(call uint64) (config, *ecdsa.PrivateKey, error) {
	var c config
	notSet := errors.New("push isn't set up: an admin adds an item to the Vault on Everyone with team_id, key_id and topic, and key, the APNs key's text, secret and sent to api.push.apple.com and api.sandbox.push.apple.com, and switches push on for it")
	fields := map[string]*string{"team_id": &c.TeamID, "key_id": &c.KeyID, "topic": &c.Topic}
	var p8 string
	fields["key"] = &p8
	for name, to := range fields {
		v, err := s.field(call, name)
		if err != nil {
			return c, nil, err
		}
		if v == "" {
			return c, nil, notSet
		}
		*to = v
	}
	block, _ := pem.Decode([]byte(p8))
	if block == nil {
		return c, nil, errors.New("the Vault's APNs key isn't a PEM key")
	}
	k, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return c, nil, fmt.Errorf("the Vault's APNs key: %w", err)
	}
	key, ok := k.(*ecdsa.PrivateKey)
	if !ok {
		return c, nil, errors.New("the Vault's APNs key isn't an ES256 key")
	}
	return c, key, nil
}

// field is one of the Vault item's fields, as the node fills it in for this
// call; empty when no item has it.
func (s *sender) field(call uint64, name string) (string, error) {
	placeholder := "{{" + name + "}}"
	body, _ := json.Marshal(map[string]string{"text": placeholder, "host": apns})
	resp, err := s.client.Post(fmt.Sprintf("%s/v1/sidecars/%s/calls/%d/vault", s.node, app, call), "application/json", bytes.NewReader(body))
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("the Vault: %s", b)
	}
	var out struct{ Text string }
	if err := json.Unmarshal(b, &out); err != nil {
		return "", err
	}
	if out.Text == placeholder {
		return "", nil
	}
	return out.Text, nil
}

// jwt is the provider token APNs takes, made again every 50 minutes (it
// refuses one older than an hour, and one made more often than every 20).
func (s *sender) jwt(c config, key *ecdsa.PrivateKey) (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.token != "" && time.Since(s.made) < 50*time.Minute {
		return s.token, nil
	}
	enc := base64.RawURLEncoding
	header, _ := json.Marshal(map[string]string{"alg": "ES256", "kid": c.KeyID})
	claims, _ := json.Marshal(map[string]any{"iss": c.TeamID, "iat": time.Now().Unix()})
	signing := enc.EncodeToString(header) + "." + enc.EncodeToString(claims)
	sum := sha256.Sum256([]byte(signing))
	r, sg, err := ecdsa.Sign(rand.Reader, key, sum[:])
	if err != nil {
		return "", err
	}
	sig := append(pad(r), pad(sg)...)
	s.token, s.made = signing+"."+enc.EncodeToString(sig), time.Now()
	return s.token, nil
}

func pad(n *big.Int) []byte {
	b := n.Bytes()
	return append(make([]byte, 32-len(b)), b...)
}

// send pushes to each iPhone or iPad, and says how many APNs accepted and
// which tokens it said no longer work (the app was deleted, say).
func (s *sender) send(ctx context.Context, call uint64, in sendInput, link map[string]any) (map[string]any, error) {
	c, key, err := s.load(call)
	if err != nil {
		return nil, err
	}
	token, err := s.jwt(c, key)
	if err != nil {
		return nil, err
	}
	aps := map[string]any{"alert": map[string]string{"title": in.Title, "body": in.Body}, "sound": "default"}
	if in.Badge != nil {
		aps["badge"] = *in.Badge
	}
	payload, _ := json.Marshal(map[string]any{"aps": aps, "link": link})
	accepted, gone, why := 0, []string{}, ""
	for _, d := range in.Devices {
		if d.Platform != "" && d.Platform != "ios" {
			continue
		}
		host := "https://api.push.apple.com"
		if d.Env == "sandbox" || d.Env == "development" {
			host = "https://api.sandbox.push.apple.com"
		}
		req, _ := http.NewRequestWithContext(ctx, "POST", host+"/3/device/"+d.Token, bytes.NewReader(payload))
		req.Header.Set("authorization", "bearer "+token)
		req.Header.Set("apns-topic", c.Topic)
		req.Header.Set("apns-push-type", "alert")
		req.Header.Set("apns-priority", "10")
		resp, err := s.client.Do(req)
		if err != nil {
			why = err.Error()
			continue
		}
		body, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		var reason struct{ Reason string }
		_ = json.Unmarshal(body, &reason)
		switch {
		case resp.StatusCode == http.StatusOK:
			accepted++
		case resp.StatusCode == http.StatusGone || reason.Reason == "BadDeviceToken" || reason.Reason == "Unregistered":
			gone = append(gone, d.Token)
		default:
			why = fmt.Sprintf("APNs said %d %s", resp.StatusCode, reason.Reason)
		}
	}
	out := map[string]any{"accepted": accepted, "devices": len(in.Devices), "gone": gone}
	if why != "" {
		out["why"] = why
	}
	return out, nil
}
