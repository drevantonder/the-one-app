package sdk

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"sort"
	"strings"
)

// ImportDiscovery makes every method of a Google API, described by its
// Discovery document, one of the plugin's operations, called with the
// account's token. Discovery documents are public, so the operations exist
// before sign-in.
func ImportDiscovery(c *Ctx, docURL string, account *Account) {
	im := &discoveryImport{ctx: c, docURL: docURL, account: account}
	c.OnReady(func(call *Call) {
		if err := im.refresh(call.Context); err != nil {
			c.Logf("discovery: %v", err)
		}
	})
}

type discoveryDoc struct {
	RootURL     string                     `json:"rootUrl"`
	ServicePath string                     `json:"servicePath"`
	Schemas     map[string]json.RawMessage `json:"schemas"`
	Resources   map[string]discoveryNode   `json:"resources"`
	Methods     map[string]discoveryMethod `json:"methods"`
}

type discoveryNode struct {
	Resources map[string]discoveryNode   `json:"resources"`
	Methods   map[string]discoveryMethod `json:"methods"`
}

type discoveryMethod struct {
	ID          string                    `json:"id"`
	Path        string                    `json:"path"`
	HTTPMethod  string                    `json:"httpMethod"`
	Description string                    `json:"description"`
	Parameters  map[string]discoveryParam `json:"parameters"`
	Request     *struct {
		Ref string `json:"$ref"`
	} `json:"request"`
}

type discoveryParam struct {
	Type        string   `json:"type"`
	Description string   `json:"description"`
	Location    string   `json:"location"`
	Required    bool     `json:"required"`
	Repeated    bool     `json:"repeated"`
	Default     string   `json:"default"`
	Enum        []string `json:"enum"`
}

type discoveryImport struct {
	ctx     *Ctx
	docURL  string
	account *Account
	base    string
	names   []string
}

func (im *discoveryImport) refresh(ctx context.Context) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, im.docURL, nil)
	if err != nil {
		return err
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("discovery document: %s", resp.Status)
	}
	var doc discoveryDoc
	if err := json.NewDecoder(resp.Body).Decode(&doc); err != nil {
		return err
	}
	im.base = doc.RootURL + doc.ServicePath
	for _, n := range im.names {
		im.ctx.RemoveOp(n)
	}
	im.names = im.names[:0]
	var methods []discoveryMethod
	var walk func(discoveryNode)
	walk = func(n discoveryNode) {
		for _, m := range n.Methods {
			methods = append(methods, m)
		}
		for _, r := range n.Resources {
			walk(r)
		}
	}
	walk(discoveryNode{Resources: doc.Resources, Methods: doc.Methods})
	sort.Slice(methods, func(i, j int) bool { return methods[i].ID < methods[j].ID })
	for _, m := range methods {
		m := m
		name := m.ID
		if i := strings.IndexByte(name, '.'); i >= 0 {
			name = name[i+1:] // drop the API name: gmail.users.messages.list → users.messages.list
		}
		name = strings.ReplaceAll(name, ".", "_")
		schema, _ := json.Marshal(inputSchema(m, doc.Schemas))
		kind := Write
		if m.HTTPMethod == http.MethodGet {
			kind = Read
		}
		im.ctx.OpRaw(name, m.Description, schema, kind, func(call *Call, input json.RawMessage) (Result, error) {
			return im.call(call.Context, m, input)
		})
		im.names = append(im.names, name)
	}
	im.ctx.Changed()
	return nil
}

func inputSchema(m discoveryMethod, schemas map[string]json.RawMessage) map[string]any {
	props := map[string]any{}
	var required []string
	for name, p := range m.Parameters {
		s := map[string]any{"type": jsonType(p.Type), "description": p.Description}
		if len(p.Enum) > 0 {
			s["enum"] = p.Enum
		}
		if p.Default != "" {
			s["default"] = p.Default
		}
		if p.Repeated {
			s = map[string]any{"type": "array", "items": s, "description": p.Description}
		}
		props[name] = s
		if p.Required && p.Default == "" {
			required = append(required, name)
		}
	}
	if m.Request != nil && m.Request.Ref != "" {
		body := resolve(m.Request.Ref, schemas, 0)
		body["description"] = "The request body (" + m.Request.Ref + ")."
		props["body"] = body
		required = append(required, "body")
	}
	sort.Strings(required)
	out := map[string]any{"type": "object", "properties": props}
	if len(required) > 0 {
		out["required"] = required
	}
	return out
}

