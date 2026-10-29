package core

import (
	"context"
	_ "embed"
	"encoding/json"
	"slices"
	"strings"

	"github.com/drevantonder/the-one-app/protocol"
)

const pluginsView = "ui://one/plugins"

//go:embed plugins.html
var pluginsHTML string

func (n *Node) addCoreViews() {
	n.AddView(protocol.Resource{URI: pluginsView, Name: "Plugins", MIMEType: protocol.MIMEApp, Text: pluginsHTML})
	n.extraOps = append(n.extraOps, coreOp{Op{CoreID, "menubar", "The menu bar's items: views of the core and plugins, in order.", schemaFor[noIn](), true, &protocol.OperationUI{Visibility: []string{"app"}}},
		func(ctx context.Context, _ json.RawMessage, _ protocol.Caller) (protocol.Result, error) {
			doc, err := n.Menubar(ctx)
			if err != nil {
				return protocol.Result{}, err
			}
			b, _ := json.Marshal(doc)
			return protocol.Result{Text: string(b), Data: doc}, nil
		}})
}

// AddView adds one of the core's MCP Apps views, for parts of the node
// outside this package.
func (n *Node) AddView(r protocol.Resource) {
	n.mu.Lock()
	n.views = append(n.views, r)
	n.mu.Unlock()
	n.changed()
}

// SetUI sets the MCP Apps metadata of a core operation added with AddCoreOp.
func (n *Node) SetUI(op string, ui *protocol.OperationUI) {
	n.mu.Lock()
	for i := range n.extraOps {
		if n.extraOps[i].Name == op {
			n.extraOps[i].UI = ui
		}
	}
	n.mu.Unlock()
	n.changed()
}

// AddMenubarItem adds one of the core's own menu bar items. They come first,
// in the order added, then Plugins; plugins' transforms run on the result.
func (n *Node) AddMenubarItem(item protocol.MenubarItem) {
	n.mu.Lock()
	n.menubar = append(n.menubar, item)
	n.mu.Unlock()
	n.changed()
}

// Menubar builds the menu bar: the core's items, then each plugin's
// transform in order.
func (n *Node) Menubar(ctx context.Context) (protocol.MenubarDoc, error) {
	n.mu.RLock()
	base := protocol.MenubarDoc{Items: append(slices.Clone(n.menubar), protocol.MenubarItem{ID: "one.plugins", Tool: "plugins_list", Resource: pluginsView})}
	n.mu.RUnlock()
	raw, err := n.transform(ctx, protocol.TransformMenubar, base, nil)
	if err != nil {
		return protocol.MenubarDoc{}, err
	}
	var doc protocol.MenubarDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return protocol.MenubarDoc{}, err
	}
	if doc.Items == nil {
		doc.Items = []protocol.MenubarItem{}
	}
	return doc, nil
}

// LocalResources lists the resources of plugins running on this device.
// A plugin may only serve URIs under ui://<its id>/.
func (n *Node) LocalResources() []protocol.Resource {
	var out []protocol.Resource
	for _, g := range n.running() {
		for _, r := range g.manifest.Resources {
			if strings.HasPrefix(r.URI, "ui://"+g.id+"/") {
				out = append(out, r)
			}
		}
	}
	return out
}

// SetRemoteResources replaces the resources other devices offer.
func (n *Node) SetRemoteResources(rs []protocol.Resource) {
	n.mu.Lock()
	same := slices.EqualFunc(rs, n.remoteUI, func(a, b protocol.Resource) bool { return a.URI == b.URI && a.Text == b.Text })
	n.remoteUI = rs
	n.mu.Unlock()
	if !same {
		n.changed()
	}
}

// Resources lists every resource once: the core's, this device's plugins',
// then other devices'.
func (n *Node) Resources() []protocol.Resource {
	n.mu.RLock()
	all := slices.Concat(n.views, nil)
	remote := slices.Clone(n.remoteUI)
	n.mu.RUnlock()
	all = append(all, n.LocalResources()...)
	all = append(all, remote...)
	var out []protocol.Resource
	seen := map[string]bool{}
	for _, r := range all {
		if !seen[r.URI] {
			seen[r.URI] = true
			out = append(out, r)
		}
	}
	return out
}

// Resource finds a resource by URI.
func (n *Node) Resource(uri string) (protocol.Resource, bool) {
	for _, r := range n.Resources() {
		if r.URI == uri {
			return r, true
		}
	}
	return protocol.Resource{}, false
}
