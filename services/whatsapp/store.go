package main

import (
	"database/sql"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
)

// The messages the link sees, kept in the same database as whatsmeow's own
// tables, so the link and its history are one file.
const schema = `
CREATE TABLE IF NOT EXISTS chats (
	jid    TEXT PRIMARY KEY,
	name   TEXT NOT NULL DEFAULT '', -- a group's subject; people are named from contacts
	unread INTEGER NOT NULL DEFAULT 0,
	last   INTEGER NOT NULL DEFAULT 0 -- unix seconds
);
CREATE TABLE IF NOT EXISTS messages (
	chat        TEXT NOT NULL,
	id          TEXT NOT NULL,
	sender      TEXT NOT NULL,
	sender_name TEXT NOT NULL DEFAULT '', -- the name the sender set, for people not in contacts
	from_me     INTEGER NOT NULL,
	time        INTEGER NOT NULL,
	text        TEXT NOT NULL,
	PRIMARY KEY (chat, id)
);
CREATE INDEX IF NOT EXISTS messages_by_time ON messages (chat, time);
`

type stored struct {
	Chat, ID, Sender, SenderName string
	FromMe                       bool
	Time                         time.Time
	Text                         string
}

// put stores a message and counts it unread, unless it is history (which
// says how many are unread itself) or the user's own.
func put(db *sql.DB, m stored, history bool) error {
	tx, err := db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	res, err := tx.Exec(`INSERT OR IGNORE INTO messages (chat, id, sender, sender_name, from_me, time, text) VALUES (?, ?, ?, ?, ?, ?, ?)`,
		m.Chat, m.ID, m.Sender, m.SenderName, m.FromMe, m.Time.Unix(), m.Text)
	if err != nil {
		return err
	}
	if n, _ := res.RowsAffected(); n == 0 {
		return tx.Commit() // seen already
	}
	unread, first := "unread", 0
	switch {
	case m.FromMe:
		unread = "0"
	case !history:
		unread, first = "unread + 1", 1
	}
	if _, err := tx.Exec(`INSERT INTO chats (jid, last, unread) VALUES (?, ?, ?)
		ON CONFLICT (jid) DO UPDATE SET last = max(last, excluded.last), unread = `+unread,
		m.Chat, m.Time.Unix(), first); err != nil {
		return err
	}
	return tx.Commit()
}

func edit(db *sql.DB, chat, id, text string) error {
	_, err := db.Exec(`UPDATE messages SET text = ? WHERE chat = ? AND id = ?`, text, chat, id)
	return err
}

func setChat(db *sql.DB, jid, name string, unread int, last time.Time) error {
	_, err := db.Exec(`INSERT INTO chats (jid, name, unread, last) VALUES (?, ?, ?, ?)
		ON CONFLICT (jid) DO UPDATE SET name = CASE WHEN excluded.name = '' THEN name ELSE excluded.name END,
		unread = excluded.unread, last = max(last, excluded.last)`, jid, name, unread, last.Unix())
	return err
}

func setUnread(db *sql.DB, jid string, unread int) error {
	_, err := db.Exec(`UPDATE chats SET unread = ? WHERE jid = ?`, unread, jid)
	return err
}

func setName(db *sql.DB, jid, name string) error {
	_, err := db.Exec(`INSERT INTO chats (jid, name) VALUES (?, ?) ON CONFLICT (jid) DO UPDATE SET name = excluded.name`, jid, name)
	return err
}

func groupName(db *sql.DB, jid string) string {
	var name string
	_ = db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, jid).Scan(&name)
	return name
}

type chatRow struct {
	JID, Name string
	Unread    int
	Last      stored
}

// chats lists the chats that have messages, most recent first, each with its
// last message.
func chats(db *sql.DB) ([]chatRow, error) {
	rows, err := db.Query(`SELECT c.jid, c.name, c.unread, m.id, m.sender, m.sender_name, m.from_me, m.time, m.text
		FROM chats c JOIN messages m ON m.chat = c.jid AND m.rowid = (
			SELECT rowid FROM messages WHERE chat = c.jid ORDER BY time DESC, rowid DESC LIMIT 1)
		ORDER BY m.time DESC, c.last DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []chatRow
	for rows.Next() {
		var c chatRow
		var at int64
		if err := rows.Scan(&c.JID, &c.Name, &c.Unread, &c.Last.ID, &c.Last.Sender, &c.Last.SenderName, &c.Last.FromMe, &at, &c.Last.Text); err != nil {
			return nil, err
		}
		c.Last.Chat, c.Last.Time = c.JID, time.Unix(at, 0)
		out = append(out, c)
	}
	return out, rows.Err()
}

// messages are the limit newest messages after skipping the skip newest,
// newest last: in one chat (or all, if chat is empty), and only those
// containing query (case-insensitive) if it isn't empty.
func messages(db *sql.DB, chat, query string, limit, skip int) ([]stored, error) {
	q := `SELECT chat, id, sender, sender_name, from_me, time, text FROM messages WHERE (? = '' OR chat = ?)
		AND (? = '' OR text LIKE '%' || ? || '%' ESCAPE '\') ORDER BY time DESC, rowid DESC LIMIT ? OFFSET ?`
	like := likeEscape.Replace(query)
	rows, err := db.Query(q, chat, chat, like, like, limit, skip)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []stored
	for rows.Next() {
		var m stored
		var at int64
		if err := rows.Scan(&m.Chat, &m.ID, &m.Sender, &m.SenderName, &m.FromMe, &at, &m.Text); err != nil {
			return nil, err
		}
		m.Time = time.Unix(at, 0)
		out = append(out, m)
	}
	for i, j := 0, len(out)-1; i < j; i, j = i+1, j-1 {
		out[i], out[j] = out[j], out[i]
	}
	return out, rows.Err()
}

var likeEscape = strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`)

// text is how a message reads; empty for ones that aren't for people, such
// as reactions and key exchanges.
func text(m *waE2E.Message) string {
	if m == nil {
		return ""
	}
	with := func(kind, caption string) string {
		if caption = strings.TrimSpace(caption); caption != "" {
			return "(" + kind + ") " + caption
		}
		return "(" + kind + ")"
	}
	switch {
	case m.GetConversation() != "":
		return m.GetConversation()
	case m.GetExtendedTextMessage() != nil:
		return m.GetExtendedTextMessage().GetText()
	case m.GetImageMessage() != nil:
		return with("photo", m.GetImageMessage().GetCaption())
	case m.GetVideoMessage() != nil:
		return with("video", m.GetVideoMessage().GetCaption())
	case m.GetAudioMessage() != nil:
		if m.GetAudioMessage().GetPTT() {
			return "(voice message)"
		}
		return "(audio)"
	case m.GetDocumentMessage() != nil:
		return with("document", m.GetDocumentMessage().GetFileName()+" "+m.GetDocumentMessage().GetCaption())
	case m.GetStickerMessage() != nil:
		return "(sticker)"
	case m.GetLocationMessage() != nil:
		return with("location", m.GetLocationMessage().GetName())
	case m.GetLiveLocationMessage() != nil:
		return "(live location)"
	case m.GetContactMessage() != nil:
		return with("contact", m.GetContactMessage().GetDisplayName())
	case m.GetPollCreationMessage() != nil:
		return with("poll", m.GetPollCreationMessage().GetName())
	case m.GetPollCreationMessageV3() != nil:
		return with("poll", m.GetPollCreationMessageV3().GetName())
	}
	return ""
}
