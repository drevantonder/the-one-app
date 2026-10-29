// Package models is the built-in plugin that makes AI models usable by
// plugins and agents across the user's devices: local ones served by Ollama,
// Apple's on-device model on Macs, image models in ComfyUI, and any
// OpenAI-compatible service the user adds a key for, such as OpenCode Go.
package models

import (
	"cmp"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"

	"github.com/drevantonder/the-one-app/sdk"
)

const version = "0.1.0"

var Plugin = sdk.Plugin{ID: "models", Name: "Models", Version: version, Platforms: []string{"linux", "darwin", "windows"}, Setup: setup}

// Model is one model a device or service can run.
type Model struct {
	Name    string `json:"name"`
	Device  string `json:"device,omitempty"` // empty for a service's, which every device reaches itself
	Backend string `json:"backend"`          // ollama, apple, comfyui, or the service's name
	Kind    string `json:"kind"`             // chat or image
	Size    int64  `json:"size,omitempty"`   // bytes on disk
	Loaded  bool   `json:"loaded,omitempty"`
}

// provider is an OpenAI-compatible service the user added a key for.
type provider struct {
	Name    string `json:"name"`
	BaseURL string `json:"baseUrl"`
	Key     string `json:"key"`
}

// presets are services that need only a key.
var presets = map[string]string{
	"opencode-go": "https://opencode.ai/zen/go/v1",
	"opencode":    "https://opencode.ai/zen/v1",
	"openrouter":  "https://openrouter.ai/api/v1",
	"openai":      "https://api.openai.com/v1",
}

type message struct {
	Role    string `json:"role" jsonschema:"system, user or assistant"`
	Content string `json:"content"`
}

type chatIn struct {
	Model     string          `json:"model" jsonschema:"the model, by name as list gives it; a service's as service/model"`
	Prompt    string          `json:"prompt,omitempty" jsonschema:"the user message, for a single turn"`
	System    string          `json:"system,omitempty" jsonschema:"a system prompt"`
	Messages  []message       `json:"messages,omitempty" jsonschema:"the conversation so far, instead of or before prompt"`
	Schema    json.RawMessage `json:"schema,omitempty" jsonschema:"a JSON Schema the answer must match, to get JSON back"`
	MaxTokens int             `json:"maxTokens,omitempty" jsonschema:"the most tokens to answer with"`
}

type imageIn struct {
	Prompt string `json:"prompt"`
	Model  string `json:"model,omitempty" jsonschema:"the image model, by name as list gives it; default the first"`
	Width  int    `json:"width,omitempty" jsonschema:"in pixels, default 1024"`
	Height int    `json:"height,omitempty" jsonschema:"in pixels, default 1024"`
	Seed   int64  `json:"seed,omitempty" jsonschema:"the same seed and prompt give the same image; default random"`
}

type modelIn struct {
	Model string `json:"model" jsonschema:"the model's name"`
}

type downloadIn struct {
	Model string `json:"model" jsonschema:"an Ollama model like qwen3:8b or gemma4:e4b, or a GGUF on Hugging Face like hf.co/LiquidAI/LFM2.5-1.2B-Instruct-GGUF:Q4_K_M"`
}

type unloadIn struct {
	Model string `json:"model,omitempty" jsonschema:"the model to unload; default every one"`
}

type addIn struct {
	Name    string `json:"name" jsonschema:"what to call the service, used before its models' names; opencode-go, opencode, openrouter and openai need no base URL"`
	Key     string `json:"key" jsonschema:"the service's API key"`
	BaseURL string `json:"baseUrl,omitempty" jsonschema:"the service's OpenAI-compatible API, ending in /v1"`
}

type removeIn struct {
	Name string `json:"name"`
}

type plugin struct {
	mu        sync.Mutex
	providers map[string]provider
	local     *local
}

