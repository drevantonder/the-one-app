// Package apple holds the built-in plugins for Apple apps: reminders,
// calendar, contacts and imessage. They run on macOS, inside TheOneApp.app,
// through small Swift helpers; other devices reach them through the mesh.
package apple

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"github.com/drevantonder/the-one-app/sdk"
)

// A helper is a small Swift program for one Apple framework, built into
// TheOneApp.app next to the one binary (see macos/build.sh). Running inside
// the signed app means macOS asks for each permission once, as The One App.
type helper string

var (
	eventkit helper = "eventkit"
	contacts helper = "contacts"
	messages helper = "messages"
)

func (h helper) path() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	if resolved, err := filepath.EvalSymlinks(exe); err == nil {
		exe = resolved
	}
	p := filepath.Join(filepath.Dir(exe), "one-"+string(h))
	if _, err := os.Stat(p); err != nil {
		return "", errors.New("this plugin runs inside TheOneApp.app; build and install it with macos/build.sh install")
	}
	return p, nil
}

// run calls a helper with a command and JSON input.
func (h helper) run(ctx context.Context, command string, input any) (sdk.Result, error) {
	path, err := h.path()
	if err != nil {
		return sdk.Result{}, err
	}
	in, _ := json.Marshal(input)
	cmd := exec.CommandContext(ctx, path, command)
	cmd.Stdin = bytes.NewReader(in)
	var out bytes.Buffer
	cmd.Stdout = &out
	_ = cmd.Run() // the helper reports errors as JSON
	var res struct {
		Result any    `json:"result"`
		Error  string `json:"error"`
	}
	if err := json.Unmarshal(out.Bytes(), &res); err != nil {
		return sdk.Result{}, fmt.Errorf("%s helper: %s", h, out.String())
	}
	if res.Error != "" {
		return sdk.Result{}, errors.New(res.Error)
	}
	if s, ok := res.Result.(string); ok {
		return sdk.Text(s), nil
	}
	return sdk.Result{Data: res.Result}, nil
}

func op[In any](c *sdk.Ctx, h helper, name, command, description string, kind sdk.Kind) {
	sdk.Op(c, name, description, kind, func(call *sdk.Call, in In) (sdk.Result, error) {
		return h.run(call.Context, command, in)
	})
}

type (
	noIn         struct{}
	reminderList struct {
		List      string `json:"list,omitempty" jsonschema:"a list's name; omit for all lists"`
		Completed bool   `json:"completed,omitempty" jsonschema:"include completed reminders"`
		Limit     int    `json:"limit,omitempty" jsonschema:"at most this many; default 200"`
	}
	reminderNew struct {
		Title    string `json:"title"`
		List     string `json:"list,omitempty" jsonschema:"a list's name; default is the default list"`
		Due      string `json:"due,omitempty" jsonschema:"ISO 8601 date (2026-11-01) or date-time (2026-11-01T09:00:00+10:00)"`
		Notes    string `json:"notes,omitempty"`
		Priority int    `json:"priority,omitempty" jsonschema:"0 none, 1 high, 5 medium, 9 low"`
	}
	reminderEdit struct {
		ID        string  `json:"id"`
		Title     *string `json:"title,omitempty"`
		List      *string `json:"list,omitempty"`
		Due       *string `json:"due,omitempty" jsonschema:"ISO 8601; empty string clears it"`
		Notes     *string `json:"notes,omitempty"`
		Priority  *int    `json:"priority,omitempty"`
		Completed *bool   `json:"completed,omitempty"`
	}
	byID struct {
		ID string `json:"id"`
	}
	eventRange struct {
		From     string `json:"from,omitempty" jsonschema:"ISO 8601; default is the start of today"`
		To       string `json:"to,omitempty" jsonschema:"ISO 8601; default is a week after from"`
		Calendar string `json:"calendar,omitempty" jsonschema:"a calendar's name; omit for all"`
		Query    string `json:"query,omitempty" jsonschema:"only events whose title, location or notes contain this"`
	}
	contactSearch struct {
		Query string `json:"query" jsonschema:"a name, email address or phone number"`
		Limit int    `json:"limit,omitempty" jsonschema:"at most this many; default 25"`
	}
	chatsIn struct {
		Limit int `json:"limit,omitempty" jsonschema:"at most this many chats, most recent first; default 30"`
	}
	readIn struct {
		Chat  string `json:"chat" jsonschema:"a chat id or group name from chats, or the other person's phone number or email"`
		Since string `json:"since,omitempty" jsonschema:"ISO 8601; only messages after this"`
		Limit int    `json:"limit,omitempty" jsonschema:"the most recent this many; default 50"`
	}
	searchIn struct {
		Query string `json:"query" jsonschema:"text to look for, case-insensitive"`
		Since string `json:"since,omitempty" jsonschema:"ISO 8601; only messages after this"`
		Limit int    `json:"limit,omitempty" jsonschema:"at most this many, newest first; default 50"`
	}
	sendIn struct {
		To   string `json:"to" jsonschema:"phone number (with country code) or email of an iMessage user"`
		Text string `json:"text"`
	}
	eventNew struct {
		Title    string `json:"title"`
		Start    string `json:"start" jsonschema:"ISO 8601 date-time, or a date for all-day events"`
		End      string `json:"end,omitempty" jsonschema:"ISO 8601; default is an hour (or a day) after start"`
		AllDay   bool   `json:"allDay,omitempty"`
		Calendar string `json:"calendar,omitempty" jsonschema:"a calendar's name; default is the default calendar"`
		Location string `json:"location,omitempty"`
		Notes    string `json:"notes,omitempty"`
	}
)

