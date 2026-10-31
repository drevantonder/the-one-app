package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync/atomic"
)

// conn is an MCP session with a desktop's server, over streamable HTTP. The
// browsers its tools launch belong to the session that launched them, so a
// desktop keeps one for as long as this runs.
type conn struct {
	url, token string
	id         string // the session's, from the server
	next       atomic.Int64
}

// errGone is a session the server no longer knows, as after it restarted.
var errGone = errors.New("the desktop's session ended")

type tool struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	InputSchema json.RawMessage `json:"inputSchema"`
}

func dial(ctx context.Context, d desktop) (*conn, error) {
	c := &conn{url: d.local() + "/mcp", token: d.token}
	_, err := c.rpc(ctx, "initialize", map[string]any{"protocolVersion": "2025-06-18", "capabilities": map[string]any{},
		"clientInfo": map[string]any{"name": app, "version": "1"}})
	if err != nil {
		return nil, err
	}
	_, err = c.send(ctx, map[string]any{"jsonrpc": "2.0", "method": "notifications/initialized"})
	return c, err
}

func (c *conn) tools(ctx context.Context) ([]tool, error) {
	raw, err := c.rpc(ctx, "tools/list", nil)
	if err != nil {
		return nil, err
	}
	var r struct{ Tools []tool }
	return r.Tools, json.Unmarshal(raw, &r)
}

// result is a tool's answer, as MCP gives it.
type result struct {
	Content    []map[string]any `json:"content"`
	Structured json.RawMessage  `json:"structuredContent,omitempty"`
	IsError    bool             `json:"isError,omitempty"`
}

func (c *conn) call(ctx context.Context, name string, args map[string]any) (result, error) {
	var r result
	raw, err := c.rpc(ctx, "tools/call", map[string]any{"name": name, "arguments": args})
	if err == nil {
		err = json.Unmarshal(raw, &r)
	}
	return r, err
}

func (c *conn) rpc(ctx context.Context, method string, params any) (json.RawMessage, error) {
	id := c.next.Add(1)
	msg := map[string]any{"jsonrpc": "2.0", "id": id, "method": method}
	if params != nil {
		msg["params"] = params
	}
	body, err := c.send(ctx, msg)
	if err != nil {
		return nil, err
	}
	var reply struct {
		Result json.RawMessage
		Error  *struct{ Message string }
	}
	if err := json.Unmarshal(body, &reply); err != nil {
		return nil, fmt.Errorf("%s: %w", method, err)
	}
	if reply.Error != nil {
		return nil, fmt.Errorf("%s: %s", method, reply.Error.Message)
	}
	return reply.Result, nil
}

// send posts one message and returns the reply to it, if it has an id.
func (c *conn) send(ctx context.Context, msg map[string]any) ([]byte, error) {
	b, _ := json.Marshal(msg)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.url, bytes.NewReader(b))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.token)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	if c.id != "" {
		req.Header.Set("Mcp-Session-Id", c.id)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if c.id == "" { // only while dialing, before anyone else uses c
		c.id = resp.Header.Get("Mcp-Session-Id")
	}
	body, err := io.ReadAll(resp.Body)
	switch {
	case err != nil:
		return nil, err
	case resp.StatusCode == http.StatusNotFound && c.id != "":
		return nil, errGone
	case resp.StatusCode >= 300:
		return nil, fmt.Errorf("the desktop's server: %s: %s", resp.Status, strings.TrimSpace(string(body)))
	case msg["id"] == nil:
		return nil, nil
	case strings.HasPrefix(resp.Header.Get("Content-Type"), "text/event-stream"):
		return event(body)
	}
	return body, nil
}

// event is the reply in a stream of server-sent events: the first message
// with an id, skipping notifications.
func event(stream []byte) ([]byte, error) {
	for _, ev := range strings.Split(strings.ReplaceAll(string(stream), "\r\n", "\n"), "\n\n") {
		var data []string
		for _, line := range strings.Split(ev, "\n") {
			if d, ok := strings.CutPrefix(line, "data:"); ok {
				data = append(data, strings.TrimPrefix(d, " "))
			}
		}
		msg := []byte(strings.Join(data, "\n"))
		var m struct{ ID json.RawMessage }
		if json.Unmarshal(msg, &m) == nil && m.ID != nil {
			return msg, nil
		}
	}
	return nil, errors.New("the desktop's server sent no reply")
}
