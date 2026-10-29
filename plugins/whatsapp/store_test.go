package whatsapp

import (
	"database/sql"
	"path/filepath"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
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

func TestUnread(t *testing.T) {
	db := testDB(t)
	at := time.Unix(1_800_000_000, 0)
	kim, group := "61400000001@s.whatsapp.net", "123@g.us"
	must := func(err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
	// History says how many are unread; its messages don't add to it.
	must(put(db, stored{Chat: group, ID: "h1", Sender: kim, Time: at, Text: "old"}, true))
	must(setChat(db, group, "Walkers", 2, at))
	must(put(db, stored{Chat: group, ID: "h1", Sender: kim, Time: at, Text: "old"}, false)) // seen already
	must(put(db, stored{Chat: kim, ID: "1", Sender: kim, Time: at.Add(time.Minute), Text: "hi"}, false))
	must(put(db, stored{Chat: kim, ID: "2", Sender: kim, Time: at.Add(2 * time.Minute), Text: "lunch?"}, false))
	must(put(db, stored{Chat: group, ID: "3", Sender: kim, Time: at.Add(3 * time.Minute), Text: "8am"}, false))

	cs, err := chats(db)
	must(err)
	if len(cs) != 2 || cs[0].JID != group || cs[0].Unread != 3 || cs[0].Name != "Walkers" || cs[1].Unread != 2 || cs[1].Message.Text != "lunch?" {
		t.Fatalf("chats: %+v", cs)
	}
	// Replying, here or on the phone, reads the chat.
	must(put(db, stored{Chat: kim, ID: "4", Sender: "me", FromMe: true, Time: at.Add(4 * time.Minute), Text: "yes"}, false))
	cs, _ = chats(db)
	if cs[0].JID != kim || cs[0].Unread != 0 {
		t.Fatalf("after replying: %+v", cs[0])
	}
}

func TestMessages(t *testing.T) {
	db := testDB(t)
	at := time.Unix(1_800_000_000, 0)
	for i, text := range []string{"one", "50% off", "three", "100%"} {
		if err := put(db, stored{Chat: "c", ID: text, Sender: "s", Time: at.Add(time.Duration(i) * time.Minute), Text: text}, false); err != nil {
			t.Fatal(err)
		}
	}
	ms, _ := messages(db, "c", "", 2)
	if len(ms) != 2 || ms[0].Text != "three" || ms[1].Text != "100%" {
		t.Errorf("the last two, oldest first: %+v", ms)
	}
	ms, _ = messages(db, "", "0%", 10)
	if len(ms) != 2 || ms[0].Text != "100%" {
		t.Errorf("search, newest first, %% literal: %+v", ms)
	}
	if err := edit(db, "c", "one", "(deleted)"); err != nil {
		t.Fatal(err)
	}
	if ms, _ = messages(db, "", "deleted", 10); len(ms) != 1 {
		t.Errorf("edit: %+v", ms)
	}
}

func TestText(t *testing.T) {
	for _, c := range []struct {
		m    *waE2E.Message
		want string
	}{
		{&waE2E.Message{Conversation: proto.String("hi")}, "hi"},
		{&waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("see https://x")}}, "see https://x"},
		{&waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("beach")}}, "(photo) beach"},
		{&waE2E.Message{AudioMessage: &waE2E.AudioMessage{PTT: proto.Bool(true)}}, "(voice message)"},
		{&waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{Text: proto.String("👍")}}, ""},
	} {
		if got := text(c.m); got != c.want {
			t.Errorf("text(%v) = %q, want %q", c.m, got, c.want)
		}
	}
}

func TestOpen(t *testing.T) {
	p := &plugin{dir: t.TempDir()}
	if err := p.open(); err != nil {
		t.Fatal(err)
	}
	if p.client.Store.ID != nil {
		t.Error("a new database has no link")
	}
	p.forget()
	if err := p.open(); err != nil {
		t.Fatal("reopening:", err)
	}
	p.close()
}
