// The WhatsApp provider: links this device to a WhatsApp account, as
// WhatsApp Desktop is linked, keeps the messages it sees, and offers them to
// the person's devices through this device's node (SPEC.md, "Local API").
//
// It never tells WhatsApp the user is online or has read a message, so the
// phone keeps notifying and senders don't see reads.
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

const app = "whatsapp"

type call struct {
	ID    uint64          `json:"id"`
	Op    string          `json:"op"`
	Input json.RawMessage `json:"input"`
}

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("WHATSAPP_DATA"), filepath.Join(home, ".local", "share", "one", "whatsapp"))
	node := "http://" + cmp.Or(os.Getenv("ONE_ADDR"), "127.0.0.1:4747")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	w, err := open(filepath.Join(dir, "whatsapp.db"), node)
	if err != nil {
		log.Fatal(err)
	}
	serve(node, w)
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps providers in memory) or restarts.
func serve(node string, w *link) {
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
				result, err := w.run(context.Background(), c.Op, c.Input)
				reply := map[string]any{"ok": result}
				var away elsewhere
				switch {
				case errors.As(err, &away): // error too, for nodes that don't know elsewhere
					reply = map[string]any{"elsewhere": err.Error(), "error": err.Error()}
				case err != nil:
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

// elsewhere is an error saying the call should go to another server, as
// when WhatsApp isn't linked here; the node then tries the next.
type elsewhere string

func (e elsewhere) Error() string { return string(e) }

func post(url string, body any) error {
	_, err := request(http.DefaultClient, url, body)
	return err
}

// request posts body as JSON, or gets url if body is nil, and returns the
// reply's body.
func request(client *http.Client, url string, body any) ([]byte, error) {
	var resp *http.Response
	var err error
	if body == nil {
		resp, err = client.Get(url)
	} else {
		b, merr := json.Marshal(body)
		if merr != nil {
			return nil, merr
		}
		resp, err = client.Post(url, "application/json", bytes.NewReader(b))
	}
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	msg, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		var e struct{ Error string }
		if json.Unmarshal(msg, &e) == nil && e.Error != "" {
			return nil, errors.New(e.Error)
		}
		return nil, errors.New(resp.Status + ": " + string(msg))
	}
	return msg, nil
}

// callOn runs one of this provider's operations on another of the user's
// servers, through this server's node, and decodes its result into out.
func callOn(node, server, op string, input, out any) error {
	body, err := request(&http.Client{Timeout: 28 * time.Second}, node+"/v1/call",
		map[string]any{"app": app, "op": op, "input": input, "server": server})
	if err != nil {
		return err
	}
	var reply struct{ Ok json.RawMessage }
	if err := json.Unmarshal(body, &reply); err != nil {
		return err
	}
	return json.Unmarshal(reply.Ok, out)
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

var describe = map[string]any{
	"title": "WhatsApp",
	"icon":  "message-circle",
	"instructions": "WhatsApp runs on the server linked to the account; calls find it. whatsapp_chats lists recent chats; " +
		"whatsapp_read reads one, newest last; whatsapp_search finds messages. whatsapp_send sends a real message to a real person: " +
		"send only what the user asked to send, to whom they asked. Reading never tells WhatsApp a message was read. " +
		"whatsapp_move moves the link to another server; whatsapp_sign_out unlinks it and deletes its messages, only when the user asks.",
	"ops": []map[string]any{
		{"name": "status", "reads": true, "description": "Whether WhatsApp is linked and connected here, as which number, and how many messages are kept; or on which of your servers it's linked.",
			"input": object(map[string]any{})},
		{"name": "sign_in", "description": "Link WhatsApp here, as WhatsApp Desktop is linked: returns a QR code to scan on the phone (WhatsApp > Settings > Linked devices > Link a device) and, given phone, a code to type there instead (Link with phone number instead), for when the QR code is on the phone itself. " +
			"Refused while WhatsApp is linked on another of your servers: whatsapp_move moves it from there.",
			"input": object(map[string]any{"phone": str("the account's number with its country code, e.g. +61 400 000 000, for a code to type on the phone")})},
		{"name": "chats", "reads": true, "description": "Recent WhatsApp chats, most recent first: who, when, the last message, and how many are unread.",
			"input": object(map[string]any{"unread": map[string]any{"type": "boolean", "description": "only unread chats"},
				"limit": num("at most this many; default 30"), "skip": num("skip this many of the most recent, to page back")})},
		{"name": "read", "reads": true, "description": "A WhatsApp chat's messages, newest last. Page back with skip. WhatsApp isn't told they were read: the sender sees no blue ticks, and the chat stays unread on the phone.",
			"input": object(map[string]any{"chat": str("the chat's name, a part of it only one chat has, a phone number, or a JID"),
				"limit": num("this many; default 30"), "skip": num("skip this many of the newest, to read further back")}, "chat")},
		{"name": "search", "reads": true, "description": "WhatsApp messages containing some text, across chats or in one, newest last.",
			"input": object(map[string]any{"query": str("text to look for, case-insensitive"), "chat": str("only in this chat"),
				"limit": num("at most this many; default 30"), "skip": num("skip this many of the newest matches, to page back")}, "query")},
		{"name": "send", "description": "Send a WhatsApp message, as the linked account, to a chat, a contact or a phone number. This sends a real message to a real person.",
			"input": object(map[string]any{"chat": str("the chat's name, a part of it only one chat has, a contact's exact name, a phone number with its country code, or a JID"),
				"text": str("the message")}, "chat", "text")},
		{"name": "move", "description": "Move the WhatsApp link, with its kept messages, from the server it's on to another of yours that runs WhatsApp, without linking again: the phone's Linked devices list doesn't change. It's deleted here once the other server has it.",
			"input": object(map[string]any{"to": str("the server to move it to, by name")}, "to")},
		{"name": "sign_out", "confirm": true, "description": "Unlink WhatsApp: removes this server from the account's linked devices and deletes the messages kept here. Destructive: linking again needs the phone, and the deleted messages don't come back. Only when the user asks to sign out.",
			"input": object(map[string]any{})},
	},
	"widgets": []any{},
}
