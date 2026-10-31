// The push provider: sends notifications to phones through Apple's push
// service (APNs), for every user its server serves, so the inbox app can reach
// someone who isn't looking. It runs on a server whose owners trust it with
// the brand's APNs key, a privileged one, and keeps that key in a file only its
// user can read: AuthKey.p8 beside config.json { team_id, key_id, topic } in
// its folder. It says whether APNs accepted a push, never whether anyone saw it.
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

const app = "push"

type call struct {
	ID    uint64          `json:"id"`
	Op    string          `json:"op"`
	Input json.RawMessage `json:"input"`
}

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("PUSH_DATA"), filepath.Join(home, ".local", "share", "one", "push"))
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	serve(node, &sender{dir: dir, client: &http.Client{Timeout: 15 * time.Second}})
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps providers in memory) or restarts.
func serve(node string, s *sender) {
	poll := &http.Client{Timeout: 40 * time.Second}
	for {
		if err := post(node+"/v1/providers/"+app, describe); err != nil {
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
			go func() {
				result, err := s.run(context.Background(), c.Op, c.Input)
				reply := map[string]any{"ok": result}
				if err != nil {
					reply = map[string]any{"error": err.Error()}
				}
				if err := post(fmt.Sprintf("%s/v1/providers/%s/calls/%d", node, app, c.ID), reply); err != nil {
					log.Printf("answering %s: %v", c.Op, err)
				}
			}()
		}
	}
}

// next waits for the node's next call; nil if there was none.
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

func post(url string, body any) error {
	b, err := json.Marshal(body)
	if err != nil {
		return err
	}
	resp, err := http.Post(url, "application/json", bytes.NewReader(b))
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		msg, _ := io.ReadAll(resp.Body)
		return errors.New(resp.Status + ": " + string(msg))
	}
	return nil
}

func object(props map[string]any, required ...string) map[string]any {
	o := map[string]any{"type": "object", "properties": props}
	if len(required) > 0 {
		o["required"] = required
	}
	return o
}

var describe = map[string]any{
	"title":        "Push",
	"icon":         "bell.badge",
	"serves":       "everyone",
	"instructions": "Push sends notifications to phones through Apple's push service. Apps reach it through the inbox app (inbox_notify), not directly.",
	"ops": []map[string]any{
		{"name": "send", "description": "Push a notification to devices: { devices: [{ token, platform, env }], title, body, link }. Says how many APNs accepted, and which tokens it said are gone.",
			"input": object(map[string]any{
				"devices": map[string]any{"type": "array", "items": map[string]any{"type": "object"}},
				"title":   map[string]any{"type": "string"},
				"body":    map[string]any{"type": "string"},
				"link":    map[string]any{"type": "object"},
				"badge":   map[string]any{"type": "integer"},
			}, "devices", "title")},
		{"name": "status", "reads": true, "description": "Whether push is set up here: the APNs team, key id and app it sends for, never the key.",
			"input": object(map[string]any{})},
	},
}
