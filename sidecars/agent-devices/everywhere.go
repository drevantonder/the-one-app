package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"maps"
	"net/http"
	"slices"
	"strings"
	"sync"
	"time"
)

// This sidecar reaches the same sidecar on the user's other servers through
// its own node's local API, as any app would: list and backups show every
// server's, and move restores a device on another. Calls passed on carry
// here: true, so they go no further.

// callNode runs op on server (by name; "" for wherever the node picks) and
// returns its result.
func (ds *devices) callNode(ctx context.Context, server, op string, input map[string]any) (json.RawMessage, error) {
	req := map[string]any{"app": app, "sidecar": app, "op": op, "input": input}
	if server != "" {
		req["server"] = server
	}
	return ds.post(ctx, req)
}

// callApp calls one of the app's operations, on this server's node.
func (ds *devices) callApp(ctx context.Context, op string, input any) (json.RawMessage, error) {
	return ds.post(ctx, map[string]any{"app": app, "op": op, "input": input})
}

func (ds *devices) post(ctx context.Context, req map[string]any) (json.RawMessage, error) {
	b, _ := json.Marshal(req)
	r, err := http.NewRequestWithContext(ctx, http.MethodPost, ds.node+"/v1/call", bytes.NewReader(b))
	if err != nil {
		return nil, err
	}
	r.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(r)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	var reply struct {
		OK    json.RawMessage `json:"ok"`
		Error string          `json:"error"`
	}
	json.Unmarshal(body, &reply)
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("%s", cmpOr(reply.Error, resp.Status))
	}
	return reply.OK, nil
}

func cmpOr(a, b string) string {
	if a != "" {
		return a
	}
	return b
}

// servers are this server's name and the other online servers that run
// devices, as the node says.
func (ds *devices) servers(ctx context.Context) (me string, others []string, err error) {
	r, err := http.NewRequestWithContext(ctx, http.MethodGet, ds.node+"/v1/state", nil)
	if err != nil {
		return "", nil, err
	}
	resp, err := http.DefaultClient.Do(r)
	if err != nil {
		return "", nil, err
	}
	defer resp.Body.Close()
	var st struct {
		Me      struct{ Name string }
		Install struct {
			Servers []struct {
				Name     string
				Here     bool
				Online   bool
				Sidecars []string
			}
		}
	}
	if err := json.NewDecoder(resp.Body).Decode(&st); err != nil {
		return "", nil, err
	}
	for _, s := range st.Install.Servers {
		if !s.Here && s.Online && slices.Contains(s.Sidecars, app) {
			others = append(others, s.Name)
		}
	}
	return st.Me.Name, others, nil
}

