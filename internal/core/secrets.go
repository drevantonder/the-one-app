package core

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"slices"
	"strings"

	"github.com/drevantonder/the-one-app/protocol"
)

// Secrets are the user's API keys and the like, kept in the core's log so
// every device has them. A plugin reads one only if its manifest names it;
// no operation gives a value back, so agents can set secrets but never read
// them.

var errNoSecret = errors.New("secret not set")

var secretName = regexp.MustCompile(`^[A-Z][A-Z0-9_]*$`)

type secretIn struct {
	Name  string `json:"name" jsonschema:"the secret's name, in capitals like OPENCODE_API_KEY"`
	Value string `json:"value" jsonschema:"the secret itself"`
}

type secretNameIn struct {
	Name string `json:"name"`
}

type secretEntry struct {
	Name  string `json:"name"`
	Value string `json:"value,omitempty"`
}

// secrets folds the core's secret entries; the latest of each name wins.
func (n *Node) secrets() (map[string]string, error) {
	entries, err := n.store.Entries(CoreID)
	if err != nil {
		return nil, err
	}
	out := map[string]string{}
	for _, e := range entries {
		var s secretEntry
		if (e.Type != "secret.set" && e.Type != "secret.removed") || json.Unmarshal(e.Payload, &s) != nil {
			continue
		}
		if e.Type == "secret.set" {
			out[s.Name] = s.Value
		} else {
			delete(out, s.Name)
		}
	}
	return out, nil
}

// secret is the value of a secret for plugin id, if its manifest names it.
func (n *Node) secret(id, name string) (string, error) {
	n.mu.RLock()
	g := n.gens[id]
	allowed := g != nil && slices.Contains(g.manifest.Secrets, name)
	n.mu.RUnlock()
	if !allowed {
		return "", fmt.Errorf("plugin %s doesn't declare the secret %s", id, name)
	}
	all, err := n.secrets()
	if err != nil {
		return "", err
	}
	v, ok := all[name]
	if !ok {
		return "", fmt.Errorf("%w: %s; set it with secrets_set", errNoSecret, name)
	}
	return v, nil
}

// secretUsers is which running plugins read each secret.
func (n *Node) secretUsers() map[string][]string {
	n.mu.RLock()
	defer n.mu.RUnlock()
	out := map[string][]string{}
	for id, g := range n.gens {
		for _, s := range g.manifest.Secrets {
			out[s] = append(out[s], id)
		}
	}
	for _, ids := range out {
		slices.Sort(ids)
	}
	return out
}

type secretInfo struct {
	Name    string   `json:"name"`
	Set     bool     `json:"set"`
	Plugins []string `json:"plugins,omitempty"`
}

func (n *Node) listSecrets() ([]secretInfo, error) {
	all, err := n.secrets()
	if err != nil {
		return nil, err
	}
	users := n.secretUsers()
	var names []string
	for name := range all {
		names = append(names, name)
	}
	for name := range users {
		if _, ok := all[name]; !ok {
			names = append(names, name)
		}
	}
	slices.Sort(names)
	out := make([]secretInfo, len(names))
	for i, name := range names {
		_, set := all[name]
		out[i] = secretInfo{Name: name, Set: set, Plugins: users[name]}
	}
	return out, nil
}

func (n *Node) addSecretOps() {
	n.extraOps = append(n.extraOps,
		coreOp{Op{CoreID, "secrets_list", "List the user's secrets, such as API keys, by name: whether each is set and which plugins use it. Values are never shown.", schemaFor[noIn](), true, nil, false},
			func(context.Context, json.RawMessage, protocol.Caller) (protocol.Result, error) {
				list, err := n.listSecrets()
				if err != nil {
					return protocol.Result{}, err
				}
				var b strings.Builder
				for _, s := range list {
					b.WriteString(s.Name)
					if !s.Set {
						b.WriteString(" (not set)")
					}
					if len(s.Plugins) > 0 {
						b.WriteString(": used by " + strings.Join(s.Plugins, ", "))
					}
					b.WriteString("\n")
				}
				if len(list) == 0 {
					b.WriteString("No secrets.")
				}
				return protocol.Result{Text: strings.TrimRight(b.String(), "\n"), Data: map[string]any{"secrets": list}}, nil
			}},
		coreOp{Op{CoreID, "secrets_set", "Set a secret, such as an API key, on every device, for the plugins that use it. Setting it again replaces it.", schemaFor[secretIn](), false, nil, false},
			func(_ context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[secretIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				in.Value = strings.TrimSpace(in.Value)
				if !secretName.MatchString(in.Name) {
					return protocol.Result{}, fmt.Errorf("name the secret in capitals, digits and _, like OPENCODE_API_KEY")
				}
				if in.Value == "" {
					return protocol.Result{}, fmt.Errorf("give the secret's value")
				}
				payload, _ := json.Marshal(secretEntry{Name: in.Name, Value: in.Value})
				if _, err := n.store.Append(CoreID, "secret.set", payload, c.Actor); err != nil {
					return protocol.Result{}, err
				}
				return protocol.Result{Text: "Set " + in.Name + "."}, nil
			}},
		coreOp{Op{CoreID, "secrets_remove", "Remove a secret from every device.", schemaFor[secretNameIn](), false, nil, false},
			func(_ context.Context, raw json.RawMessage, c protocol.Caller) (protocol.Result, error) {
				in, err := decode[secretNameIn](raw)
				if err != nil {
					return protocol.Result{}, err
				}
				all, err := n.secrets()
				if err != nil {
					return protocol.Result{}, err
				}
				if _, ok := all[in.Name]; !ok {
					return protocol.Result{}, fmt.Errorf("no secret named %s", in.Name)
				}
				payload, _ := json.Marshal(secretEntry{Name: in.Name})
				if _, err := n.store.Append(CoreID, "secret.removed", payload, c.Actor); err != nil {
					return protocol.Result{}, err
				}
				return protocol.Result{Text: "Removed " + in.Name + "."}, nil
			}},
	)
}
