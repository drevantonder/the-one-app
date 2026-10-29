package models

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"slices"
	"strings"
	"sync"
	"time"
)

// openAI is an OpenAI-compatible chat API: Ollama's, Apple's fm server's,
// or a service's.
type openAI struct {
	baseURL string
	key     string
	http    *http.Client
}

type chatRequest struct {
	Model           string          `json:"model"`
	Messages        []message       `json:"messages"`
	MaxTokens       int             `json:"max_tokens,omitempty"`
	ReasoningEffort string          `json:"reasoning_effort,omitempty"`
	ResponseFormat  *responseFormat `json:"response_format,omitempty"`
	Stream          bool            `json:"stream"`
}

type responseFormat struct {
	Type       string     `json:"type"`
	JSONSchema jsonSchema `json:"json_schema"`
}

type jsonSchema struct {
	Name   string          `json:"name"`
	Schema json.RawMessage `json:"schema"`
	Strict bool            `json:"strict"`
}

func (o openAI) do(req *http.Request, out any) error {
	req.Header.Set("user-agent", "the-one-app/"+version)
	if o.key != "" {
		req.Header.Set("authorization", "Bearer "+o.key)
	}
	if o.http == nil {
		return do(req, out)
	}
	res, err := o.http.Do(req)
	if err != nil {
		return err
	}
	defer res.Body.Close()
	if res.StatusCode >= 300 {
		return httpError(res)
	}
	return json.NewDecoder(res.Body).Decode(out)
}

func (o openAI) chat(ctx context.Context, in chatRequest) (string, error) {
	body, _ := json.Marshal(in)
	req, _ := http.NewRequestWithContext(ctx, "POST", o.baseURL+"/chat/completions", bytes.NewReader(body))
	req.Header.Set("content-type", "application/json")
	req.Header.Set("x-opencode-session", session(in.Messages))
	var out struct {
		Choices []struct {
			Message struct {
				Content string `json:"content"`
			} `json:"message"`
		} `json:"choices"`
	}
	if err := o.do(req, &out); err != nil {
		return "", err
	}
	if len(out.Choices) == 0 || strings.TrimSpace(out.Choices[0].Message.Content) == "" {
		return "", errors.New("the model gave no answer; if it reasons, its reasoning may have used up maxTokens")
	}
	return strings.TrimSpace(out.Choices[0].Message.Content), nil
}

func (o openAI) healthy(ctx context.Context) bool {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	req, _ := http.NewRequestWithContext(ctx, "GET", o.baseURL+"/models", nil)
	var out any
	return o.do(req, &out) == nil
}

func (o openAI) models(ctx context.Context) ([]string, error) {
	req, _ := http.NewRequestWithContext(ctx, "GET", o.baseURL+"/models", nil)
	var out struct {
		Data []struct {
			ID string `json:"id"`
		} `json:"data"`
	}
	if err := o.do(req, &out); err != nil {
		return nil, err
	}
	names := make([]string, len(out.Data))
	for i, m := range out.Data {
		names[i] = m.ID
	}
	slices.Sort(names)
	return names, nil
}

// services' model lists change rarely, so they're kept for a while.
var serviceCache = struct {
	sync.Mutex
	m map[provider]cached
}{m: map[provider]cached{}}

type cached struct {
	names []string
	at    time.Time
}

func serviceModels(ctx context.Context, pr provider) ([]string, error) {
	serviceCache.Lock()
	c, ok := serviceCache.m[pr]
	serviceCache.Unlock()
	if ok && time.Since(c.at) < 10*time.Minute {
		return c.names, nil
	}
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	names, err := openAI{baseURL: pr.BaseURL, key: pr.Key}.models(ctx)
	if err != nil {
		return nil, err
	}
	serviceCache.Lock()
	serviceCache.m[pr] = cached{names, time.Now()}
	serviceCache.Unlock()
	return names, nil
}

// session names a conversation by how it starts, so each turn of it gets
// the same ID: OpenCode routes and caches prompts by it.
func session(msgs []message) string {
	h := sha256.New()
	for i, m := range msgs {
		if i == 2 {
			break
		}
		h.Write([]byte(m.Role + "\x00" + m.Content + "\x00"))
	}
	return hex.EncodeToString(h.Sum(nil))[:32]
}
