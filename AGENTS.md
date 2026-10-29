# The One App

- Small core, everything else a plugin. Built-in plugins (`plugins/`) use exactly the public `sdk` a user plugin would; never give them private hooks into the core.
- Plugins never write shared state. They append to their own log and register transforms; the core rebuilds shared things (operations, MCP instructions) from empty, in plugin order. The core never names or knows about a specific plugin.
- Plugin state is rebuilt by replaying the log on every start. Handlers must give the same result for the same entries in the same order.
- Changes to `protocol/` are breaking for every plugin: bump `protocol.Version`.
- Run `go test ./...` before finishing. `cmd/one/e2e_test.go` builds the real binary and drives it over MCP.
- Commit and push to main as you go: one atomic commit per change, made once it works and its tests pass, not one at the end. Conventional Commits, with no co-author or "Generated with" lines.
- GitHub main is the only shared state: several threads, on several machines, work on this at once. Pull (`git pull --rebase`) before starting and before each push. Clone the repo; never copy a working tree to another machine.
- After each push to main, run `one update` so every device runs the latest. Deploy only what is on main, and only with `one update`: it builds the tip of main and every node installs it and restarts in place. Never install or restart a node by hand.
- No filler docs or comments. Document only what the code can't make clear.