var Reminders = sdk.Plugin{ID: "reminders", Name: "Reminders", Version: "0.1.0", Platforms: []string{"darwin"}, Setup: func(c *sdk.Ctx) error {
	op[noIn](c, eventkit, "lists", "reminders.lists", "List the reminder lists.", sdk.Read)
	op[reminderList](c, eventkit, "list", "reminders.list", "List reminders, incomplete ones unless asked.", sdk.Read)
	op[reminderNew](c, eventkit, "create", "reminders.create", "Create a reminder.", sdk.Write)
	op[reminderEdit](c, eventkit, "update", "reminders.update", "Change a reminder: title, list, due date, notes, priority, or completed.", sdk.Write)
	op[byID](c, eventkit, "delete", "reminders.delete", "Delete a reminder.", sdk.Write)
	return nil
}}

var Calendar = sdk.Plugin{ID: "calendar", Name: "Calendar", Version: "0.1.0", Platforms: []string{"darwin"}, Setup: func(c *sdk.Ctx) error {
	op[noIn](c, eventkit, "calendars", "calendar.calendars", "List the calendars.", sdk.Read)
	op[eventRange](c, eventkit, "events", "calendar.events", "List events in a time range (default: the next week).", sdk.Read)
	op[eventNew](c, eventkit, "create", "calendar.create", "Create an event.", sdk.Write)
	op[byID](c, eventkit, "delete", "calendar.delete", "Delete an event.", sdk.Write)
	return nil
}}

var Contacts = sdk.Plugin{ID: "contacts", Name: "Contacts", Version: "0.1.0", Platforms: []string{"darwin"}, Setup: func(c *sdk.Ctx) error {
	op[contactSearch](c, contacts, "search", "contacts.search", "Find contacts by name, email address or phone number.", sdk.Read)
	op[byID](c, contacts, "get", "contacts.get", "Get one contact by id.", sdk.Read)
	op[noIn](c, contacts, "me", "contacts.me", "Get the user's own contact card.", sdk.Read)
	return nil
}}

var IMessage = sdk.Plugin{ID: "imessage", Name: "iMessage", Version: "0.1.0", Platforms: []string{"darwin"}, Setup: func(c *sdk.Ctx) error {
	op[chatsIn](c, messages, "chats", "messages.chats", "List recent iMessage and SMS conversations with their participants.", sdk.Read)
	op[readIn](c, messages, "read", "messages.read", "Read the messages in one conversation, oldest first.", sdk.Read)
	op[searchIn](c, messages, "search", "messages.search", "Search messages across conversations, newest first.", sdk.Read)
	op[sendIn](c, messages, "send", "messages.send", "Send an iMessage from the user's account. This sends a real message to a real person.", sdk.Write)
	return nil
}}
