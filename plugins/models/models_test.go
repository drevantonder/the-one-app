package models

import (
	"strings"
	"testing"
)

func TestSameModel(t *testing.T) {
	if !sameModel("qwen3:8b", "qwen3:8b") || !sameModel("gemma", "gemma:latest") || sameModel("qwen3:8b", "qwen3:4b") {
		t.Fatal("sameModel")
	}
}

func TestKlein(t *testing.T) {
	w := klein("flux-2-klein-9b-Q8_0.gguf", "qwen_3_8b.safetensors", "flux2-vae.safetensors", "a fox", 1024, 768, 7)
	if w["model"].(map[string]any)["class_type"] != "UnetLoaderGGUF" {
		t.Fatal("a GGUF model needs the GGUF loader")
	}
	if s := w["sigmas"].(map[string]any)["inputs"].(map[string]any); s["steps"] != 4 || s["height"] != 768 {
		t.Fatalf("distilled: %v", s)
	}
	base := klein("flux-2-klein-base-9b.safetensors", "e", "v", "a fox", 1024, 1024, 7)
	if base["model"].(map[string]any)["class_type"] != "UNETLoader" || base["guider"].(map[string]any)["inputs"].(map[string]any)["cfg"] != 4.0 {
		t.Fatal("base model")
	}
	if isKlein("flux1-dev.safetensors") || !isKlein("FLUX.2-Klein-4B.gguf") {
		t.Fatal("isKlein")
	}
}

func TestReport(t *testing.T) {
	r := report([]Model{
		{Name: "qwen3:8b", Device: "personal", Backend: "ollama", Kind: "chat", Size: 5e9, Loaded: true},
		{Name: "apple", Device: "mac", Backend: "apple", Kind: "chat"},
		{Name: "opencode-go/kimi-k2.6", Backend: "opencode-go", Kind: "chat"},
	}, []string{"ptc: offline"})
	for _, want := range []string{"personal:\n  qwen3:8b (ollama chat, 5.0 GB, loaded)", "mac:\n  apple", "services:\n  opencode-go/kimi-k2.6", "error: ptc: offline"} {
		if !strings.Contains(r, want) {
			t.Fatalf("missing %q in\n%s", want, r)
		}
	}
}

func TestSession(t *testing.T) {
	first := []message{{Role: "system", Content: "be brief"}, {Role: "user", Content: "hi"}}
	later := append(first, message{Role: "assistant", Content: "hello"}, message{Role: "user", Content: "more"})
	if session(first) != session(later) || session(first) == session(first[:1]) {
		t.Fatal("a conversation keeps its session, and another gets its own")
	}
}
