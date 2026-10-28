package main

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"strings"

	"github.com/drevantonder/the-one-app/sdk"
)

// declaration is a plugin with no code: a remote or local MCP server, and
// how to sign in to it. It is all `one plugins add` needs.
type declaration struct {
	MCP       sdk.MCPServer `json:"mcp,omitempty"`
	Discovery string        `json:"discovery,omitempty"` // a Google API Discovery document
	OAuth     *struct {
		Provider string `json:"provider,omitempty"` // "google", or empty for the URLs below
		sdk.OAuthProvider
	} `json:"oauth,omitempty"`
}

// declaredPlugin builds a plugin from ONE_DECLARATION, with the same SDK
// helpers a Go plugin would use.
func declaredPlugin() (sdk.Plugin, error) {
	id := os.Getenv("ONE_PLUGIN")
	var d declaration
	if err := json.Unmarshal([]byte(os.Getenv("ONE_DECLARATION")), &d); err != nil {
		return sdk.Plugin{}, fmt.Errorf("bad declaration: %w", err)
	}
	return sdk.Plugin{ID: id, Version: "declared", Setup: func(c *sdk.Ctx) error {
		var account *sdk.Account
		if d.OAuth != nil {
			p := d.OAuth.OAuthProvider
			if d.OAuth.Provider == "google" {
				p = sdk.Google(p.Scopes...)
			}
			if p.AuthURL == "" || p.TokenURL == "" {
				return errors.New("oauth needs a provider, or authUrl and tokenUrl")
			}
			account = sdk.OAuth(c, p)
		}
		if d.Discovery != "" {
			sdk.ImportDiscovery(c, d.Discovery, account)
			return nil
		}
		sdk.ImportMCP(c, d.MCP, account)
		return nil
	}}, nil
}

// addPlugin turns `one plugins add` flags into a declaration.
func addPlugin(args []string) error {
	if len(args) < 1 || strings.HasPrefix(args[0], "-") {
		return errors.New("usage: one plugins add ID (--mcp URL | --command CMD | --discovery URL) [--oauth google | --auth-url U --token-url U] [--scopes a,b] [--header K=V]")
	}
	id := args[0]
	fs := flag.NewFlagSet("add", flag.ContinueOnError)
	url := fs.String("mcp", "", "remote MCP server URL")
	discovery := fs.String("discovery", "", "Google API Discovery document URL")
	command := fs.String("command", "", "local MCP server command, run on each device")
	provider := fs.String("oauth", "", "OAuth provider: google")
	authURL := fs.String("auth-url", "", "OAuth authorization URL")
	tokenURL := fs.String("token-url", "", "OAuth token URL")
	scopes := fs.String("scopes", "", "OAuth scopes, comma separated")
	var headers headerFlag
	fs.Var(&headers, "header", "header to send, K=V (repeatable)")
	if err := fs.Parse(args[1:]); err != nil {
		return err
	}
	var d declaration
	d.MCP = sdk.MCPServer{URL: *url, Headers: headers.m}
	if *command != "" {
		d.MCP.Command = strings.Fields(*command)
	}
	d.Discovery = *discovery
	if d.MCP.URL == "" && len(d.MCP.Command) == 0 && d.Discovery == "" {
		return errors.New("give --mcp URL, --command CMD or --discovery URL")
	}
	if *provider != "" || *authURL != "" {
		d.OAuth = &struct {
			Provider string `json:"provider,omitempty"`
			sdk.OAuthProvider
		}{Provider: *provider, OAuthProvider: sdk.OAuthProvider{AuthURL: *authURL, TokenURL: *tokenURL}}
		if *scopes != "" {
			d.OAuth.Scopes = strings.Split(*scopes, ",")
		}
	}
	spec, _ := json.Marshal(d)
	input, _ := json.Marshal(map[string]any{"id": id, "spec": json.RawMessage(spec)})
	return run("one", "plugins_add", input)
}

type headerFlag struct{ m map[string]string }

func (h *headerFlag) String() string { return "" }
func (h *headerFlag) Set(v string) error {
	k, val, ok := strings.Cut(v, "=")
	if !ok {
		return errors.New("header must be K=V")
	}
	if h.m == nil {
		h.m = map[string]string{}
	}
	h.m[k] = val
	return nil
}
