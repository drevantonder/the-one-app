package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"os/exec"
	"strings"
	"time"
)

// A device nobody has called or watched for a while is paused: it keeps its
// memory, and its browser's pages, but uses no CPU. The next call to it
// unpauses it first, which takes a moment.
const idlePause = 30 * time.Minute

func (ds *devices) pauseIdle(ctx context.Context, all []device, lent map[string]loan) {
	for _, d := range all {
		if _, ok := lent[d.Name]; ok || d.State != "running" || ds.idle(d.Name) < idlePause {
			continue
		}
		if watched(ctx, d) {
			ds.touch(d.Name)
			continue
		}
		ds.drop(d.Name) // its session's requests would wait
		if err := pause(ctx, d); errors.Is(err, errCant) {
			return
		} else if err != nil {
			log.Printf("pausing %s: %v", d.Name, err)
			continue
		}
		log.Printf("paused %s, idle for %s", d.Name, idlePause)
	}
}

// watched reports whether a browser has the device's viewer open: a
// connection to its port, which the viewer keeps open while it shows it.
func watched(ctx context.Context, d device) bool {
	out, err := exec.CommandContext(ctx, "ss", "-Htn", "state", "established", fmt.Sprintf("( sport = :%d )", d.port)).Output()
	return err == nil && strings.TrimSpace(string(out)) != ""
}

// running is the device named, unpausing it if paused, and refusing a
// stopped one; it counts as a call to it.
func (ds *devices) running(ctx context.Context, name string) (device, error) {
	d, err := find(ctx, name)
	if err != nil {
		return d, err
	}
	if err := ds.mayStart(ctx, d.Name); err != nil {
		return d, err
	}
	switch d.State {
	case "stopped":
		return d, fmt.Errorf("device %s is stopped: start it first", d.Name)
	case "paused":
		if err := unpause(ctx, d); err != nil {
			return d, err
		}
		d.State = "running"
	}
	ds.touch(d.Name)
	return d, nil
}
