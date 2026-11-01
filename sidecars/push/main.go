// The push sidecar: sends notifications to phones through Apple's push
// service (APNs), for every user its server serves, so an app or an agent can
// reach someone who isn't looking. Phones register their tokens with it; a push
// goes to the caller's own phones, says which app sent it, and opens that app's
// screen when tapped. It runs on a privileged server, and the edition's APNs
// key is a Vault item on Everyone switched on for push, which the node fills in
// for each call (see load); its folder keeps devices.json, the phones
// registered. It says whether APNs accepted a push, never whether anyone saw it.
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
	ID     uint64          `json:"id"`
	Op     string          `json:"op"`
	Input  json.RawMessage `json:"input"`
	Caller struct {
		User string `json:"user"`
		Name string `json:"user_name"`
	} `json:"caller"`
	// The app whose ctx.sidecars made the call, as the node says.
	From string `json:"from"`
}

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("PUSH_DATA"), filepath.Join(home, ".local", "share", "one", "push"))
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	serve(node, &sender{dir: dir, node: node, client: &http.Client{Timeout: 15 * time.Second}})
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps sidecars in memory) or restarts.
func serve(node string, s *sender) {
	poll := &http.Client{Timeout: 40 * time.Second}
	for {
		if err := post(node+"/v1/sidecars/"+app, describe); err != nil {
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
				result, err := s.run(context.Background(), c)
				reply := map[string]any{"ok": result}
				if err != nil {
					reply = map[string]any{"error": err.Error()}
				}
				if err := post(fmt.Sprintf("%s/v1/sidecars/%s/calls/%d", node, app, c.ID), reply); err != nil {
					log.Printf("answering %s: %v", c.Op, err)
				}
			}()
		}
	}
}

// next waits for the node's next call; nil if there was none.
func next(client *http.Client, node string) (*call, error) {
	resp, err := client.Get(node + "/v1/sidecars/" + app + "/calls")
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
	"description":  "Sends notifications to your phone",
	"version":      built,
	"icon":         "bell.badge",
	"serves":       "everyone",
	"instructions": "Push sends a notification to your own phones through Apple's push service: push_send with a title and a body. An app pushes with ctx.sidecars.push.send, and tapping its push opens its screen. It says whether APNs accepted it, never whether anyone saw it.",
	"ops": []map[string]any{
		{"name": "send", "description": "Push a notification to your phones: { title, body, screen?, params? }; from an app, tapping it opens that app's screen (main unless given). Says how many APNs accepted.",
			"input": object(map[string]any{
				"title":   map[string]any{"type": "string"},
				"body":    map[string]any{"type": "string"},
				"app":     map[string]any{"type": "string", "description": "the app it opens, when no app sends it"},
				"screen":  map[string]any{"type": "string", "description": "the screen it opens"},
				"params":  map[string]any{"type": "object", "description": "that screen's params"},
				"badge":   map[string]any{"type": "integer"},
				"devices": map[string]any{"type": "array", "items": map[string]any{"type": "object"}, "description": "only these devices, [{ token, platform, env }], not your registered phones"},
				"link":    map[string]any{"type": "object", "description": "{ app, screen, params }, as app, screen and params"},
			}, "title")},
		{"name": "register", "description": "A phone registers for push: its APNs device token (hex), platform ios, env sandbox or production, and a name.",
			"input": object(map[string]any{
				"token":    map[string]any{"type": "string"},
				"platform": map[string]any{"type": "string"},
				"env":      map[string]any{"type": "string"},
				"name":     map[string]any{"type": "string"},
			}, "token")},
		{"name": "unregister", "description": "A phone stops getting pushes.",
			"input": object(map[string]any{"token": map[string]any{"type": "string"}}, "token")},
		{"name": "devices", "reads": true, "description": "Your phones registered for push here.",
			"input": object(map[string]any{})},
		{"name": "status", "reads": true, "description": "Whether push is set up here: the APNs team, key id and app it sends for, never the key.",
			"input": object(map[string]any{})},
	},
}
