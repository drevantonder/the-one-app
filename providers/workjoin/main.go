// The work-email join driver: lets people join the install by signing in with
// Google, limited to one domain (a workplace's), instead of an admin handing
// each of them a pairing code. It runs on a privileged server whose admins
// trust it: it makes new users' pairing codes through that server's local API,
// as its local user, an admin. Nothing in the node knows it exists.
//
// It's off until an admin puts config.json in its folder:
//
//	{ "domain": "example.org", "client_id": "…apps.googleusercontent.com",
//	  "client_secret": "…", "public_url": "https://join.example.org",
//	  "listen": "127.0.0.1:8790", "group": "<group id to invite them to>" }
//
// and serves listen at public_url (a reverse proxy, Tailscale Funnel). The
// Google OAuth client's redirect URI is public_url + "/callback". A brand
// whose shells sign people in has public_url as brand.toml's `sign_in`: the
// shells open public_url + "/start?app=1", which ends at <cli>://join.
package main

import (
	"bytes"
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

const app = "workjoin"

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("WORKJOIN_DATA"), filepath.Join(home, ".local", "share", "one", "workjoin"))
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	j := &joiner{dir: dir, node: node}
	// It serves even while it's off, so the app's sign-in says so; config.json is read on each request.
	c, err := j.config()
	if err != nil {
		log.Printf("off: %v", err)
	}
	go func() {
		listen := cmpOr(c.Listen, "127.0.0.1:8790")
		log.Printf("serving %s for @%s at %s", listen, c.Domain, c.PublicURL)
		log.Fatal(http.ListenAndServe(listen, j.routes()))
	}()
	serve(node, j)
}

type call struct {
	ID    uint64          `json:"id"`
	Op    string          `json:"op"`
	Input json.RawMessage `json:"input"`
}

// serve registers with the node, so the install can see it's there and ask its
// status, and answers its calls.
func serve(node string, j *joiner) {
	poll := &http.Client{Timeout: 40 * time.Second}
	for {
		if err := post(node+"/v1/providers/"+app, describe, nil); err != nil {
			log.Printf("registering: %v", err)
			time.Sleep(3 * time.Second)
			continue
		}
		for {
			c, err := next(poll, node)
			if err != nil {
				log.Printf("waiting for calls: %v", err)
				time.Sleep(time.Second)
				break
			}
			if c == nil {
				continue
			}
			result, err := j.run(context.Background(), c.Op)
			reply := map[string]any{"ok": result}
			if err != nil {
				reply = map[string]any{"error": err.Error()}
			}
			if err := post(fmt.Sprintf("%s/v1/providers/%s/calls/%d", node, app, c.ID), reply, nil); err != nil {
				log.Printf("answering %s: %v", c.Op, err)
			}
		}
	}
}

func next(client *http.Client, node string) (*call, error) {
	resp, err := client.Get(node + "/v1/providers/" + app + "/calls")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	switch resp.StatusCode {
	case http.StatusNoContent:
		return nil, nil
	case http.StatusOK:
		var c call
		return &c, json.Unmarshal(body, &c)
	}
	return nil, fmt.Errorf("%s: %s", resp.Status, body)
}

// post sends JSON and, given out, reads the JSON answer into it.
func post(url string, body, out any) error {
	b, err := json.Marshal(body)
	if err != nil {
		return err
	}
	resp, err := http.Post(url, "application/json", bytes.NewReader(b))
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	answer, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		var e struct{ Error string }
		if json.Unmarshal(answer, &e) == nil && e.Error != "" {
			return errors.New(e.Error)
		}
		return errors.New(resp.Status + ": " + string(answer))
	}
	if out != nil {
		return json.Unmarshal(answer, out)
	}
	return nil
}

var describe = map[string]any{
	"title":        "Work email join",
	"icon":         "person.badge.key",
	"instructions": "Lets people join this install by signing in with Google on the workplace's domain. workjoin_status says whether it's on, for which domain, and who joined through it.",
	"ops": []map[string]any{
		{"name": "status", "reads": true, "description": "Whether joining with a work email is on here, for which domain and at which address, and who has joined through it.",
			"input": map[string]any{"type": "object", "properties": map[string]any{}}},
	},
}
