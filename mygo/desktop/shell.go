package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"time"

	"github.com/drevantonder/the-one-app/mygo/bridge"
	"github.com/egoist/mygo"
)

// shell is bridge.Shell for one screen's window, with MyGo's own menus, dialogs, clipboard and notifications.
type shell struct{ s *screen }

func (sh *shell) Menu(items []bridge.MenuItem, x, y float64, rect []float64) (int, error) {
	chosen := make(chan int, 1)
	var built []*mygo.MenuItem
	checks := false
	for _, it := range items {
		checks = checks || it.Checked
	}
	for i, it := range items {
		if i > 0 && (it.Section || (items[i-1].Row && !it.Row)) {
			built = append(built, mygo.Separator())
		}
		i := i
		m := &mygo.MenuItem{Label: it.Title, Click: func(*mygo.MenuItem, *mygo.Window) { chosen <- i }}
		if checks {
			m.Type, m.Checked = mygo.MenuItemCheckbox, it.Checked
		}
		built = append(built, m)
	}
	mygo.NewMenu(built).PopupAt(sh.s.win, int(x), int(y))
	// The menu's gone: its click, if any, comes now or a moment after.
	select {
	case i := <-chosen:
		return i, nil
	case <-time.After(300 * time.Millisecond):
		return -1, nil
	}
}

func (sh *shell) Alert(title, text, action string, danger bool) (bool, error) {
	opts := mygo.MessageOptions{Parent: sh.s.win, Message: title, Detail: text, Buttons: []string{"OK"}}
	if action != "" {
		opts.Buttons, opts.DefaultButton, opts.CancelButton = []string{"Cancel", action}, 1, 0
		if danger {
			opts.DestructiveButtons = []int{1}
		}
	}
	r, err := mygo.Dialog.Message(opts)
	return action != "" && r.Button == 1, err
}

func (sh *shell) Copy(text string) error {
	mygo.Clipboard.WriteText(text)
	return nil
}

func (sh *shell) Link(u string) error {
	outside(u)
	return nil
}

func (sh *shell) Title(text, name string) {
	t := name
	if t == "" {
		t = text
	}
	if t == "" {
		t = sh.s.route.App
	}
	sh.s.win.SetTitle(t)
}

func (sh *shell) Open(to bridge.Route, sheet bool) (json.RawMessage, error) {
	if !sheet {
		if openScreen(to, nil) == nil {
			return nil, errors.New("It didn’t open.")
		}
		return nil, nil
	}
	s := openScreen(to, sh.s)
	if s == nil {
		return nil, errors.New("It didn’t open.")
	}
	return <-s.closed, nil
}

func (sh *shell) Close(value json.RawMessage) {
	sh.s.value = value
	sh.s.win.Close()
}

func (sh *shell) Notify() (string, error) {
	if !mygo.NotificationsSupported() {
		return "denied", nil
	}
	err := mygo.NewNotification(mygo.NotificationOptions{Title: "The One App", Body: "Notifications are on."}).Show()
	if errors.Is(err, mygo.ErrNotificationsDenied) {
		return "denied", nil
	}
	if err != nil {
		return "default", err
	}
	return "granted", nil
}

func (sh *shell) Save(name string, data []byte) error {
	path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: sh.s.win, DefaultPath: name})
	if err != nil || path == "" {
		return err
	}
	return os.WriteFile(path, data, 0o644)
}

func (sh *shell) Preview(name string, data []byte) error {
	dir, err := os.MkdirTemp("", "one-preview-")
	if err != nil {
		return err
	}
	path := filepath.Join(dir, filepath.Base(name))
	if err := os.WriteFile(path, data, 0o600); err != nil {
		return err
	}
	return mygo.Shell.OpenPath(path)
}

func (sh *shell) SignOut() error { return signOut() }

func (sh *shell) Device() string {
	switch runtime.GOOS {
	case "darwin":
		return "Mac"
	case "windows":
		return "Windows"
	}
	return "Linux"
}
