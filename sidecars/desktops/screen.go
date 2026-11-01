package main

import (
	_ "embed"
	"encoding/json"
	"fmt"
)

// The Desktops page: every desktop as a live picture, what each uses, who
// borrowed it, and a button to sign in to sites. The node serves it as this
// sidecar's screen (SPEC.md, "Sidecars"); it calls this sidecar's operations.

//go:embed page.html
var page string

var screens = []map[string]any{{"name": "main", "title": "Desktops"}}

// screenBody answers the node's _screen: the body of the screen named.
func screenBody(raw json.RawMessage) (any, error) {
	var in struct{ Name string }
	json.Unmarshal(raw, &in)
	if in.Name != "main" {
		return nil, fmt.Errorf("no screen %s", in.Name)
	}
	return page, nil
}
