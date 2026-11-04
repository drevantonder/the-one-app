package main

// What this machine keeps about each device's lifecycle, beside the runtimes' own state, in this service's folder:
//
//   - running.json: whether each device should be running, of every kind, set only once a start or stop worked. A
//     restart of this service, or of the machine, starts again those that should run and have room (revive).
//   - moved.json: devices moved to another machine. Moving one says where it's going before the other machine
//     restores it, and that it went once it did; until it's deleted here, it's kept stopped and refuses to start,
//     saying where it went.

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"
)

var lifeMu sync.Mutex

func lifeFile(name string) string { return filepath.Join(dataDir, name) }

func readLife(file string, v any) {
	if b, err := os.ReadFile(lifeFile(file)); err == nil {
		if err := json.Unmarshal(b, v); err != nil {
			log.Printf("%s: %v", file, err)
		}
	}
}

func writeLife(file string, v any) error {
	b, _ := json.MarshalIndent(v, "", "  ")
	tmp := lifeFile(file) + ".part"
	if err := os.WriteFile(tmp, b, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, lifeFile(file))
}

// wanted is whether each device should be running, by name.
func wanted() map[string]bool {
	lifeMu.Lock()
	defer lifeMu.Unlock()
	w := map[string]bool{}
	readLife("running.json", &w)
	return w
}

// want keeps whether a device should be running, once starting or stopping it worked.
func want(name string, running bool) {
	lifeMu.Lock()
	defer lifeMu.Unlock()
	w := map[string]bool{}
	readLife("running.json", &w)
	if had, ok := w[name]; ok && had == running {
		return
	}
	w[name] = running
	if err := writeLife("running.json", w); err != nil {
		log.Printf("keeping whether %s should run: %v", name, err)
	}
}

// unwant forgets a device that's deleted.
func unwant(name string) {
	lifeMu.Lock()
	defer lifeMu.Unlock()
	w := map[string]bool{}
	readLife("running.json", &w)
	if _, ok := w[name]; !ok {
		return
	}
	delete(w, name)
	if err := writeLife("running.json", w); err != nil {
		log.Printf("%v", err)
	}
}

// A move is a device moved, or being moved, from here to another machine.
type move struct {
	To   string    `json:"to"`
	At   time.Time `json:"at"`
	Done bool      `json:"done,omitempty"` // the other machine restored it; until then it's being moved
}

func moves() map[string]move {
	lifeMu.Lock()
	defer lifeMu.Unlock()
	m := map[string]move{}
	readLife("moved.json", &m)
	return m
}

// setMove keeps a device's move, or with none, forgets it.
func setMove(name string, m *move) error {
	lifeMu.Lock()
	defer lifeMu.Unlock()
	all := map[string]move{}
	readLife("moved.json", &all)
	if m == nil {
		if _, ok := all[name]; !ok {
			return nil
		}
		delete(all, name)
	} else {
		all[name] = *m
	}
	return writeLife("moved.json", all)
}

// movedAway is where the device named here went, once it's settled whether it did: a move that never finished
// asks the machine it was going to whether it has it.
func (ds *devices) movedAway(ctx context.Context, name string) (move, error) {
	m, ok := moves()[name]
	if !ok || m.Done {
		return m, nil
	}
	has, err := ds.hasThere(ctx, m.To, name)
	if err != nil {
		return m, fmt.Errorf("device %s was being moved to %s, which didn't answer whether it has it: it stays stopped here until it does (%v)", name, m.To, err)
	}
	if has {
		m.Done = true
		return m, setMove(name, &m)
	}
	return move{}, setMove(name, nil)
}

// hasThere asks machine whether it has a device named name.
func (ds *devices) hasThere(ctx context.Context, machine, name string) (bool, error) {
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	raw, err := ds.callNode(ctx, machine, "list", map[string]any{"here": true})
	if err != nil {
		return false, err
	}
	var theirs struct{ Devices []listed }
	if err := json.Unmarshal(raw, &theirs); err != nil {
		return false, err
	}
	return slices.ContainsFunc(theirs.Devices, func(d listed) bool { return d.Name == name && d.MovedTo == "" }), nil
}

