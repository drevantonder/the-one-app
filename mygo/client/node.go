package client

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/url"
	"strings"
)

// NodeError is a failure a node said, in a person's words.
type NodeError struct{ Message string }

func (e *NodeError) Error() string { return e.Message }

// Client is the name `call`s say they came from ("windows", "mac", …), as each shell's does.
var Client = "mygo"

// Send is a request to the person's nodes, its body JSON (or none) → its response, once it's ok. A call carries an
// id, so sent again it runs once.
func (d *ThisDevice) Send(method, path string, query url.Values, body any, accept string) (*Response, error) {
	if len(query) > 0 {
		path += "?" + query.Encode()
	}
	var raw []byte
	var headers []Header
	if body != nil {
		if m, ok := body.(map[string]any); ok && strings.HasPrefix(path, "/v1/call") && m["id"] == nil {
			id := make([]byte, 8)
			rand.Read(id)
			m["id"] = hex.EncodeToString(id)
		}
		var err error
		if raw, err = json.Marshal(body); err != nil {
			return nil, err
		}
		headers = append(headers, Header{"content-type", "application/json"})
	}
	if accept != "" {
		headers = append(headers, Header{"accept", accept})
	}
	r, err := d.Fetch(method, path, headers, raw)
	if err != nil {
		return nil, err
	}
	if s := r.Status(); s >= 200 && s < 300 {
		return r, nil
	} else {
		text, _ := r.Bytes()
		var out struct{ Error string }
		if json.Unmarshal(text, &out) == nil && out.Error != "" {
			return nil, &NodeError{out.Error}
		}
		if s >= 500 {
			return nil, &NodeError{"Your devices aren’t answering."}
		}
		return nil, &NodeError{"That didn’t work. Try again."}
	}
}

// JSON is JSON in and out: a failure the node says (`{ error }`) is returned as one.
func (d *ThisDevice) JSON(method, path string, query url.Values, body any) (json.RawMessage, error) {
	r, err := d.Send(method, path, query, body, "")
	if err != nil {
		return nil, err
	}
	b, err := r.Bytes()
	if err != nil {
		return nil, err
	}
	var e struct{ Error *string }
	if json.Unmarshal(b, &e) == nil && e.Error != nil {
		return nil, &NodeError{*e.Error}
	}
	return b, nil
}

// Call runs an app's operation as this person → its result.
func (d *ThisDevice) Call(app, op string, input json.RawMessage) (json.RawMessage, error) {
	if len(input) == 0 {
		input = json.RawMessage("{}")
	}
	b, err := d.JSON("POST", "/v1/call", nil, map[string]any{"app": app, "op": op, "input": input, "client": Client})
	if err != nil {
		return nil, err
	}
	var out struct{ Ok json.RawMessage }
	if err := json.Unmarshal(b, &out); err != nil {
		return nil, err
	}
	if out.Ok == nil {
		return json.RawMessage("null"), nil
	}
	return out.Ok, nil
}

// Screen is an app's screen, as its page.
func (d *ThisDevice) Screen(app, screen, params, scope string) (string, error) {
	q := url.Values{"params": {params}}
	if scope != "" {
		q.Set("scope", scope)
	}
	r, err := d.Send("GET", "/v1/screens/"+url.PathEscape(app)+"/"+url.PathEscape(screen), q, nil, "")
	if err != nil {
		return "", err
	}
	b, err := r.Bytes()
	return string(b), err
}

// Apps are the apps the person has.
func (d *ThisDevice) Apps() ([]json.RawMessage, error) {
	b, err := d.JSON("GET", "/v1/apps", nil, nil)
	if err != nil {
		return nil, err
	}
	var out []json.RawMessage
	return out, json.Unmarshal(b, &out)
}

// API is an own screen's `one.api`: the node's /v1, as this person; nothing else.
func (d *ThisDevice) API(method, path string, body json.RawMessage) (json.RawMessage, error) {
	u, err := url.Parse(path)
	if err != nil || u.Host != "" || !strings.HasPrefix(u.Path, "/v1/") {
		return nil, &NodeError{"That isn’t on your devices."}
	}
	method = strings.ToUpper(method)
	var b any
	if method != "GET" {
		m := map[string]any{}
		if len(body) > 0 {
			if err := json.Unmarshal(body, &m); err != nil {
				return nil, err
			}
		}
		if u.Path == "/v1/call" {
			m["client"] = Client
		}
		b = m
	}
	return d.JSON(method, u.Path, u.Query(), b)
}

// Events gives the node's events as they come, until ctx is done.
func (d *ThisDevice) Events(ctx context.Context, each func(json.RawMessage)) error {
	r, err := d.Send("GET", "/v1/events", nil, nil, "text/event-stream")
	if err != nil {
		return err
	}
	stop := context.AfterFunc(ctx, r.Cancel)
	defer stop()
	var rest bytes.Buffer
	var data strings.Builder
	for ctx.Err() == nil {
		part, err := r.Chunk()
		if err != nil {
			return err
		}
		if part == nil {
			return errors.New("the node's events ended")
		}
		rest.Write(part)
		for {
			line, err := rest.ReadString('\n')
			if err != nil {
				// Not a whole line yet: kept for the next part.
				rest.Reset()
				rest.WriteString(line)
				break
			}
			line = strings.TrimRight(line, "\r\n")
			if v, ok := strings.CutPrefix(line, "data:"); ok {
				data.WriteString(strings.TrimSpace(v))
			} else if line == "" && data.Len() > 0 {
				if json.Valid([]byte(data.String())) {
					each(json.RawMessage(data.String()))
				}
				data.Reset()
			}
		}
	}
	return ctx.Err()
}
