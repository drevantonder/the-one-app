package t3code

import (
	"strings"
	"testing"
	"time"
)

const list = `93cb4dac-5cb7-4695-b848-3898b45e8a2f
  scopes: orchestration:read orchestration:operate
  method: bearer-access-token
  subject: one-time-token
  client: iphone | mobile | iOS | 127.0.0.1
  issued: 2026-10-28T07:34:09.400Z
  last connected: 2026-10-29T04:07:34.921Z
  expires: 2026-11-27T07:34:09.400Z

6c2935c8-8da1-44ff-8d84-a7927cc6d12e
  scopes: orchestration:read
  method: bearer-access-token
  subject: mcp-client
  client: t3 thread cli | bot
  expires: 2026-11-27T07:34:09.400Z
`

func TestSessions(t *testing.T) {
	ss := sessions(list)
	if len(ss) != 2 || ss[1].Subject != "mcp-client" {
		t.Fatalf("got %+v", ss)
	}
	as := apps(ss)
	if len(as) != 1 || as[0].Client != "iphone | mobile | iOS | 127.0.0.1" || as[0].Expires.Day() != 27 || as[0].LastConnected.IsZero() {
		t.Fatalf("got %+v", as)
	}
	now := time.Date(2026, 11, 24, 0, 0, 0, 0, time.UTC)
	r := report([]host{{Name: "ptc", Online: true, Host: true, Version: "0.1.3", Apps: as}, {Name: "linux"}}, now)
	for _, want := range []string{"ptc (host): T3 Code 0.1.3", "expires in 4 days: pair again soon", "linux: offline"} {
		if !strings.Contains(r, want) {
			t.Errorf("report lacks %q:\n%s", want, r)
		}
	}
}
