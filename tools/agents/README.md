# Agent experience

Whether a fresh agent, prompted the way a person types, finds its way with the MCP server only, the CLI only, or both:

```sh
python3 tests/agents/run.py                      # every prompt, all three setups, local Gemma 4 12B: about 40 minutes
python3 tests/agents/run.py --prompts token,mac --setups mcp --repeat 3
python3 tests/agents/run.py --model opencode/deepseek-v4.1-flash-free --prompts board-latest,disk
```

It needs OpenCode (`opencode upgrade`), `pasta` (passt) and Ollama with the model pulled; it makes a 64k-context variant of an Ollama model itself. OpenCode's free hosted models share a daily limit that runs out after about a dozen runs (the run stops every setup at the same prompt when it does); never point `--model` at one that bills.

Each setup gets a throwaway network (its own HOME and port), whose node writes the harness's AGENTS.md, skills, board plugin and MCP entry as on any device. It's seeded with the real network's AGENTS.md and skills (read only), made-up board posts, two repos and a Family group, and no linked accounts. All of it runs in a network namespace that reaches the internet and Ollama but not the tailnet, the LAN or the real node, with the real home read-only, and is deleted at the end.

Transcripts land in `~/one-export/agent-experience/runs/<setup>/<prompt>.md` (and `.jsonl`), metrics and automatic checks in `summary.json`. Grading an answer still takes reading it: `expect` in `prompts.json` says what a good one does, `min_calls` the fewest calls it needs, `forbidden` what it must never call. Not run by `cargo test`.
