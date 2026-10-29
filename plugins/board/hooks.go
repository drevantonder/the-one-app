package board

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
)

// hookPath is the node URL path the agents' prompt hook posts to.
const hookPath = "/hooks/board/news"

// hookCommand runs on every prompt: it hands the agent's hook input (with
// its session id) to the node, and prints the board news it gets back.
func hookCommand(addr string) string {
	return "curl -sf --max-time 3 --data-binary @- http://" + addr + hookPath + " || true"
}

// hookFiles are the agents' hook settings, each written only if its
// folder exists. Both read the same format.
func hookFiles(home string) []string {
	claude := os.Getenv("CLAUDE_CONFIG_DIR")
	if claude == "" {
		claude = filepath.Join(home, ".claude")
	}
	codex := os.Getenv("CODEX_HOME")
	if codex == "" {
		codex = filepath.Join(home, ".codex")
	}
	return []string{filepath.Join(claude, "settings.json"), filepath.Join(codex, "hooks.json")}
}

// installHooks adds the board's prompt hook to every agent's settings,
// replacing an older one, and leaves everything else in them as it was.
func installHooks(home, addr string) error {
	for _, file := range hookFiles(home) {
		if _, err := os.Stat(filepath.Dir(file)); err != nil {
			continue
		}
		old, err := os.ReadFile(file)
		if err != nil && !os.IsNotExist(err) {
			return err
		}
		settings := map[string]any{}
		if len(bytes.TrimSpace(old)) > 0 {
			if err := json.Unmarshal(old, &settings); err != nil {
				return err // the user's file; never overwrite what we can't read
			}
		}
		if !withHook(settings, hookCommand(addr)) {
			continue
		}
		b, _ := json.MarshalIndent(settings, "", "  ")
		if err := os.WriteFile(file, append(b, '\n'), 0o644); err != nil {
			return err
		}
	}
	return nil
}

// withHook puts command among settings' UserPromptSubmit hooks, in place
// of any board hook already there, and says whether anything changed.
func withHook(settings map[string]any, command string) bool {
	hooks, _ := settings["hooks"].(map[string]any)
	if hooks == nil {
		hooks = map[string]any{}
	}
	groups, _ := hooks["UserPromptSubmit"].([]any)
	var kept []any
	for _, g := range groups {
		group, _ := g.(map[string]any)
		list, _ := group["hooks"].([]any)
		var others []any
		for _, h := range list {
			h, _ := h.(map[string]any)
			if c, _ := h["command"].(string); c == command {
				return false
			} else if !bytes.Contains([]byte(c), []byte(hookPath)) {
				others = append(others, h)
			}
		}
		if len(others) > 0 {
			group["hooks"] = others
			kept = append(kept, group)
		}
	}
	kept = append(kept, map[string]any{"hooks": []any{map[string]any{"type": "command", "command": command}}})
	hooks["UserPromptSubmit"] = kept
	settings["hooks"] = hooks
	return true
}
