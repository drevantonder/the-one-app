package core

import (
	"context"
	"encoding/json"
	"errors"
	"slices"
	"sync"
	"time"

	"github.com/drevantonder/the-one-app/protocol"
	"github.com/drevantonder/the-one-app/rpc"
)

// Coordinator gives the node a single writer for consistent data and jobs.
// Leases live in the bucket; forwarding goes over the tailnet.
type Coordinator interface {
	// Acquire returns the lease's holder, taking the lease if it is free or
	// expired. mine reports whether this device now holds it.
	Acquire(ctx context.Context, lease string) (holder string, mine bool, err error)
	// ClaimRun reports whether this device should run the job now: it holds
	// the job's lease and the job is due. Claiming records the run.
	ClaimRun(ctx context.Context, lease string, every time.Duration) (bool, error)
	// Flush makes this device's entries durable in the bucket.
	Flush(ctx context.Context) error
	// Forward runs an operation on another device's node.
	Forward(ctx context.Context, device, plugin, op string, input json.RawMessage, caller protocol.Caller) (protocol.Result, error)
}

// SetCoordinator enables consistent mode and scheduled jobs. Without one,
// consistent entries are written locally and jobs run on every device.
func (n *Node) SetCoordinator(c Coordinator) {
	n.mu.Lock()
	n.coord = c
	n.mu.Unlock()
}

func (n *Node) coordinator() Coordinator {
	n.mu.RLock()
	defer n.mu.RUnlock()
	return n.coord
}

func modeOf(m protocol.Manifest, typ string) protocol.Mode {
	for _, e := range m.Entries {
		if e.Type == typ {
			return e.Mode
		}
	}
	return protocol.Available
}

// checkConsistent decides whether this device may append a consistent entry.
func (n *Node) checkConsistent(ctx context.Context, plugin, typ string) error {
	n.mu.RLock()
	g := n.gens[plugin]
	n.mu.RUnlock()
	if g == nil || modeOf(g.manifest, typ) != protocol.Consistent {
		return nil
	}
	c := n.coordinator()
	if c == nil {
		return nil
	}
	holder, mine, err := c.Acquire(ctx, "plugin/"+plugin)
	if err != nil {
		return err
	}
	if !mine {
		return &rpc.Error{Code: protocol.ErrNotHolder, Message: holder}
	}
	return nil
}

func notHolder(err error) (string, bool) {
	var re *rpc.Error
	if errors.As(err, &re) && re.Code == protocol.ErrNotHolder {
		return re.Message, true
	}
	return "", false
}

// runSchedules runs plugin jobs when due, each on exactly one device.
func (n *Node) runSchedules(ctx context.Context) {
	var mu sync.Mutex
	running := map[string]bool{}
	localLast := map[string]time.Time{}
	t := time.NewTicker(10 * time.Second)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
		}
		for _, g := range n.running() {
			for _, s := range g.manifest.Schedules {
				name := g.id + "/" + s.Name
				every := time.Duration(s.EveryMs) * time.Millisecond
				mu.Lock()
				busy := running[name]
				mu.Unlock()
				if busy || every <= 0 {
					continue
				}
				due := false
				if c := n.coordinator(); c != nil {
					ok, err := c.ClaimRun(ctx, "job/"+name, every)
					if err != nil {
						n.log.Warn("job lease", "job", name, "err", err)
						continue
					}
					due = ok
				} else if time.Since(localLast[name]) >= every {
					due, localLast[name] = true, time.Now()
				}
				if !due {
					continue
				}
				mu.Lock()
				running[name] = true
				mu.Unlock()
				go func(plugin, schedule, name string) {
					defer func() { mu.Lock(); delete(running, name); mu.Unlock() }()
					g, release, err := n.acquire(plugin)
					if err != nil {
						return
					}
					defer release()
					n.log.Info("running job", "job", name)
					if err := g.conn.Call(ctx, protocol.MethodRun, protocol.RunParams{Schedule: schedule}, nil); err != nil {
						n.log.Error("job failed", "job", name, "err", err)
					}
				}(g.id, s.Name, name)
			}
		}
	}
}

// Schedules lists every running plugin's jobs, for status.
func (n *Node) Schedules() []string {
	var out []string
	for _, g := range n.running() {
		for _, s := range g.manifest.Schedules {
			out = append(out, g.id+"/"+s.Name)
		}
	}
	return slices.Clip(out)
}
