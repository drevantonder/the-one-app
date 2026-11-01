// The Messenger provider: Facebook Messenger, read and sent through the
// browser of a desktop borrowed from the desktops driver, with a copy of a
// browser profile signed in to messenger.com: one per account. Messenger has
// no API for personal accounts and its chats are end-to-end encrypted, so
// messenger.com in a browser is the way in.
package main

import (
	"bytes"
	"cmp"
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

const app = "messenger"

type call struct {
	ID    uint64          `json:"id"`
	Op    string          `json:"op"`
	Input json.RawMessage `json:"input"`
}

func main() {
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	home, _ := os.UserHomeDir()
	m, err := load(node, cmp.Or(os.Getenv("MESSENGER_DATA"), filepath.Join(home, ".local", "share", "one", "messenger")))
	if err != nil {
		log.Fatal(err)
	}
	go m.keepGivingBack()
	poll := &http.Client{Timeout: 40 * time.Second}
	for {
		if err := post(node+"/v1/providers/"+app, describe()); err != nil {
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
				result, err := m.run(c.Op, c.Input)
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

func describe() map[string]any {
	chat := map[string]any{"type": "string", "description": "the chat's name, or a part of it only one recent chat has"}
	account := map[string]any{"type": "string", "description": "which account: the browser profile, as messenger_accounts lists them; default the first"}
	withAccount := func(props map[string]any, required ...string) map[string]any {
		props["account"] = account
		return object(props, required...)
	}
	return map[string]any{
		"title":       "Messenger",
		"description": "Reads and sends your Messenger chats in a desktop's browser",
		"version":     built,
		"icon":        "message.fill",
		"color":       "#0866ff",
		"domain":      "messenger.com",
		"instructions": "Messenger is the person's Facebook Messenger, read through messenger.com in a desktop's browser. Each account is a browser profile signed in to it (Personal, PTC), as messenger_accounts lists them; the others take account, the first by default. " +
			"messenger_chats lists recent chats, messenger_read reads one, messenger_search finds messages. Each takes some seconds: a browser does the reading. " +
			"If Messenger isn't signed in, messenger_sign_in gives the person a link to sign in; never sign in for them. " +
			"messenger_send sends a real message to a real person: only when the person asked, with the exact text they approved.",
		"ops": []map[string]any{
			{"name": "accounts", "reads": true, "description": "The browser profiles Messenger reads, the default first: each { account, ok }, with why when ok is false and it needs signing in again.",
				"input": object(map[string]any{})},
			{"name": "chats", "reads": true, "description": "Recent Messenger chats, newest first: who, when, their last message, and whether unread.",
				"input": withAccount(map[string]any{"unread": map[string]any{"type": "boolean", "description": "only unread chats"}})},
			{"name": "read", "description": "A recent chat's latest messages. Opening a chat marks it read on Messenger, so one that was unread is marked unread again afterwards; the sender may still see it as seen.",
				"input": withAccount(map[string]any{"chat": chat}, "chat")},
			{"name": "search", "reads": true, "description": "Messages containing some words, across every chat: which chats have them, and the message when a chat has one.",
				"input": withAccount(map[string]any{"query": map[string]any{"type": "string", "description": "words to find"}}, "query")},
			{"name": "send", "confirm": true, "description": "Send a real message, as the person, to a recent chat or to someone by their exact name. It can't be taken back: only when the person asked, with the text they approved.",
				"input": withAccount(map[string]any{"chat": map[string]any{"type": "string", "description": "the chat's name, or a part of it only one recent chat has; anyone else by their exact name"},
					"text": map[string]any{"type": "string", "description": "the message"}}, "chat", "text")},
			{"name": "sign_in", "description": "For the person: links Messenger to a browser profile (made if new) and gives a link to messenger.com in it, to sign in there themselves. The default profile's again when none is given.",
				"input": object(map[string]any{"profile": map[string]any{"type": "string", "description": "the browser profile, like Personal or PTC; computer_profiles lists them"}})},
			{"name": "sign_out", "description": "Stop reading a browser profile's Messenger. The profile, and its sign-ins to other sites, stay.",
				"input": object(map[string]any{"account": account}, "account")},
		},
		"screens": []map[string]any{{"name": "settings", "title": "Messenger"}},
		"widgets": []any{},
	}
}

func object(props map[string]any, required ...string) map[string]any {
	o := map[string]any{"type": "object", "properties": props}
	if len(required) > 0 {
		o["required"] = required
	}
	return o
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
