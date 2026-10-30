// The Gmail provider: offers the person's Gmail accounts to their devices
// through this device's node (SPEC.md, "Local API"). It keeps each account's
// OAuth refresh token in a file only its user can read.
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

const app = "gmail"

type call struct {
	ID    uint64          `json:"id"`
	Op    string          `json:"op"`
	Input json.RawMessage `json:"input"`
}

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("GMAIL_DATA"), filepath.Join(home, ".local", "share", "one", "gmail"))
	addr := cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	redirect = "http://" + addr + "/hooks/gmail/oauth"
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	a, err := load(filepath.Join(dir, "accounts.json"))
	if err != nil {
		log.Fatal(err)
	}
	serve("http://"+addr, &gmail{a})
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps providers in memory) or restarts.
func serve(node string, g *gmail) {
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
				result, err := g.run(context.Background(), c.Op, c.Input)
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

func str(description string) map[string]any {
	return map[string]any{"type": "string", "description": description}
}

func num(description string) map[string]any {
	return map[string]any{"type": "integer", "description": description}
}

func list(description string) map[string]any {
	return map[string]any{"type": "array", "items": map[string]any{"type": "string"}, "description": description}
}

// with adds the account every Gmail operation takes.
func with(props map[string]any, required ...string) map[string]any {
	props["account"] = str("which account: its address or a part of it only it has; default the first, as gmail_accounts lists them")
	return object(props, required...)
}

var writing = map[string]any{
	"to": str("addresses, comma-separated; a reply goes to the sender unless given"), "cc": str("addresses, comma-separated"),
	"bcc": str("addresses, comma-separated"), "subject": str("the subject; a reply keeps the original's unless given"),
	"body": str("the text of the email"), "reply_to": str("the id of a message to reply to, keeping the reply in its thread"),
}

var describe = map[string]any{
	"title": "Gmail",
	"icon":  "mail",
	"instructions": "Gmail is the person's Gmail accounts, served by the device signed in to them. gmail_search finds messages with Gmail's search syntax " +
		"(from:, is:unread, in:inbox, newer_than:2d, ...); gmail_read reads one and gmail_thread a whole conversation. gmail_label archives " +
		"(remove INBOX), marks read (remove UNREAD), stars or labels messages. Write with gmail_draft and let the person send it, unless they asked you to send: " +
		"gmail_send and gmail_send_draft send real email to real people. With several accounts, each operation takes account; gmail_accounts lists them.",
	"ops": []map[string]any{
		{"name": "accounts", "description": "The Gmail accounts signed in here, and whether each still works. The first is the default.",
			"input": object(map[string]any{})},
		{"name": "sign_in", "description": "Sign in to a Gmail account, or another one: returns a link to open in a browser; then pass the address the browser ends up on as code.",
			"input": object(map[string]any{"code": str("the address the browser was sent to after signing in (or just its code)"),
				"client_id": str("an OAuth client id, only to replace the one kept here"), "client_secret": str("that client's secret")})},
		{"name": "sign_out", "description": "Forget a Gmail account here.",
			"input": object(map[string]any{"account": str("the account's address, or a part of it only it has")}, "account")},
		{"name": "profile", "description": "The account's address and how many messages and threads it has.",
			"input": with(map[string]any{})},
		{"name": "search", "description": "Messages matching a Gmail search, newest first: id, date, sender, subject, labels and a snippet. With no query, all mail but spam and trash.",
			"input": with(map[string]any{"query": str("a Gmail search, as in Gmail's search box, e.g. in:inbox is:unread from:someone@example.com"),
				"limit": num("at most this many; default 20, at most 100"), "page": str("the page token the previous search gave, for the next page")})},
		{"name": "read", "description": "One message: its headers, labels, attachments and plain-text body.",
			"input": with(map[string]any{"id": str("the message's id, as search gives it")}, "id")},
		{"name": "thread", "description": "A whole conversation, oldest first, each message as read gives it.",
			"input": with(map[string]any{"id": str("the thread's id, or the id of any message in it")}, "id")},
		{"name": "labels", "description": "The account's labels, Gmail's own first.",
			"input": with(map[string]any{})},
		{"name": "label", "description": "Add labels to and remove labels from messages. Archive: remove INBOX. Mark read: remove UNREAD. Star: add STARRED.",
			"input": with(map[string]any{"ids": list("the messages' ids"), "add": list("label names or ids to add"),
				"remove": list("label names or ids to remove")}, "ids")},
		{"name": "trash", "description": "Move messages to the trash, where Gmail deletes them after 30 days.",
			"input": with(map[string]any{"ids": list("the messages' ids")}, "ids")},
		{"name": "drafts", "description": "The account's drafts, newest first: draft id, date, recipients and subject.",
			"input": with(map[string]any{"limit": num("at most this many; default 20"), "page": str("the page token the previous call gave")})},
		{"name": "draft", "description": "Save a plain-text draft, or replace one, for the person to review and send.",
			"input": with(merge(writing, map[string]any{"id": str("a draft's id, to replace that draft")}))},
		{"name": "send_draft", "description": "Send a draft as it is. This sends real email to real people.",
			"input": with(map[string]any{"id": str("the draft's id")}, "id")},
		{"name": "delete_draft", "description": "Delete a draft for good.",
			"input": with(map[string]any{"id": str("the draft's id")}, "id")},
		{"name": "send", "description": "Send a plain-text email, or a reply. This sends real email to real people: send only what the person asked to send.",
			"input": with(merge(writing, nil))},
	},
	"widgets": []any{},
}

func merge(a, b map[string]any) map[string]any {
	out := map[string]any{}
	for k, v := range a {
		out[k] = v
	}
	for k, v := range b {
		out[k] = v
	}
	return out
}
