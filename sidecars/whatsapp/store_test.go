package main

import (
	"database/sql"
	"path/filepath"
	"testing"
	"time"
)

func testDB(t *testing.T) *sql.DB {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(schema); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

func must(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func texts(ms []stored) []string {
	var out []string
	for _, m := range ms {
		out = append(out, m.Text)
	}
	return out
}

func same(t *testing.T, got, want []string) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("got %q, want %q", got, want)
	}
	for i := range got {
		if got[i] != want[i] {
			t.Fatalf("got %q, want %q", got, want)
		}
	}
}

const kim, group = "61400000001@s.whatsapp.net", "123@g.us"

var at = time.Unix(1_800_000_000, 0)

func TestUnread(t *testing.T) {
	db := testDB(t)
	// History says how many are unread; its messages don't add to it.
	must(t, put(db, stored{Chat: group, ID: "h1", Sender: kim, Time: at, Text: "old"}, true))
	must(t, setChat(db, group, "Walkers", 2, at))
	must(t, put(db, stored{Chat: group, ID: "h1", Sender: kim, Time: at, Text: "old"}, false)) // seen already
	must(t, put(db, stored{Chat: kim, ID: "1", Sender: kim, Time: at.Add(time.Minute), Text: "hi"}, false))
	must(t, put(db, stored{Chat: kim, ID: "2", Sender: kim, Time: at.Add(2 * time.Minute), Text: "you there?"}, false))

	cs, err := chats(db)
	must(t, err)
	if len(cs) != 2 || cs[0].JID != kim || cs[0].Unread != 2 || cs[0].Last.Text != "you there?" || cs[1].Name != "Walkers" || cs[1].Unread != 2 {
		t.Fatalf("chats: %+v", cs)
	}
	// Replying means the user has read the chat.
	must(t, put(db, stored{Chat: kim, ID: "3", Sender: "me", FromMe: true, Time: at.Add(3 * time.Minute), Text: "yes"}, false))
	cs, err = chats(db)
	must(t, err)
	if cs[0].Unread != 0 || !cs[0].Last.FromMe {
		t.Fatalf("after replying: %+v", cs[0])
	}
}

func TestReadPagesNewestLast(t *testing.T) {
	db := testDB(t)
	for i, s := range []string{"one", "two", "three", "four", "five"} {
		must(t, put(db, stored{Chat: kim, ID: s, Sender: kim, Time: at.Add(time.Duration(i) * time.Minute), Text: s}, true))
	}
	must(t, put(db, stored{Chat: group, ID: "g", Sender: kim, Time: at, Text: "elsewhere"}, true))

	ms, err := messages(db, kim, "", 2, 0)
	must(t, err)
	same(t, texts(ms), []string{"four", "five"})
	ms, err = messages(db, kim, "", 2, 2)
	must(t, err)
	same(t, texts(ms), []string{"two", "three"})
	ms, err = messages(db, kim, "", 30, 4)
	must(t, err)
	same(t, texts(ms), []string{"one"})
}

func TestSearch(t *testing.T) {
	db := testDB(t)
	must(t, put(db, stored{Chat: kim, ID: "1", Sender: kim, Time: at, Text: "Dinner at 7?"}, true))
	must(t, put(db, stored{Chat: group, ID: "2", Sender: kim, Time: at.Add(time.Minute), Text: "dinner is 100% on"}, true))
	must(t, put(db, stored{Chat: group, ID: "3", Sender: kim, Time: at.Add(2 * time.Minute), Text: "lunch"}, true))

	ms, err := messages(db, "", "DINNER", 30, 0)
	must(t, err)
	same(t, texts(ms), []string{"Dinner at 7?", "dinner is 100% on"})
	ms, err = messages(db, group, "dinner", 30, 0)
	must(t, err)
	same(t, texts(ms), []string{"dinner is 100% on"})
	// % and _ are text, not patterns.
	ms, err = messages(db, "", "0%", 30, 0)
	must(t, err)
	same(t, texts(ms), []string{"dinner is 100% on"})
	ms, err = messages(db, "", "_", 30, 0)
	must(t, err)
	same(t, texts(ms), nil)
}

func TestEditAndRename(t *testing.T) {
	db := testDB(t)
	must(t, put(db, stored{Chat: group, ID: "1", Sender: kim, Time: at, Text: "typo"}, false))
	must(t, edit(db, group, "1", "fixed"))
	must(t, setName(db, group, "Walkers"))
	must(t, setChat(db, group, "", 0, at)) // history without a name keeps the name
	cs, err := chats(db)
	must(t, err)
	if cs[0].Last.Text != "fixed" || cs[0].Name != "Walkers" || groupName(db, group) != "Walkers" {
		t.Fatalf("%+v", cs[0])
	}
}
