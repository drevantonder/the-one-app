package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"net/http"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

// Whether a machine runs chrome devices is its owner's to say: a chrome device is the machine's own Chrome, outside
// any container, reaching what the machine reaches. Until they say, it does where the machine runs only its owner's
// own calls, and doesn't where it runs a group's, global's or another person's too (Where Things Run): their data is
// on it. What the owner chose is kept in this sidecar's folder, and holds whatever the machine comes to run.

// chromeAllowed is whether a machine runs chrome devices: as its owner chose, or by default only where it runs no one
// else's calls.
func chromeAllowed(chosen *bool, shared bool) bool {
	if chosen != nil {
		return *chosen
	}
	return !shared
}

// runsShared reports whether a machine whose owner is owner runs calls of scopes other than theirs, as runs_for
// lists them (global, group:<id>, user:<id>).
func runsShared(owner string, runsFor []string) bool {
	return slices.ContainsFunc(runsFor, func(s string) bool { return s != "user:"+owner })
}

// mayChooseChrome is whether caller (a user's id) may switch chrome devices on or off on a machine owned by owner.
func mayChooseChrome(caller, owner, machine string) error {
	if owner == "" {
		return fmt.Errorf("whose %s is isn't known here yet: try again in a moment", machine)
	}
	if caller != owner {
		return fmt.Errorf("only %s's owner may allow Chrome on it", machine)
	}
	return nil
}

func chromeFile() string { return filepath.Join(dataDir, "chrome.json") }

// chromeChoice is what this machine's owner chose, or nil.
func chromeChoice() *bool {
	b, err := os.ReadFile(chromeFile())
	if err != nil {
		return nil
	}
	var c struct{ Allowed *bool }
	json.Unmarshal(b, &c)
	return c.Allowed
}

// A machine is one of the install's devices, as its node's /v1/state says.
type machine struct {
	Name    string   `json:"name"`
	User    string   `json:"user"`
	Here    bool     `json:"here"`
	RunsFor []string `json:"runs_for"`
}

// machines are this machine and the install's devices, as its node says.
func (ds *devices) machines(ctx context.Context) (here machine, all []machine, err error) {
	r, err := http.NewRequestWithContext(ctx, http.MethodGet, ds.node+"/v1/state", nil)
	if err != nil {
		return here, nil, err
	}
	resp, err := http.DefaultClient.Do(r)
	if err != nil {
		return here, nil, err
	}
	defer resp.Body.Close()
	var st struct {
		Me, Device struct{ Name string }
		User       struct{ ID string }
		Install    struct{ Devices []machine }
	}
	if err := json.NewDecoder(resp.Body).Decode(&st); err != nil {
		return here, nil, err
	}
	for _, d := range st.Install.Devices {
		if d.Here {
			here = d
		}
	}
	here.Name = cmpOr(st.Me.Name, cmpOr(st.Device.Name, here.Name))
	here.User = cmpOr(here.User, st.User.ID)
	return here, st.Install.Devices, nil
}

// chromeHere is this machine's row of chrome: whether it runs chrome devices, and why.
func (ds *devices) chromeHere(ctx context.Context) (map[string]any, error) {
	me, _, err := ds.machines(ctx)
	if err != nil {
		return nil, err
	}
	return chromeRow(me), nil
}

func chromeRow(me machine) map[string]any {
	chosen := chromeChoice()
	shared := runsShared(me.User, me.RunsFor)
	return map[string]any{"on": me.Name, "allowed": chromeAllowed(chosen, shared), "shared": shared, "chosen": chosen != nil, "can": chromeKind{}.can()}
}

// chromeWhyNot is why this machine runs no chrome devices now, or "".
func (ds *devices) chromeWhyNot(ctx context.Context) string {
	if why := (chromeKind{}).can(); why != "" {
		return why
	}
	row, err := ds.chromeHere(ctx)
	if err != nil {
		return "whether its owner allows Chrome here isn't known: " + err.Error()
	}
	if row["allowed"] == true {
		return ""
	}
	if row["chosen"] == true {
		return fmt.Sprintf("%s's owner switched Chrome off here", row["on"])
	}
	return fmt.Sprintf("Chrome is off on %s, which runs others' calls too, until its owner allows it", row["on"])
}

// runChrome runs chrome and set_chrome.
func (ds *devices) runChrome(ctx context.Context, op string, here bool, in map[string]any) (any, error) {
	self, all, err := ds.machines(ctx)
	if err != nil {
		return nil, err
	}
	mine, me := chromeRow(self), self.Name
	if op == "chrome" {
		if here {
			return mine, nil
		}
		rows := []any{mine}
		got, failed := ds.everywhere(ctx, "chrome", nil)
		for _, s := range slices.Sorted(maps.Keys(got)) {
			var row map[string]any
			if json.Unmarshal(got[s], &row) == nil && row["on"] != nil {
				rows = append(rows, row)
			}
		}
		out := map[string]any{"devices": rows}
		if len(failed) > 0 {
			out["unreachable"] = failed
		}
		return out, nil
	}
	on, _ := in["on"].(string)
	on = cmpOr(strings.TrimSpace(on), me)
	allowed, ok := in["allowed"].(bool)
	if !ok {
		return nil, errors.New("allowed: true or false")
	}
	owner := ""
	for _, d := range all {
		if strings.EqualFold(d.Name, on) {
			on, owner = d.Name, d.User
		}
	}
	if on == me {
		owner = self.User
	}
	// Who asked must own the machine: here, and again there when it's passed on, where it's asked as this machine's
	// person, so it's passed on only for the person whose machines both are.
	if err := mayChooseChrome(callerUser(ctx), owner, on); err != nil {
		return nil, err
	}
	if on != me {
		if here {
			return nil, fmt.Errorf("set_chrome for %s reached %s", on, me)
		}
		raw, err := ds.callNode(ctx, on, "set_chrome", map[string]any{"on": on, "allowed": allowed, "here": true})
		if err != nil {
			return nil, err
		}
		var row map[string]any
		json.Unmarshal(raw, &row)
		return row, nil
	}
	b, _ := json.Marshal(map[string]any{"allowed": allowed})
	if err := os.WriteFile(chromeFile(), b, 0o600); err != nil {
		return nil, err
	}
	return chromeRow(self), nil
}

type callerUserKey struct{}

// callerUser is the id of the person a call is from.
func callerUser(ctx context.Context) string {
	user, _ := ctx.Value(callerUserKey{}).(string)
	return user
}
