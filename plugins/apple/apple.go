// Package apple holds the built-in reminders and calendar plugins. They run
// on macOS, where a small Swift helper reads and writes Apple's stores
// through EventKit; other devices reach them through the mesh.
package apple

import (
	"bytes"
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"

	"github.com/drevantonder/the-one-app/sdk"
)

//go:embed eventkit.swift
var helperSource []byte

//go:embed Info.plist
var helperPlist []byte

var (
	helperOnce sync.Once
	helperPath string
	helperErr  error
)

// helper compiles the Swift helper the first time it's needed, and again
// only when its source changes. macOS asks for permission once per build.
func helper() (string, error) {
	helperOnce.Do(func() {
		sum := sha256.Sum256(append(append([]byte{}, helperSource...), helperPlist...))
		cache, err := os.UserCacheDir()
		if err != nil {
			helperErr = err
			return
		}
		dir := filepath.Join(cache, "one", "eventkit-"+hex.EncodeToString(sum[:6]))
		helperPath = filepath.Join(dir, "one-eventkit")
		if _, err := os.Stat(helperPath); err == nil {
			return
		}
		if err := os.MkdirAll(dir, 0o755); err != nil {
			helperErr = err
			return
		}
		src, plist := filepath.Join(dir, "eventkit.swift"), filepath.Join(dir, "Info.plist")
		_ = os.WriteFile(src, helperSource, 0o644)
		_ = os.WriteFile(plist, helperPlist, 0o644)
		tmp := helperPath + ".tmp"
		build := exec.Command("swiftc", "-O", src, "-o", tmp, "-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist", "-Xlinker", plist)
		if out, err := build.CombinedOutput(); err != nil {
			helperErr = fmt.Errorf("building the EventKit helper needs the Xcode command line tools (xcode-select --install): %v\n%s", err, out)
			return
		}
		if out, err := exec.Command("codesign", "-s", "-", "-f", "--identifier", "dev.theoneapp.eventkit", tmp).CombinedOutput(); err != nil {
			helperErr = fmt.Errorf("signing the EventKit helper: %v\n%s", err, out)
			return
		}
		helperErr = os.Rename(tmp, helperPath)
	})
	return helperPath, helperErr
}

// run calls the helper with a command and JSON input.
func run(ctx context.Context, command string, input any) (sdk.Result, error) {
	path, err := helper()
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
		return sdk.Result{}, fmt.Errorf("EventKit helper: %s", out.String())
	}
	if res.Error != "" {
		return sdk.Result{}, errors.New(res.Error)
	}
	if s, ok := res.Result.(string); ok {
		return sdk.Text(s), nil
	}
	return sdk.Result{Data: res.Result}, nil
}

func op[In any](c *sdk.Ctx, name, command, description string, kind sdk.Kind) {
	sdk.Op(c, name, description, kind, func(call *sdk.Call, in In) (sdk.Result, error) {
		return run(call.Context, command, in)
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

var Reminders = sdk.Plugin{ID: "reminders", Version: "0.1.0", Platforms: []string{"darwin"}, Setup: func(c *sdk.Ctx) error {
	op[noIn](c, "lists", "reminders.lists", "List the reminder lists.", sdk.Read)
	op[reminderList](c, "list", "reminders.list", "List reminders, incomplete ones unless asked.", sdk.Read)
	op[reminderNew](c, "create", "reminders.create", "Create a reminder.", sdk.Write)
	op[reminderEdit](c, "update", "reminders.update", "Change a reminder: title, list, due date, notes, priority, or completed.", sdk.Write)
	op[byID](c, "delete", "reminders.delete", "Delete a reminder.", sdk.Write)
	return nil
}}

var Calendar = sdk.Plugin{ID: "calendar", Version: "0.1.0", Platforms: []string{"darwin"}, Setup: func(c *sdk.Ctx) error {
	op[noIn](c, "calendars", "calendar.calendars", "List the calendars.", sdk.Read)
	op[eventRange](c, "events", "calendar.events", "List events in a time range (default: the next week).", sdk.Read)
	op[eventNew](c, "create", "calendar.create", "Create an event.", sdk.Write)
	op[byID](c, "delete", "calendar.delete", "Delete an event.", sdk.Write)
	return nil
}}