// everywhere runs op on every other server that runs devices, at once,
// giving each one's result, or why it couldn't.
func (ds *devices) everywhere(ctx context.Context, op string, input map[string]any) (map[string]json.RawMessage, map[string]string) {
	_, others, err := ds.servers(ctx)
	got, failed := map[string]json.RawMessage{}, map[string]string{}
	if err != nil {
		failed["this server"] = err.Error()
		return got, failed
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	in := map[string]any{"here": true}
	for k, v := range input {
		in[k] = v
	}
	var mu sync.Mutex
	var wg sync.WaitGroup
	for _, s := range others {
		wg.Go(func() {
			r, err := ds.callNode(ctx, s, op, in)
			mu.Lock()
			defer mu.Unlock()
			if err != nil {
				failed[s] = err.Error()
			} else {
				got[s] = r
			}
		})
	}
	wg.Wait()
	return got, failed
}

// listed is a device as list shows it.
type listed struct {
	Name        string    `json:"name"`
	Server      string    `json:"server"`
	State       string    `json:"state"` // running, paused or stopped
	Started     time.Time `json:"started,omitzero"`
	Size        string    `json:"size"`
	CPUs        float64   `json:"cpus,omitempty"`         // at most; none: no limit
	MemoryLimit int64     `json:"memory_limit,omitempty"` // bytes
	Memory      int64     `json:"memory,omitempty"`       // bytes in use now
	CPU         float64   `json:"cpu"`                    // percent of one CPU, now
	Borrowed    *loan     `json:"borrowed,omitempty"`
	Spare       bool      `json:"spare,omitempty"`
	LastCall    time.Time `json:"last_call,omitzero"`
}

// listHere is this server's devices.
func (ds *devices) listHere(ctx context.Context) ([]listed, error) {
	me, _, err := ds.servers(ctx)
	if err != nil {
		return nil, err
	}
	all, err := list(ctx)
	if err != nil {
		return nil, err
	}
	lims, err := limits(ctx, all)
	if err != nil {
		return nil, err
	}
	use, _ := stats(ctx)
	lent := ds.loans()
	ds.poolMu.Lock()
	used := maps.Clone(ds.used)
	ds.poolMu.Unlock()
	out := []listed{}
	for _, d := range all {
		l := listed{Name: d.Name, Server: me, State: d.State, Started: d.Started, Size: lims[d.Name].sizeOf(),
			CPUs: lims[d.Name].CPUs, MemoryLimit: lims[d.Name].Memory, Memory: use[d.Name].Memory, CPU: use[d.Name].CPU, LastCall: used[d.Name]}
		if b, ok := lent[d.Name]; ok {
			l.Borrowed = &b
		} else {
			l.Spare = d.pool
		}
		out = append(out, l)
	}
	return out, nil
}

// listAll is every server's devices, this one's first.
func (ds *devices) listAll(ctx context.Context) (map[string]any, error) {
	all, err := ds.listHere(ctx)
	if err != nil {
		return nil, err
	}
	got, failed := ds.everywhere(ctx, "list", nil)
	for _, s := range slices.Sorted(maps.Keys(got)) {
		var r struct{ Devices []listed }
		if err := json.Unmarshal(got[s], &r); err != nil {
			failed[s] = err.Error()
		}
		all = append(all, r.Devices...)
	}
	out := map[string]any{"devices": all}
	if len(all) == 0 {
		out["note"] = "No devices. Borrow one with borrow, or make a lasting one with create."
	}
	if len(failed) > 0 {
		var why []string
		for s, e := range failed {
			why = append(why, s+": "+e)
		}
		slices.Sort(why)
		out["unreachable"] = strings.Join(why, "; ")
	}
	return out, nil
}

// located is the other server that has the device named, or "".
func (ds *devices) located(ctx context.Context, name string) string {
	got, _ := ds.everywhere(ctx, "list", nil)
	for s, r := range got {
		var theirs struct{ Devices []listed }
		json.Unmarshal(r, &theirs)
		for _, d := range theirs.Devices {
			if d.Name == name {
				return s
			}
		}
	}
	return ""
}

// borrowElsewhere runs op (borrow or sign_in) on another server that runs
// devices, for a server that shouldn't make one itself; nil if none could.
func (ds *devices) borrowElsewhere(ctx context.Context, op string, input map[string]any) json.RawMessage {
	_, others, err := ds.servers(ctx)
	if err != nil {
		return nil
	}
	in := map[string]any{"here": true}
	for k, v := range input {
		in[k] = v
	}
	for _, s := range others {
		r, err := ds.callNode(ctx, s, op, in)
		if err == nil {
			return r
		}
		log.Printf("%s on %s: %v", op, s, err)
	}
	return nil
}

// signingIn reports whether a device here is signing in to the profile.
func (ds *devices) signingIn(item string) bool {
	for _, l := range ds.loans() {
		if l.SigningIn && l.Item == item {
			return true
		}
	}
	return false
}

// signingInElsewhere is the other server with a device signing in to the
// profile, or "".
func (ds *devices) signingInElsewhere(ctx context.Context, item string) string {
	got, _ := ds.everywhere(ctx, "signing_in", nil)
	for s, r := range got {
		var theirs struct {
			SigningIn []struct{ Item string } `json:"signing_in"`
		}
		json.Unmarshal(r, &theirs)
		for _, l := range theirs.SigningIn {
			if l.Item == item {
				return s
			}
		}
	}
	return ""
}
