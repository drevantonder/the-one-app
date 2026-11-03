package main

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"
)

// The page asks for the list every few seconds and for each running device's
// picture now and then, and asking the devices takes a while (the other devices'
// sidecars over the relay, Podman, simctl, adb, a picture taken). So the last
// answer is kept and given at once, and asked again in the background once it's
// older than fresh. One older than stale, or none yet, is waited for. A change
// made here forgets the list, so whoever made it sees it.

type kept struct {
	fresh, stale time.Duration
	mu           sync.Mutex
	val          any
	at           time.Time
	gen          int // which forget it's from, so an answer asked before one isn't kept
	asking       bool
}

func (k *kept) get(ask func(context.Context) (any, error)) (any, time.Time, error) {
	k.mu.Lock()
	v, at, gen := k.val, k.at, k.gen
	if !at.IsZero() && time.Since(at) < k.stale {
		if time.Since(at) > k.fresh && !k.asking {
			k.asking = true
			go k.ask(ask, gen)
		}
		k.mu.Unlock()
		return v, at, nil
	}
	k.mu.Unlock()
	return k.ask(ask, gen)
}

func (k *kept) ask(ask func(context.Context) (any, error), gen int) (any, time.Time, error) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	v, err := ask(ctx)
	now := time.Now()
	k.mu.Lock()
	defer k.mu.Unlock()
	k.asking = false
	if err == nil && gen == k.gen {
		k.val, k.at = v, now
	}
	return v, now, err
}

func (k *kept) forget() {
	k.mu.Lock()
	k.val, k.at = nil, time.Time{}
	k.gen++
	k.mu.Unlock()
}

// changes are the operations after which the list isn't what it was.
var changes = []string{"adopt", "borrow", "give_back", "sign_in", "save_profile", "create", "resize", "start", "stop", "delete", "restore", "move"}

// listKept is list, kept: this device's, or with others, every device's.
func (ds *devices) listKept(here bool) (any, error) {
	k, ask := &ds.keptAll, func(ctx context.Context) (any, error) { return ds.listAll(ctx, true) }
	if here {
		k, ask = &ds.keptHere, func(ctx context.Context) (any, error) {
			all, err := ds.listHere(ctx)
			return map[string]any{"devices": all}, err
		}
	}
	v, _, err := k.get(ask)
	return v, err
}

// pictures are the devices' pictures, kept by device and width.
type pictures struct {
	mu sync.Mutex
	by map[string]*kept
}

// screenKept is a device's picture, kept for a little while: a page watching it
// asks again every so often, and gets the one taken in the background since. Its
// hash says which it is: given the one it has (have), it's told it's the same
// rather than sent it again.
func (ds *devices) screenKept(name string, width int, have string, take func(context.Context) (any, error)) (any, error) {
	ds.pics.mu.Lock()
	if ds.pics.by == nil {
		ds.pics.by = map[string]*kept{}
	}
	key := fmt.Sprintf("%s/%d", name, width)
	k := ds.pics.by[key]
	if k == nil {
		k = &kept{fresh: 15 * time.Second, stale: 24 * time.Hour}
		ds.pics.by[key] = k
	}
	ds.pics.mu.Unlock()
	v, at, err := k.get(func(ctx context.Context) (any, error) {
		r, err := take(ctx)
		if err != nil {
			return nil, err
		}
		b, _ := json.Marshal(r)
		sum := sha256.Sum256(b)
		return map[string]any{"picture": r, "hash": base64.RawURLEncoding.EncodeToString(sum[:9])}, nil
	})
	if err != nil {
		return nil, err
	}
	m := v.(map[string]any)
	age := int(time.Since(at).Seconds())
	if have != "" && have == m["hash"] {
		return map[string]any{"same": true, "hash": m["hash"], "age": age}, nil
	}
	out := map[string]any{"hash": m["hash"], "age": age}
	if p, ok := m["picture"].(map[string]any); ok {
		for k, v := range p {
			out[k] = v
		}
	}
	return out, nil
}

// forgetPictures drops a device's pictures, when it stops or goes.
func (ds *devices) forgetPictures(name string) {
	ds.pics.mu.Lock()
	defer ds.pics.mu.Unlock()
	for key := range ds.pics.by {
		if strings.HasPrefix(key, name+"/") {
			delete(ds.pics.by, key)
		}
	}
}