func jsonType(t string) string {
	switch t {
	case "integer", "number", "boolean", "array", "object":
		return t
	}
	return "string"
}

// resolve inlines a Discovery schema, cutting off deep or recursive nesting
// (which becomes a plain object) to keep tool schemas a sensible size.
func resolve(ref string, schemas map[string]json.RawMessage, depth int) map[string]any {
	if depth > 3 {
		return map[string]any{"type": "object"}
	}
	var s map[string]any
	if err := json.Unmarshal(schemas[ref], &s); err != nil {
		return map[string]any{"type": "object"}
	}
	return inline(s, schemas, depth)
}

func inline(s map[string]any, schemas map[string]json.RawMessage, depth int) map[string]any {
	out := map[string]any{}
	if r, ok := s["$ref"].(string); ok {
		return resolve(r, schemas, depth+1)
	}
	for k, v := range s {
		switch k {
		case "id", "annotations", "readOnly":
			continue
		case "type":
			out[k] = jsonType(fmt.Sprint(v))
			if v == "any" {
				delete(out, k)
			}
		case "properties":
			props := map[string]any{}
			for name, p := range v.(map[string]any) {
				if pm, ok := p.(map[string]any); ok {
					props[name] = inline(pm, schemas, depth+1)
				}
			}
			out[k] = props
		case "items", "additionalProperties":
			if pm, ok := v.(map[string]any); ok {
				out[k] = inline(pm, schemas, depth+1)
			}
		case "description", "enum", "format", "default":
			out[k] = v
		}
	}
	return out
}

var pathParam = regexp.MustCompile(`\{(\+?)([^}]+)\}`)

func (im *discoveryImport) call(ctx context.Context, m discoveryMethod, input json.RawMessage) (Result, error) {
	var in map[string]any
	if len(input) > 0 && string(input) != "null" {
		if err := json.Unmarshal(input, &in); err != nil {
			return Result{}, fmt.Errorf("bad input: %w", err)
		}
	}
	value := func(name string) (any, bool) {
		if v, ok := in[name]; ok {
			return v, true
		}
		if d := m.Parameters[name].Default; d != "" {
			return d, true
		}
		return nil, false
	}
	var missing []string
	path := pathParam.ReplaceAllStringFunc(m.Path, func(match string) string {
		g := pathParam.FindStringSubmatch(match)
		v, ok := value(g[2])
		if !ok {
			missing = append(missing, g[2])
			return match
		}
		if g[1] == "+" {
			return fmt.Sprint(v)
		}
		return url.PathEscape(fmt.Sprint(v))
	})
	if len(missing) > 0 {
		return Result{}, fmt.Errorf("missing %s", strings.Join(missing, ", "))
	}
	q := url.Values{}
	for name, p := range m.Parameters {
		if p.Location != "query" {
			continue
		}
		v, ok := in[name]
		if !ok {
			continue
		}
		if list, isList := v.([]any); isList {
			for _, item := range list {
				q.Add(name, fmt.Sprint(item))
			}
		} else {
			q.Set(name, fmt.Sprint(v))
		}
	}
	u := im.base + path
	if len(q) > 0 {
		u += "?" + q.Encode()
	}
	var body io.Reader
	if b, ok := in["body"]; ok {
		raw, _ := json.Marshal(b)
		body = bytes.NewReader(raw)
	}
	req, err := http.NewRequestWithContext(ctx, m.HTTPMethod, u, body)
	if err != nil {
		return Result{}, err
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if im.account != nil {
		token, err := im.account.Token(ctx)
		if err != nil {
			return Result{}, fmt.Errorf("%w: use sign_in first", err)
		}
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return Result{}, err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
	if err != nil {
		return Result{}, err
	}
	if resp.StatusCode >= 300 {
		var e struct {
			Error struct{ Message string } `json:"error"`
		}
		if json.Unmarshal(raw, &e) == nil && e.Error.Message != "" {
			return Result{}, fmt.Errorf("%s: %s", resp.Status, e.Error.Message)
		}
		return Result{}, errors.New(resp.Status)
	}
	if len(raw) == 0 {
		return Text("Done."), nil
	}
	var data any
	if err := json.Unmarshal(raw, &data); err != nil {
		return Text(string(raw)), nil
	}
	return Result{Data: data}, nil
}