func setup(c *sdk.Ctx) error {
	p := &plugin{providers: map[string]provider{}, local: newLocal(c)}
	c.Entry("provider", sdk.Available)
	c.Entry("provider.removed", sdk.Available)
	c.OnReset(func() {
		p.mu.Lock()
		p.providers = map[string]provider{}
		p.mu.Unlock()
	})
	sdk.Handle(c, "provider", func(_ sdk.Entry, in provider) error {
		p.mu.Lock()
		p.providers[in.Name] = in
		p.mu.Unlock()
		return nil
	})
	sdk.Handle(c, "provider.removed", func(_ sdk.Entry, in removeIn) error {
		p.mu.Lock()
		delete(p.providers, in.Name)
		p.mu.Unlock()
		return nil
	})

	sdk.Op(c, "list", "List every model plugins and agents can use: each device's local models (Ollama, Apple's on-device model, ComfyUI image models), whether each is loaded, and the models of each service added with add_service.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		models, errs := p.all(call)
		return sdk.Result{Text: report(models, errs), Data: map[string]any{"models": models}}, nil
	})

	sdk.Op(c, "here", "List this device's local models and downloads in progress; list shows every device's.", sdk.Read, func(call *sdk.Call, _ struct{}) (sdk.Result, error) {
		models, downloads := p.local.models(call.Context, p.me(call)), p.local.downloads()
		text := report(models, nil)
		for name, d := range downloads {
			switch {
			case d.Error != "":
				text += fmt.Sprintf("\ndownloading %s failed: %s", name, d.Error)
			case d.Total > 0:
				text += fmt.Sprintf("\ndownloading %s: %s, %.0f%% of %.1f GB", name, d.Status, float64(d.Completed)*100/float64(d.Total), float64(d.Total)/1e9)
			default:
				text += fmt.Sprintf("\ndownloading %s: %s", name, d.Status)
			}
		}
		return sdk.Result{Text: text, Data: map[string]any{"models": models, "downloads": downloads}}, nil
	})

	sdk.Op(c, "chat", "Ask a model and get its answer. Runs on whichever device has the model; a service's models run from this device. With schema, the answer is JSON matching it.", sdk.Read, func(call *sdk.Call, in chatIn) (sdk.Result, error) {
		return p.chat(call, in)
	})

	sdk.Op(c, "image", "Make an image from a prompt with an image model in ComfyUI on this device. Other models are unloaded from the GPU first.", sdk.Read, func(call *sdk.Call, in imageIn) (sdk.Result, error) {
		img, err := p.local.image(call.Context, in)
		if err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: "Made an image of: " + in.Prompt, Images: []sdk.Image{img}}, nil
	})

	sdk.Op(c, "download", "Download a model to this device with Ollama. It carries on in the background; here shows its progress.", sdk.Write, func(call *sdk.Call, in downloadIn) (sdk.Result, error) {
		if err := p.local.download(in.Model); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Downloading " + in.Model + " on " + p.me(call) + "."), nil
	})

	sdk.Op(c, "delete", "Delete a downloaded Ollama model from this device, freeing its disk space.", sdk.Write, func(call *sdk.Call, in modelIn) (sdk.Result, error) {
		if err := p.local.ollama.delete(call.Context, in.Model); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Deleted " + in.Model + " from " + p.me(call) + "."), nil
	})

	sdk.Op(c, "unload", "Unload models from this device's memory now, rather than after five idle minutes.", sdk.Write, func(call *sdk.Call, in unloadIn) (sdk.Result, error) {
		if err := p.local.unload(call.Context, in.Model); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Unloaded."), nil
	})

	sdk.Op(c, "add_service", "Add an OpenAI-compatible model service with its API key, such as an OpenCode Go key, so its models can be used from every device. Adding one with the same name replaces it.", sdk.Write, func(call *sdk.Call, in addIn) (sdk.Result, error) {
		pr := provider{Name: strings.TrimSpace(in.Name), Key: strings.TrimSpace(in.Key), BaseURL: strings.TrimRight(cmp.Or(in.BaseURL, presets[in.Name]), "/")}
		switch {
		case pr.Name == "" || strings.Contains(pr.Name, "/"):
			return sdk.Result{}, errors.New("give the service a name without a /")
		case pr.BaseURL == "":
			return sdk.Result{}, fmt.Errorf("give %s's base URL", pr.Name)
		case pr.Key == "":
			return sdk.Result{}, errors.New("give the service's API key")
		}
		models, err := serviceModels(call.Context, pr)
		if err != nil {
			return sdk.Result{}, fmt.Errorf("that key doesn't work: %w", err)
		}
		if _, err := call.Append("provider", pr); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Result{Text: fmt.Sprintf("Added %s, with %d models.", pr.Name, len(models)), Data: map[string]any{"models": models}}, nil
	})

	sdk.Op(c, "remove_service", "Remove a model service and its key.", sdk.Write, func(call *sdk.Call, in removeIn) (sdk.Result, error) {
		if _, ok := p.provider(in.Name); !ok {
			return sdk.Result{}, fmt.Errorf("no service named %q", in.Name)
		}
		if _, err := call.Append("provider.removed", in); err != nil {
			return sdk.Result{}, err
		}
		return sdk.Text("Removed " + in.Name + "."), nil
	})

	c.Instructions(func(text string) string {
		return text + "\n\nModels: to use an AI model for a task, such as a local model for private or bulk work or an image model, call models_list, then models_chat or models_image."
	})
	return nil
}

func (p *plugin) provider(name string) (provider, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	pr, ok := p.providers[name]
	return pr, ok
}

func (p *plugin) providerList() []provider {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := make([]provider, 0, len(p.providers))
	for _, pr := range p.providers {
		out = append(out, pr)
	}
	slices.SortFunc(out, func(a, b provider) int { return strings.Compare(a.Name, b.Name) })
	return out
}

