package core

import (
	"context"
	"slices"
	"strings"

	"github.com/drevantonder/the-one-app/protocol"
)

// orderKey is an entry's position in the merged log.
type orderKey struct {
	hlc    uint64
	device string
	seq    int64
}

func keyOf(e protocol.Entry) orderKey { return orderKey{e.HLC, e.Device, e.Seq} }

func (a orderKey) less(b orderKey) bool {
	if a.hlc != b.hlc {
		return a.hlc < b.hlc
	}
	if c := strings.Compare(a.device, b.device); c != 0 {
		return c < 0
	}
	return a.seq < b.seq
}

// Ingest stores entries from another node or the bucket and brings plugins
// up to date. Entries that sort after what a plugin has seen are applied in
// place; one that sorts earlier (a late arrival from a device that was
// offline) restarts the plugin, which replays its whole log in order.
func (n *Node) Ingest(ctx context.Context, entries []protocol.Entry) (int, error) {
	added, err := n.store.Ingest(entries)
	if err != nil || len(added) == 0 {
		return 0, err
	}
	byPlugin := map[string][]protocol.Entry{}
	for _, e := range added {
		byPlugin[e.Plugin] = append(byPlugin[e.Plugin], e)
	}
	if _, ok := byPlugin[CoreID]; ok {
		n.reconcile(ctx)
	}
	for id, list := range byPlugin {
		if id == CoreID {
			continue
		}
		n.deliver(ctx, id, list)
	}
	return len(added), nil
}

func (n *Node) deliver(ctx context.Context, id string, list []protocol.Entry) {
	g, release, err := n.acquire(id)
	if err != nil {
		return // not running here; it replays everything when it starts
	}
	late := false
	for _, e := range list {
		if keyOf(e).less(g.last) {
			late = true
			break
		}
	}
	if !late {
		sortEntries(list)
		err = g.conn.Call(ctx, protocol.MethodApply, protocol.ApplyParams{Entries: list}, nil)
		if err == nil {
			n.mu.Lock()
			g.last = keyOf(list[len(list)-1])
			n.mu.Unlock()
		}
	}
	release()
	if late || err != nil {
		if err != nil {
			n.log.Warn("apply failed; restarting plugin", "plugin", id, "err", err)
		}
		if err := n.startPlugin(ctx, id); err != nil {
			n.log.Error("plugin restart failed", "plugin", id, "err", err)
		}
	}
}

// reconcile starts and stops plugins to match the synced enabled state.
func (n *Node) reconcile(ctx context.Context) {
	n.refreshPublished(ctx)
	enabled := n.enabledState()
	for _, id := range n.sourceOrder() {
		n.mu.RLock()
		_, running := n.gens[id]
		n.mu.RUnlock()
		switch {
		case enabled[id] && !running:
			if err := n.startPlugin(ctx, id); err != nil {
				n.log.Error("plugin failed to start", "plugin", id, "err", err)
			}
		case !enabled[id] && running:
			n.stopPlugin(id)
		}
	}
}

func sortEntries(list []protocol.Entry) {
	for i := 1; i < len(list); i++ {
		for j := i; j > 0 && keyOf(list[j]).less(keyOf(list[j-1])); j-- {
			list[j], list[j-1] = list[j-1], list[j]
		}
	}
}

// refreshPublished turns published builds into plugin sources, and swaps a
// running plugin to a newly published build. A plugin linked from source on
// this device keeps running from its source.
func (n *Node) refreshPublished(ctx context.Context) {
	var restart, stop []string
	n.mu.Lock()
	declared, removed := n.declaredState()
	for id, spec := range declared {
		src := n.sources[id]
		switch {
		case src == nil:
			n.sources[id] = &source{id: id, declared: spec}
		case src.declared != nil && string(src.declared) != string(spec):
			src.declared = spec
			if n.gens[id] != nil {
				restart = append(restart, id)
			}
		}
	}
	for id := range removed {
		if src := n.sources[id]; src != nil && (src.declared != nil || src.published != nil) {
			delete(n.sources, id)
			stop = append(stop, id)
		}
	}
	for id, p := range n.publishedState() {
		if slices.Contains(n.builtins, id) || id == CoreID || removed[id] {
			continue
		}
		src := n.sources[id]
		switch {
		case src == nil:
			p := p
			n.sources[id] = &source{id: id, published: &p}
		case src.published != nil && src.published.Build != p.Build:
			p := p
			src.published = &p
			if n.gens[id] != nil {
				restart = append(restart, id)
			}
		}
	}
	n.mu.Unlock()
	for _, id := range stop {
		n.stopPlugin(id)
	}
	for _, id := range restart {
		n.log.Info("plugin changed; swapping generation", "plugin", id)
		if err := n.startPlugin(ctx, id); err != nil {
			n.log.Error("published build failed to start", "plugin", id, "err", err)
		}
	}
}
