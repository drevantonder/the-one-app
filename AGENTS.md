# TheOneApp

- Small core, everything else a plugin. Built-in plugins (`plugins/`) use exactly the public `sdk` a user plugin would; never give them private hooks into the core.
- Plugins never write shared state. They append to their own log and register transforms; the core rebuilds shared things (operations, `wake`, MCP instructions) from empty, in plugin order.
- Plugin state is rebuilt by replaying the log on every start. Handlers must give the same result for the same entries in the same order.
- Changes to `protocol/` are breaking for every plugin: bump `protocol.Version`.
- Run `go test ./...` before finishing. `cmd/one/e2e_test.go` builds the real binary and drives it over MCP.
- No filler docs or comments. Document only what the code can't make clear.
