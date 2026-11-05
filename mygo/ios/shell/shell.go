// Package shell is what the iPhone's shell does natively for The One App that MyGo doesn't: the system's sign-in
// sheet (ASWebAuthenticationSession) and menus (an action sheet). Its Swift is ios/, compiled into MyGo's host.
package shell

import (
	"context"

	"github.com/drevantonder/the-one-app/mygo/bridge"
	"github.com/egoist/mygo"
)

var native = mygo.Native("oneshell")

// OpenSignIn shows url in the system's sign-in sheet, which stays until CloseSignIn: sign-in comes back to a
// loopback port, which the sheet loads like any page.
func OpenSignIn(url string) error {
	return native.Call(context.Background(), "openSignIn", map[string]any{"url": url}, nil)
}

// CloseSignIn takes the sheet away.
func CloseSignIn() { _ = native.Call(context.Background(), "closeSignIn", nil, nil) }

// Menu shows a screen's menu as an action sheet → the index chosen, or -1.
func Menu(items []bridge.MenuItem) (int, error) {
	var out struct {
		Index *int `json:"index"`
	}
	if err := native.Call(context.Background(), "menu", map[string]any{"items": items}, &out); err != nil || out.Index == nil {
		return -1, err
	}
	return *out.Index, nil
}

// Group leaves text in the App Group the app shares with its extensions → the group's folder.
func Group(group, text string) (string, error) {
	var out struct {
		Path string `json:"path"`
	}
	err := native.Call(context.Background(), "group", map[string]any{"group": group, "text": text}, &out)
	return out.Path, err
}

// Notify asks to show notifications, and registers for pushes.
func Notify() (bool, error) {
	var out struct {
		Granted bool `json:"granted"`
	}
	err := native.Call(context.Background(), "notify", nil, &out)
	return out.Granted, err
}

// Passkey asks the system for a passkey assertion for rp (the install's sign-in domain) over challenge (base64).
func Passkey(rp, challenge string) (map[string]string, error) {
	var out map[string]string
	err := native.Call(context.Background(), "passkey", map[string]any{"rp": rp, "challenge": challenge}, &out)
	return out, err
}