// errMoved is why a moved device doesn't start here.
func errMoved(name string, m move) error {
	return fmt.Errorf("device %s moved to %s on %s: use it there; it's kept stopped here until you delete it here", name, m.To, m.At.Local().Format("2 Jan 15:04"))
}

// mayStart refuses a device moved, or maybe moved, to another machine.
func (ds *devices) mayStart(ctx context.Context, name string) error {
	m, err := ds.movedAway(ctx, name)
	if err != nil {
		return err
	}
	if m.Done {
		return errMoved(name, m)
	}
	return nil
}

// tendMoved stops devices moved away that still run here, as after a move that stopped before stopping it.
func (ds *devices) tendMoved(ctx context.Context, all []device) {
	for name := range moves() {
		i := slices.IndexFunc(all, func(d device) bool { return d.Name == name })
		if i < 0 {
			setMove(name, nil) // deleted here
			continue
		}
		if all[i].State == "stopped" {
			continue
		}
		if m, err := ds.movedAway(ctx, name); err != nil || !m.Done {
			continue
		}
		ds.drop(name)
		if err := stop(ctx, all[i]); err != nil {
			log.Printf("stopping %s, moved away: %v", name, err)
		}
	}
}

// revive starts devices that should be running but aren't, as after the machine restarted, once their runtime
// answers, and as many as have room; and stops Linux ones that shouldn't be running. Devices from before running.json
// should run as their runtime kept it: Podman's restart policy, a Mac's stopped marker.
func (ds *devices) revive(ctx context.Context) {
	var all []device
	var err error
	for i := 0; rt != nil; i++ {
		if all, err = list(ctx); err == nil {
			break
		}
		if i == 60 {
			log.Printf("not starting devices: %v", err)
			break
		}
		time.Sleep(5 * time.Second)
	}
	lims, err := limits(ctx, all)
	if err != nil {
		log.Print(err)
	}
	w := wanted()
	moved := moves()
	for _, d := range all {
		should, ok := w[d.Name]
		if !ok {
			should = lims[d.Name].Restart
			want(d.Name, should)
		}
		if p, ok := rt.(podmanRT); ok && lims[d.Name].Restart {
			p.noRestart(ctx, d) // this, not Podman, starts it, once it has room
		}
		if _, gone := moved[d.Name]; gone {
			should = false
		}
		switch {
		case should && d.State == "stopped":
			gb := int(lims[d.Name].Memory >> 30)
			if !roomFor(max(gb, 1)) {
				log.Printf("not starting %s, which should be running: no memory to spare", d.Name)
				continue
			}
			log.Printf("starting %s, which should be running", d.Name)
			if err := rt.start(ctx, d); err != nil {
				log.Print(err)
			}
		case !should && d.State != "stopped":
			log.Printf("stopping %s, which should be stopped", d.Name)
			if err := stop(ctx, d); err != nil {
				log.Print(err)
			}
		}
	}
	lent := ds.loans()
	for _, v := range ds.vms() {
		k := v.kind()
		if !w[v.Name] || k == nil || k.can() != "" || v.Kind == chromeName || k.running(ctx, v) {
			continue
		}
		if _, ok := lent[v.Name]; !ok && (v.Of != "" || strings.HasPrefix(v.Name, poolPrefix)) {
			continue // a throwaway copy nobody borrows now
		}
		if !roomFor(gbOf(v)) {
			log.Printf("not starting %s, which should be running: no memory to spare", v.Name)
			continue
		}
		log.Printf("starting %s, which should be running", v.Name)
		v.Paused = false
		ds.saveVM(v)
		if err := k.start(ctx, v); err != nil {
			log.Printf("starting %s: %v", v.Name, err)
		}
	}
}
