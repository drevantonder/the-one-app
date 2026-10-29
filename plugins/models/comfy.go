package models

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/big"
	"net/http"
	"net/url"
	"slices"
	"strings"
	"time"

	"github.com/drevantonder/the-one-app/sdk"
)

// comfy is a ComfyUI server. Its image models are FLUX.2 Klein diffusion
// models, for which it builds the workflow itself.
type comfy struct{ base string }

func (c comfy) get(ctx context.Context, path string, out any) error {
	req, _ := http.NewRequestWithContext(ctx, "GET", c.base+path, nil)
	return do(req, out)
}

func (c comfy) post(ctx context.Context, path string, in, out any) error {
	body, _ := json.Marshal(in)
	req, _ := http.NewRequestWithContext(ctx, "POST", c.base+path, bytes.NewReader(body))
	req.Header.Set("content-type", "application/json")
	return do(req, out)
}

// options lists the files a loader node's input can name.
func (c comfy) options(ctx context.Context, node, input string) ([]string, error) {
	var info map[string]struct {
		Input struct {
			Required map[string][]json.RawMessage `json:"required"`
		} `json:"input"`
	}
	if err := c.get(ctx, "/object_info/"+node, &info); err != nil {
		return nil, err
	}
	in := info[node].Input.Required[input]
	if len(in) == 0 {
		return nil, nil
	}
	var names []string
	if json.Unmarshal(in[0], &names) != nil { // newer ComfyUI: ["COMBO", {"options": [...]}]
		var opts struct {
			Options []string `json:"options"`
		}
		if len(in) > 1 {
			_ = json.Unmarshal(in[1], &opts)
		}
		names = opts.Options
	}
	return names, nil
}

func isKlein(name string) bool {
	n := strings.ToLower(name)
	return strings.Contains(n, "klein") && (strings.Contains(n, "flux-2") || strings.Contains(n, "flux2") || strings.Contains(n, "flux.2"))
}

func (c comfy) models(ctx context.Context) ([]string, error) {
	var out []string
	for _, node := range []string{"UnetLoaderGGUF", "UNETLoader"} {
		names, err := c.options(ctx, node, "unet_name")
		if err != nil {
			if node == "UNETLoader" {
				return nil, err
			}
			continue // no ComfyUI-GGUF
		}
		for _, n := range names {
			if isKlein(n) && !slices.Contains(out, n) {
				out = append(out, n)
			}
		}
	}
	return out, nil
}

// free unloads ComfyUI's models from the GPU, if it is running.
func (c comfy) free(ctx context.Context) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	_ = c.post(ctx, "/free", map[string]bool{"unload_models": true, "free_memory": true}, nil)
}

// pick is the first of names containing any of want, in want's order.
func pick(names []string, want ...string) string {
	for _, w := range want {
		for _, n := range names {
			if strings.Contains(strings.ToLower(n), w) {
				return n
			}
		}
	}
	return ""
}

func (c comfy) image(ctx context.Context, in imageIn) (sdk.Image, error) {
	if strings.TrimSpace(in.Prompt) == "" {
		return sdk.Image{}, errors.New("give a prompt")
	}
	models, err := c.models(ctx)
	if err != nil {
		return sdk.Image{}, errors.New("ComfyUI isn't running on this device")
	}
	model := in.Model
	if model == "" && len(models) > 0 {
		model = models[0]
	}
	if !slices.Contains(models, model) {
		return sdk.Image{}, fmt.Errorf("no FLUX.2 Klein model named %q in ComfyUI here; it has: %s", model, strings.Join(models, ", "))
	}
	encoders, err := c.options(ctx, "CLIPLoader", "clip_name")
	if err != nil {
		return sdk.Image{}, err
	}
	vaes, err := c.options(ctx, "VAELoader", "vae_name")
	if err != nil {
		return sdk.Image{}, err
	}
	// Klein 4B pairs with Qwen3 4B, 9B with Qwen3 8B.
	size := "qwen_3_8b"
	if strings.Contains(strings.ToLower(model), "4b") {
		size = "qwen_3_4b"
	}
	encoder, vae := pick(encoders, size), pick(vaes, "flux2-vae", "flux2", "full_encoder_small_decoder")
	if encoder == "" || vae == "" {
		return sdk.Image{}, fmt.Errorf("%s needs a %s text encoder in models/text_encoders and the FLUX.2 VAE in models/vae: see https://huggingface.co/Comfy-Org/flux2-klein-9B", model, size)
	}
	if in.Seed == 0 {
		n, _ := rand.Int(rand.Reader, big.NewInt(1<<48))
		in.Seed = n.Int64()
	}
	w, h := max(in.Width, 0), max(in.Height, 0)
	if w == 0 {
		w = 1024
	}
	if h == 0 {
		h = 1024
	}
	return c.run(ctx, klein(model, encoder, vae, in.Prompt, w/16*16, h/16*16, in.Seed))
}