// chat runs on this device if the model is a service's or local, and
// otherwise on the first other device that has it.
func (p *plugin) chat(call *sdk.Call, in chatIn) (sdk.Result, error) {
	if in.Model == "" {
		return sdk.Result{}, errors.New("give a model; list shows them")
	}
	msgs := in.Messages
	if in.System != "" {
		msgs = append([]message{{Role: "system", Content: in.System}}, msgs...)
	}
	if in.Prompt != "" {
		msgs = append(msgs, message{Role: "user", Content: in.Prompt})
	}
	if len(msgs) == 0 {
		return sdk.Result{}, errors.New("give a prompt or messages")
	}
	req := chatRequest{Messages: msgs, MaxTokens: in.MaxTokens}
	if len(in.Schema) > 0 {
		req.ResponseFormat = &responseFormat{Type: "json_schema", JSONSchema: jsonSchema{Name: "answer", Schema: in.Schema, Strict: true}}
	}
	if name, model, ok := strings.Cut(in.Model, "/"); ok {
		if pr, ok := p.provider(name); ok {
			req.Model = model
			return answer(openAI{baseURL: pr.BaseURL, key: pr.Key}.chat(call.Context, req))
		}
	}
	if text, ok, err := p.local.chat(call.Context, in.Model, req); ok {
		return answer(text, err)
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
		if err != nil || !slices.ContainsFunc(decodeModels(res), func(m Model) bool { return m.Kind == "chat" && sameModel(m.Name, in.Model) }) {
			continue
		}
		var fwd map[string]any
		raw, _ := json.Marshal(in)
		_ = json.Unmarshal(raw, &fwd)
		fwd["device"] = d.Name
		return call.Invoke("models", "chat", fwd)
	}
	return sdk.Result{}, fmt.Errorf("no device or service has a model named %q; list shows them", in.Model)
}

func answer(text string, err error) (sdk.Result, error) {
	if err != nil {
		return sdk.Result{}, err
	}
	return sdk.Text(text), nil
}

// all lists every online device's models, then every service's.
func (p *plugin) all(call *sdk.Call) ([]Model, []string) {
	var errs []string
	ds, err := nodes(call)
	if err != nil {
		errs = append(errs, err.Error())
	}
	providers := p.providerList()
	lists := make([][]Model, len(ds)+len(providers))
	var wg sync.WaitGroup
	var mu sync.Mutex
	fail := func(who string, err error) {
		mu.Lock()
		errs = append(errs, who+": "+err.Error())
		mu.Unlock()
	}
	for i, d := range ds {
		if !d.Online {
			continue
		}
		wg.Go(func() {
			if d.Self {
				lists[i] = p.local.models(call.Context, d.Name)
				return
			}
			res, err := call.Invoke("models", "here", map[string]any{"device": d.Name})
			if err != nil {
				fail(d.Name, err)
				return
			}
			lists[i] = decodeModels(res)
		})
	}
	for j, pr := range providers {
		wg.Go(func() {
			names, err := serviceModels(call.Context, pr)
			if err != nil {
				fail(pr.Name, err)
				return
			}
			for _, n := range names {
				lists[len(ds)+j] = append(lists[len(ds)+j], Model{Name: pr.Name + "/" + n, Backend: pr.Name, Kind: "chat"})
			}
		})
	}
	wg.Wait()
	var out []Model
	for _, l := range lists {
		out = append(out, l...)
	}
	return out, errs
}

func decodeModels(res sdk.Result) []Model {
	raw, _ := json.Marshal(res.Data)
	var d struct {
		Models []Model `json:"models"`
	}
	_ = json.Unmarshal(raw, &d)
	return d.Models
}

// sameModel matches Ollama's names with or without the default :latest tag.
func sameModel(a, b string) bool {
	return strings.TrimSuffix(a, ":latest") == strings.TrimSuffix(b, ":latest")
}

func report(models []Model, errs []string) string {
	var b strings.Builder
	where := ""
	for i, m := range models {
		w := cmp.Or(m.Device, "services")
		if i == 0 || w != where {
			where = w
			fmt.Fprintf(&b, "%s:\n", w)
		}
		fmt.Fprintf(&b, "  %s (%s %s", m.Name, m.Backend, m.Kind)
		if m.Size > 0 {
			fmt.Fprintf(&b, ", %.1f GB", float64(m.Size)/1e9)
		}
		if m.Loaded {
			b.WriteString(", loaded")
		}
		b.WriteString(")\n")
	}
	if len(models) == 0 {
		b.WriteString("No models. Download one with download, or add a service's key with add_service.\n")
	}
	for _, e := range errs {
		fmt.Fprintf(&b, "error: %s\n", e)
	}
	return strings.TrimRight(b.String(), "\n")
}

type onlineDevice struct {
	Name   string `json:"name"`
	Self   bool   `json:"self"`
	Online bool   `json:"online"`
}

func nodes(call *sdk.Call) ([]onlineDevice, error) {
	res, err := call.Invoke("one", "devices", struct{}{})
	if err != nil {
		return nil, err
	}
	raw, _ := json.Marshal(res.Data)
	var d struct {
		Devices []onlineDevice `json:"devices"`
	}
	return d.Devices, json.Unmarshal(raw, &d)
}

// me is this device's name: the hostname until the node joins the tailnet.
func (p *plugin) me(call *sdk.Call) string {
	ds, _ := nodes(call)
	for _, d := range ds {
		if d.Self {
			return d.Name
		}
	}
	return ""
}
