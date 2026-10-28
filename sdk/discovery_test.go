package sdk

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
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
	im := &discoveryImport{}
	base := srv.URL + "/"
	m := discoveryMethod{
		ID: "gmail.users.messages.modify", Path: "gmail/v1/users/{userId}/messages/{id}/modify", HTTPMethod: "POST",
		Parameters: map[string]discoveryParam{
			"userId": {Location: "path", Required: true, Default: "me"},
			"id":     {Location: "path", Required: true},
			"fields": {Location: "query", Repeated: true},
		},
	}
	res, err := im.call(context.Background(), base, m, json.RawMessage(`{"id":"a/b","fields":["x","y"],"body":{"addLabelIds":["L1"]}}`))
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
	if _, err := im.call(context.Background(), base, m, json.RawMessage(`{}`)); err == nil {
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

func TestDiscoveryUsesTheChosenAccount(t *testing.T) {
	var auth string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		auth = r.Header.Get("Authorization")
		w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	later := time.Now().Add(time.Hour)
	accounts := &Accounts{grants: map[string]*grant{
		"me@home.example": {access: "home-token", expires: later},
		"me@work.example": {access: "work-token", expires: later},
	}}
	im := &discoveryImport{accounts: accounts}
	m := discoveryMethod{Path: "profile", HTTPMethod: "GET"}

	schema := accounts.withAccount(map[string]any{"type": "object"})
	if _, ok := schema["properties"].(map[string]any)["account"]; !ok {
		t.Fatalf("with two accounts, operations take an account: %v", schema)
	}
	if _, err := im.call(context.Background(), srv.URL+"/", m, json.RawMessage(`{"account":"me@work.example"}`)); err != nil {
		t.Fatal(err)
	}
	if auth != "Bearer work-token" {
		t.Errorf("Authorization = %q", auth)
	}
	_, err := im.call(context.Background(), srv.URL+"/", m, json.RawMessage(`{}`))
	if err == nil || !strings.Contains(err.Error(), "me@home.example, me@work.example") {
		t.Errorf("without an account, want the choices; got %v", err)
	}

	delete(accounts.grants, "me@home.example")
	if _, ok := accounts.withAccount(map[string]any{"type": "object"})["properties"]; ok {
		t.Error("with one account, operations don't take an account")
	}
	if _, err := im.call(context.Background(), srv.URL+"/", m, json.RawMessage(`{}`)); err != nil || auth != "Bearer work-token" {
		t.Errorf("the only account is used: %v %q", err, auth)
	}
}

func TestIDTokenEmail(t *testing.T) {
	payload := base64.RawURLEncoding.EncodeToString([]byte(`{"email":"me@home.example"}`))
	if got := idTokenEmail("h." + payload + ".s"); got != "me@home.example" {
		t.Errorf("got %q", got)
	}
}
