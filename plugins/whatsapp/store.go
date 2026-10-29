package whatsapp

import (
	"database/sql"
	"fmt"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
)

// The messages the link sees, kept beside the link's own tables in the same
// database, so moving the link moves them too.
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

// put stores a message, and counts it unread unless it is history (which
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

type chatRow struct {
	JID, Name string
	Unread    int
	Last      time.Time
	Message   *stored // the last one
}

// chats lists chats with messages, most recent first.
func chats(db *sql.DB) ([]chatRow, error) {
	rows, err := db.Query(`SELECT c.jid, c.name, c.unread, c.last, m.id, m.sender, m.sender_name, m.from_me, m.time, m.text
		FROM chats c LEFT JOIN messages m ON m.chat = c.jid AND m.id = (
			SELECT id FROM messages WHERE chat = c.jid ORDER BY time DESC, rowid DESC LIMIT 1)
		WHERE c.last > 0 ORDER BY c.last DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []chatRow
	for rows.Next() {
		var c chatRow
		var last int64
		var id, sender, senderName, text sql.NullString
		var fromMe sql.NullBool
		var at sql.NullInt64
		if err := rows.Scan(&c.JID, &c.Name, &c.Unread, &last, &id, &sender, &senderName, &fromMe, &at, &text); err != nil {
			return nil, err
		}
		c.Last = time.Unix(last, 0)
		if id.Valid {
			c.Message = &stored{Chat: c.JID, ID: id.String, Sender: sender.String, SenderName: senderName.String, FromMe: fromMe.Bool, Time: time.Unix(at.Int64, 0), Text: text.String}
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

// messages are a chat's last limit messages, oldest first, or those
// matching query across chats (chat empty), newest first.
func messages(db *sql.DB, chat, query string, limit int) ([]stored, error) {
	where, args := []string{}, []any{}
	if chat != "" {
		where, args = append(where, "chat = ?"), append(args, chat)
	}
	if query != "" {
		where, args = append(where, `text LIKE ? ESCAPE '\'`), append(args, "%"+likeEscape.Replace(query)+"%")
	}
	q := `SELECT chat, id, sender, sender_name, from_me, time, text FROM messages`
	if len(where) > 0 {
		q += " WHERE " + strings.Join(where, " AND ")
	}
	q += fmt.Sprintf(" ORDER BY time DESC, rowid DESC LIMIT %d", limit)
	rows, err := db.Query(q, args...)
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
	if query == "" { // a conversation reads oldest first
		for i, j := 0, len(out)-1; i < j; i, j = i+1, j-1 {
			out[i], out[j] = out[j], out[i]
		}
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
