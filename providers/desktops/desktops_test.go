package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestPick(t *testing.T) {
	one := []desktop{{Name: "main"}}
	two := []desktop{{Name: "main"}, {Name: "work"}}
	if d, err := pick(one, ""); err != nil || d.Name != "main" {
		t.Errorf("the only desktop is the default: %v %v", d, err)
	}
	if d, err := pick(two, "work"); err != nil || d.Name != "work" {
		t.Errorf("by name: %v %v", d, err)
	}
	if _, err := pick(two, ""); err == nil || !strings.Contains(err.Error(), "main, work") {
		t.Errorf("with several, ask which: %v", err)
	}
	if _, err := pick(nil, ""); err == nil {
		t.Error("no desktops must be an error")
	}
	if _, err := pick(one, "work"); err == nil {
		t.Error("a missing desktop must be an error")
	}
}

func TestNextPort(t *testing.T) {
	if p := nextPort(nil); p != firstPort {
		t.Errorf("first: %d", p)
	}
	if p := nextPort([]desktop{{port: firstPort + 3}, {port: firstPort}}); p != firstPort+4 {
		t.Errorf("after the highest: %d", p)
	}
}

func TestEvent(t *testing.T) {
	stream := "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n" +
		"id: 2\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":7,\"result\":{}}\r\n\r\n"
	msg, err := event([]byte(stream))
	if err != nil {
		t.Fatal(err)
	}
	var m struct{ ID int }
	if err := json.Unmarshal(msg, &m); err != nil || m.ID != 7 {
		t.Errorf("the reply, not the notification: %s (%v)", msg, err)
	}
	if _, err := event([]byte("data: {\"method\":\"x\"}\n\n")); err == nil {
		t.Error("a stream with no reply must be an error")
	}
}

func TestWithDesktop(t *testing.T) {
	s := withDesktop(json.RawMessage(`{"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}`))
	props := s["properties"].(map[string]any)
	if props["url"] == nil || props["desktop"] == nil || s["required"] == nil {
		t.Errorf("keeps the tool's input and adds desktop: %v", s)
	}
	if s := withDesktop(nil); s["properties"].(map[string]any)["desktop"] == nil {
		t.Errorf("a tool with no input still takes desktop: %v", s)
	}
}

func TestContent(t *testing.T) {
	r := result{Content: []map[string]any{{"type": "text", "text": "launched"}}, Structured: json.RawMessage(`{"prepared_pid":5}`)}
	out, err := content(r)
	items := out.(map[string]any)["content"].([]map[string]any)
	if err != nil || len(items) != 2 || items[1]["text"] != `{"prepared_pid":5}` {
		t.Errorf("structured content is kept as text: %v %v", out, err)
	}
	r.IsError = true
	if _, err := content(r); err == nil || !strings.Contains(err.Error(), "prepared_pid") {
		t.Errorf("an error carries everything the tool said: %v", err)
	}
}

func TestGist(t *testing.T) {
	if g := gist("Click a thing. Prefer tokens.\nMore."); g != "Click a thing." {
		t.Errorf("%q", g)
	}
	if g := gist("No full stop"); g != "No full stop" {
		t.Errorf("%q", g)
	}
}
