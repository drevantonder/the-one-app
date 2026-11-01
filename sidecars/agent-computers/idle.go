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

// A computer nobody has called or watched for a while is paused: it keeps its
// memory, and its browser's pages, but uses no CPU. The next call to it
// unpauses it first, which takes a moment.
const idlePause = 30 * time.Minute

func (ds *computers) pauseIdle(ctx context.Context, all []computer, lent map[string]loan) {
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

// watched reports whether a browser has the computer's viewer open: a
// connection to its port, which the viewer keeps open while it shows it.
func watched(ctx context.Context, d computer) bool {
	out, err := exec.CommandContext(ctx, "ss", "-Htn", "state", "established", fmt.Sprintf("( sport = :%d )", d.port)).Output()
	return err == nil && strings.TrimSpace(string(out)) != ""
}

// running is the computer named, unpausing it if paused, and refusing a
// stopped one; it counts as a call to it.
func (ds *computers) running(ctx context.Context, name string) (computer, error) {
	d, err := find(ctx, name)
	if err != nil {
		return d, err
	}
	switch d.State {
	case "stopped":
		return d, fmt.Errorf("computer %s is stopped: start it first", d.Name)
	case "paused":
		if err := unpause(ctx, d); err != nil {
			return d, err
		}
		d.State = "running"
	}
	ds.touch(d.Name)
	return d, nil
}

// revive starts computers that should be running but aren't, once Podman
// answers: after a reboot on Linux, podman-restart does, unless the user's
// systemd wasn't ready for it; Apple's container has no restart policy.
func revive(ctx context.Context) {
	var all []computer
	var err error
	for i := 0; ; i++ {
		if all, err = list(ctx); err == nil {
			break
		}
		if i == 60 {
			log.Printf("not starting computers: %v", err)
			return
		}
		time.Sleep(5 * time.Second)
	}
	lims, err := limits(ctx, all)
	if err != nil {
		log.Print(err)
		return
	}
	for _, d := range all {
		if d.State != "stopped" || !lims[d.Name].Restart {
			continue
		}
		log.Printf("starting %s, which should be running", d.Name)
		if err := rt.start(ctx, d); err != nil {
			log.Print(err)
		}
	}
}