// klein is the text-to-image workflow for a FLUX.2 Klein model, in
// ComfyUI's API format. The distilled models take 4 steps without
// guidance; the base ones need more steps and guidance.
func klein(model, encoder, vae, prompt string, w, h int, seed int64) map[string]any {
	steps, cfg := 4, 1.0
	if strings.Contains(strings.ToLower(model), "base") {
		steps, cfg = 28, 4.0
	}
	loader := "UNETLoader"
	unet := map[string]any{"unet_name": model, "weight_dtype": "default"}
	if strings.HasSuffix(strings.ToLower(model), ".gguf") {
		loader, unet = "UnetLoaderGGUF", map[string]any{"unet_name": model}
	}
	node := func(class string, inputs map[string]any) map[string]any {
		return map[string]any{"class_type": class, "inputs": inputs}
	}
	link := func(id string) []any { return []any{id, 0} }
	return map[string]any{
		"model":    node(loader, unet),
		"clip":     node("CLIPLoader", map[string]any{"clip_name": encoder, "type": "flux2", "device": "default"}),
		"vae":      node("VAELoader", map[string]any{"vae_name": vae}),
		"positive": node("CLIPTextEncode", map[string]any{"text": prompt, "clip": link("clip")}),
		"negative": node("ConditioningZeroOut", map[string]any{"conditioning": link("positive")}),
		"guider":   node("CFGGuider", map[string]any{"model": link("model"), "positive": link("positive"), "negative": link("negative"), "cfg": cfg}),
		"latent":   node("EmptyFlux2LatentImage", map[string]any{"width": w, "height": h, "batch_size": 1}),
		"sigmas":   node("Flux2Scheduler", map[string]any{"steps": steps, "width": w, "height": h}),
		"sampler":  node("KSamplerSelect", map[string]any{"sampler_name": "euler"}),
		"noise":    node("RandomNoise", map[string]any{"noise_seed": seed}),
		"sample":   node("SamplerCustomAdvanced", map[string]any{"noise": link("noise"), "guider": link("guider"), "sampler": link("sampler"), "sigmas": link("sigmas"), "latent_image": link("latent")}),
		"decode":   node("VAEDecode", map[string]any{"samples": link("sample"), "vae": link("vae")}),
		"save":     node("SaveImage", map[string]any{"images": link("decode"), "filename_prefix": "one"}),
	}
}

// run queues a workflow and waits for its first image.
func (c comfy) run(ctx context.Context, workflow map[string]any) (sdk.Image, error) {
	b := make([]byte, 8)
	_, _ = rand.Read(b)
	var queued struct {
		PromptID   string         `json:"prompt_id"`
		NodeErrors map[string]any `json:"node_errors"`
	}
	if err := c.post(ctx, "/prompt", map[string]any{"prompt": workflow, "client_id": hex.EncodeToString(b)}, &queued); err != nil {
		return sdk.Image{}, err
	}
	ctx, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	for {
		var history map[string]struct {
			Status struct {
				Status   string  `json:"status_str"`
				Messages [][]any `json:"messages"`
			} `json:"status"`
			Outputs map[string]struct {
				Images []struct {
					Filename  string `json:"filename"`
					Subfolder string `json:"subfolder"`
					Type      string `json:"type"`
				} `json:"images"`
			} `json:"outputs"`
		}
		if err := c.get(ctx, "/history/"+queued.PromptID, &history); err != nil {
			return sdk.Image{}, err
		}
		if h, ok := history[queued.PromptID]; ok {
			if h.Status.Status == "error" {
				return sdk.Image{}, fmt.Errorf("ComfyUI failed: %v", h.Status.Messages)
			}
			for _, o := range h.Outputs {
				for _, img := range o.Images {
					return c.view(ctx, img.Filename, img.Subfolder, img.Type)
				}
			}
		}
		select {
		case <-ctx.Done():
			return sdk.Image{}, ctx.Err()
		case <-time.After(500 * time.Millisecond):
		}
	}
}

func (c comfy) view(ctx context.Context, name, subfolder, typ string) (sdk.Image, error) {
	q := url.Values{"filename": {name}, "subfolder": {subfolder}, "type": {typ}}
	req, _ := http.NewRequestWithContext(ctx, "GET", c.base+"/view?"+q.Encode(), nil)
	res, err := http.DefaultClient.Do(req)
	if err != nil {
		return sdk.Image{}, err
	}
	defer res.Body.Close()
	if res.StatusCode >= 300 {
		return sdk.Image{}, httpError(res)
	}
	data, err := io.ReadAll(res.Body)
	return sdk.Image{MIMEType: "image/png", Data: data}, err
}
