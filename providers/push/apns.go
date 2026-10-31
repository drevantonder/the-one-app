package main

import (
	"bytes"
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
	"sync"
	"time"
)

// config is what the push folder's config.json says: the Apple developer team,
// the APNs key's id, and the app's bundle id that pushes go to (its topic).
type config struct {
	TeamID string `json:"team_id"`
	KeyID  string `json:"key_id"`
	Topic  string `json:"topic"`
}

type sender struct {
	dir    string
	client *http.Client

	mu    sync.Mutex
	token string
	made  time.Time
}

type device struct {
	Token    string `json:"token"`
	Platform string `json:"platform"`
	Env      string `json:"env"`
}

type sendInput struct {
	Devices []device       `json:"devices"`
	Title   string         `json:"title"`
	Body    string         `json:"body"`
	Link    map[string]any `json:"link"`
	Badge   *int           `json:"badge"`
}

func (s *sender) run(ctx context.Context, op string, input json.RawMessage) (any, error) {
	switch op {
	case "status":
		c, _, err := s.load()
		if err != nil {
			return map[string]any{"ready": false, "why": err.Error()}, nil
		}
		return map[string]any{"ready": true, "team_id": c.TeamID, "key_id": c.KeyID, "topic": c.Topic}, nil
	case "send":
		var in sendInput
		if err := json.Unmarshal(input, &in); err != nil {
			return nil, err
		}
		return s.send(ctx, in)
	}
	return nil, fmt.Errorf("push has no operation %s", op)
}

// load reads the config and the APNs key.
func (s *sender) load() (config, *ecdsa.PrivateKey, error) {
	var c config
	notSet := fmt.Errorf("push isn't set up here: put the APNs key in %s and config.json { team_id, key_id, topic } beside it", filepath.Join(s.dir, "AuthKey.p8"))
	b, err := os.ReadFile(filepath.Join(s.dir, "config.json"))
	if err != nil {
		return c, nil, notSet
	}
	if err := json.Unmarshal(b, &c); err != nil || c.TeamID == "" || c.KeyID == "" || c.Topic == "" {
		return c, nil, notSet
	}
	p8, err := os.ReadFile(filepath.Join(s.dir, "AuthKey.p8"))
	if err != nil {
		return c, nil, notSet
	}
	block, _ := pem.Decode(p8)
	if block == nil {
		return c, nil, errors.New("AuthKey.p8 isn't a PEM key")
	}
	k, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return c, nil, fmt.Errorf("AuthKey.p8: %w", err)
	}
	key, ok := k.(*ecdsa.PrivateKey)
	if !ok {
		return c, nil, errors.New("AuthKey.p8 isn't an ES256 key")
	}
	return c, key, nil
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
func (s *sender) send(ctx context.Context, in sendInput) (any, error) {
	c, key, err := s.load()
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
	payload, _ := json.Marshal(map[string]any{"aps": aps, "link": in.Link})
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
	out := map[string]any{"accepted": accepted, "gone": gone}
	if why != "" {
		out["why"] = why
	}
	return out, nil
}
