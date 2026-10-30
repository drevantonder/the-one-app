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
	w, err := open(filepath.Join(dir, "whatsapp.db"))
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

var describe = map[string]any{
	"title": "WhatsApp",
	"icon":  "message-circle",
	"instructions": "WhatsApp runs on the device linked to the account (give device). whatsapp_chats lists recent chats; " +
		"whatsapp_read reads one, newest last; whatsapp_search finds messages. whatsapp_send sends a real message to a real person: " +
		"send only what the user asked to send, to whom they asked. Reading never tells WhatsApp a message was read.",
	"ops": []map[string]any{
		{"name": "status", "description": "Whether WhatsApp is linked and connected here, as which number, and how many messages are kept.",
			"input": object(map[string]any{})},
		{"name": "sign_in", "description": "Link WhatsApp here, as WhatsApp Desktop is linked: returns a QR code to scan on the phone (WhatsApp > Settings > Linked devices > Link a device), or, given phone, a code to enter there instead.",
			"input": object(map[string]any{"phone": str("the account's number with its country code, e.g. +61 400 000 000, to link with a code instead of a QR code")})},
		{"name": "chats", "description": "Recent WhatsApp chats, most recent first: who, when, the last message, and how many are unread.",
			"input": object(map[string]any{"unread": map[string]any{"type": "boolean", "description": "only unread chats"},
				"limit": num("at most this many; default 30"), "skip": num("skip this many of the most recent, to page back")})},
		{"name": "read", "description": "A WhatsApp chat's messages, newest last. Page back with skip.",
			"input": object(map[string]any{"chat": str("the chat's name, a part of it only one chat has, a phone number, or a JID"),
				"limit": num("this many; default 30"), "skip": num("skip this many of the newest, to read further back")}, "chat")},
		{"name": "search", "description": "WhatsApp messages containing some text, across chats or in one, newest last.",
			"input": object(map[string]any{"query": str("text to look for, case-insensitive"), "chat": str("only in this chat"),
				"limit": num("at most this many; default 30"), "skip": num("skip this many of the newest matches, to page back")}, "query")},
		{"name": "send", "description": "Send a WhatsApp message, as the linked account, to a chat, a contact or a phone number. This sends a real message to a real person.",
			"input": object(map[string]any{"chat": str("the chat's name, a part of it only one chat has, a contact's exact name, a phone number with its country code, or a JID"),
				"text": str("the message")}, "chat", "text")},
	},
	"widgets": []any{},
}
