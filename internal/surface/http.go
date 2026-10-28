// Package surface exposes a node's operations three ways from one list: an
// HTTP API (with OpenAPI), MCP tools, and (client-side) the CLI.
package surface

import (
	"encoding/json"
	"io"
	"net/http"

	"github.com/drevantonder/the-one-app/internal/core"
	"github.com/drevantonder/the-one-app/protocol"
)

// Handler serves the API and MCP for a node.
func Handler(n *core.Node, version string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok", "device": n.Device(), "version": version})
	})
	mux.HandleFunc("GET /v1/operations", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, n.Operations())
	})
	mux.HandleFunc("GET /openapi.json", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, OpenAPI(n.Operations(), version))
	})
	mux.HandleFunc("POST /v1/call/{plugin}/{op}", func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(io.LimitReader(r.Body, 4<<20))
		if err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]string{"error": err.Error()})
			return
		}
		if len(body) == 0 {
			body = []byte("{}")
		}
		if !json.Valid(body) {
			writeJSON(w, http.StatusBadRequest, map[string]string{"error": "body must be JSON"})
			return
		}
		res, err := n.Call(r.Context(), r.PathValue("plugin"), r.PathValue("op"), body, callerOf(r))
		if err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]string{"error": errText(err)})
			return
		}
		writeJSON(w, http.StatusOK, res)
	})
	mux.Handle("/mcp", MCP(n, version))
	return mux
}

// callerOf identifies who is calling. For now every request reaching the
// node is the owner: the node only listens on loopback and the tailnet.
func callerOf(r *http.Request) protocol.Caller {
	if a := r.Header.Get("X-One-Actor"); a != "" {
		return protocol.Caller{Actor: a}
	}
	return protocol.Caller{Actor: "owner"}
}

func mcpCaller() protocol.Caller { return protocol.Caller{Actor: "owner"} }

func errText(err error) string { return err.Error() }

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	_ = enc.Encode(v)
}

// OpenAPI generates an OpenAPI 3.1 document for the current operations.
func OpenAPI(ops []core.Op, version string) map[string]any {
	paths := map[string]any{}
	result := map[string]any{
		"type": "object",
		"properties": map[string]any{
			"text": map[string]any{"type": "string"},
			"data": map[string]any{},
		},
	}
	for _, o := range ops {
		paths["/v1/call/"+o.Plugin+"/"+o.Name] = map[string]any{
			"post": map[string]any{
				"operationId": o.ToolName(),
				"summary":     o.Description,
				"tags":        []string{o.Plugin},
				"requestBody": map[string]any{
					"required": true,
					"content":  map[string]any{"application/json": map[string]any{"schema": o.InputSchema}},
				},
				"responses": map[string]any{
					"200": map[string]any{"description": "Result", "content": map[string]any{"application/json": map[string]any{"schema": result}}},
					"400": map[string]any{"description": "Error", "content": map[string]any{"application/json": map[string]any{"schema": map[string]any{
						"type": "object", "properties": map[string]any{"error": map[string]any{"type": "string"}},
					}}}},
				},
			},
		}
	}
	return map[string]any{
		"openapi": "3.1.0",
		"info":    map[string]any{"title": "TheOneApp", "version": version},
		"paths":   paths,
	}
}
