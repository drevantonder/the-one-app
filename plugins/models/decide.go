package models

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"slices"
	"strings"

	"github.com/drevantonder/the-one-app/sdk"
)

// deciders are decision-model services, used once the secret holding their
// key is set. They speak System One, as Ollama does for its decision models.
var deciders = []struct{ name, baseURL, secret string }{
	{"typesafe", "https://api.typesafe.ai/v1", "TYPESAFE_API_KEY"},
}

type decideIn struct {
	Model     string          `json:"model" jsonschema:"a decision model, by name as list gives it (kind decide), like typesafe/jev-latest or clef-flash"`
	State     json.RawMessage `json:"state" jsonschema:"what to decide about: an object of named fields, such as {\"post\": \"...\"}"`
	Questions json.RawMessage `json:"questions" jsonschema:"named questions, each {\"type\": \"noul\"|\"choice\"|\"score\", \"instructions\": \"...\"}; choice takes criteria, an object of option: description; score takes levels, lowest first"`
}

type decideRequest struct {
	Model     string          `json:"model"`
	State     json.RawMessage `json:"state"`
	Questions json.RawMessage `json:"questions"`
}

// systemOne is a System One decision API.
type systemOne struct {
	baseURL string
	key     string
}

func (s systemOne) decide(ctx context.Context, in decideRequest) (json.RawMessage, error) {
	body, _ := json.Marshal(in)
	req, _ := http.NewRequestWithContext(ctx, "POST", s.baseURL+"/systemone", bytes.NewReader(body))
	req.Header.Set("content-type", "application/json")
	var out json.RawMessage
	return out, openAI{key: s.key}.do(req, &out)
}

func (s systemOne) models(ctx context.Context) ([]string, error) {
	req, _ := http.NewRequestWithContext(ctx, "GET", s.baseURL+"/models", nil)
	var out struct {
		Models []struct {
			Name string `json:"name"`
		} `json:"models"`
	}
	if err := (openAI{key: s.key}).do(req, &out); err != nil {
		return nil, err
	}
	var names []string
	for _, m := range out.Models {
		names = append(names, m.Name)
	}
	slices.Sort(names)
	return names, nil
}

// decisionServices is the decision services whose key is set.
func decisionServices(call *sdk.Call) map[string]systemOne {
	out := map[string]systemOne{}
	for _, d := range deciders {
		if key, err := call.Secret(d.secret); err == nil && key != "" {
			out[d.name] = systemOne{baseURL: d.baseURL, key: key}
		}
	}
	return out
}

// decide runs a service's model from this device, a local one here, and
// otherwise on the first other device that has it.
func (p *plugin) decide(call *sdk.Call, in decideIn) (sdk.Result, error) {
	if in.Model == "" || len(in.Questions) == 0 {
		return sdk.Result{}, errors.New("give a model and questions; list shows the decision models")
	}
	req := decideRequest{Model: in.Model, State: in.State, Questions: in.Questions}
	if name, model, ok := strings.Cut(in.Model, "/"); ok {
		if s, ok := decisionServices(call)[name]; ok {
			req.Model = model
			return decision(s.decide(call.Context, req))
		}
	}
	if out, ok, err := p.local.decide(call.Context, req); ok {
		return decision(out, err)
	}
	ds, err := nodes(call)
	if err != nil {
		return sdk.Result{}, err
	}
	for _, d := range ds {
		if d.Self || !d.Online {
			continue
		}
		res, err := call.Invoke("models", "here", map[string]any{"device": d.Name})
		if err != nil || !slices.ContainsFunc(decodeModels(res), func(m Model) bool { return m.Kind == "decide" && sameModel(m.Name, in.Model) }) {
			continue
		}
		return call.Invoke("models", "decide", map[string]any{"model": in.Model, "state": in.State, "questions": in.Questions, "device": d.Name})
	}
	return sdk.Result{}, fmt.Errorf("no device or service has a decision model named %q; list shows them", in.Model)
}

// decision gives the answers as data, and as text, one line a question.
func decision(raw json.RawMessage, err error) (sdk.Result, error) {
	if err != nil {
		return sdk.Result{}, err
	}
	var out struct {
		Answers map[string]map[string]any `json:"answers"`
	}
	if err := json.Unmarshal(raw, &out); err != nil {
		return sdk.Result{}, err
	}
	var lines []string
	for name, a := range out.Answers {
		switch {
		case a["choice"] != nil:
			lines = append(lines, fmt.Sprintf("%s: %v (confidence %.2f)", name, a["choice"], a["confidence"]))
		case a["score"] != nil:
			lines = append(lines, fmt.Sprintf("%s: %.2f (confidence %.2f)", name, a["score"], a["confidence"]))
		default:
			lines = append(lines, fmt.Sprintf("%s: %.2f probability yes", name, a["noul"]))
		}
	}
	slices.Sort(lines)
	return sdk.Result{Text: strings.Join(lines, "\n"), Data: out}, nil
}
