package sdk

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestDiscoveryCallBuildsTheRequest(t *testing.T) {
	var got *http.Request
	var gotBody string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		got, gotBody = r, string(b)
		w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()
	im := &discoveryImport{base: srv.URL + "/"}
	m := discoveryMethod{
		ID: "gmail.users.messages.modify", Path: "gmail/v1/users/{userId}/messages/{id}/modify", HTTPMethod: "POST",
		Parameters: map[string]discoveryParam{
			"userId": {Location: "path", Required: true, Default: "me"},
			"id":     {Location: "path", Required: true},
			"fields": {Location: "query", Repeated: true},
		},
	}
	res, err := im.call(context.Background(), m, json.RawMessage(`{"id":"a/b","fields":["x","y"],"body":{"addLabelIds":["L1"]}}`))
	if err != nil {
		t.Fatal(err)
	}
	if got.URL.EscapedPath() != "/gmail/v1/users/me/messages/a%2Fb/modify" {
		t.Errorf("path = %s", got.URL.EscapedPath())
	}
	if q := got.URL.Query()["fields"]; len(q) != 2 {
		t.Errorf("fields = %v", q)
	}
	if gotBody != `{"addLabelIds":["L1"]}` {
		t.Errorf("body = %s", gotBody)
	}
	if res.Data.(map[string]any)["ok"] != true {
		t.Errorf("data = %v", res.Data)
	}
	if _, err := im.call(context.Background(), m, json.RawMessage(`{}`)); err == nil {
		t.Error("a missing required path parameter must fail")
	}
}

func TestDiscoverySchemaInlinesBodiesAndCutsRecursion(t *testing.T) {
	schemas := map[string]json.RawMessage{
		"Part": json.RawMessage(`{"type":"object","properties":{"parts":{"type":"array","items":{"$ref":"Part"}}}}`),
	}
	s := inputSchema(discoveryMethod{HTTPMethod: "POST", Request: &struct {
		Ref string `json:"$ref"`
	}{Ref: "Part"}}, schemas)
	raw, _ := json.Marshal(s)
	if len(raw) > 4000 {
		t.Fatalf("recursive schema not cut off: %d bytes", len(raw))
	}
	if s["required"].([]string)[0] != "body" {
		t.Errorf("body should be required: %v", s["required"])
	}
}
