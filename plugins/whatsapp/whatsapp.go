// Package whatsapp is the built-in plugin for WhatsApp. It joins the user's
// account as a linked device, as WhatsApp Desktop does, over WhatsApp's own
// protocol (go.mau.fi/whatsmeow), and keeps the messages it sees in a
// database of its own, beside the link's keys.
//
// One device holds the link at a time. Only that device offers the
// operations that use it, so the core routes calls there; release moves the
// link, through the bucket, to whichever device signs in next.
//
// The link never tells WhatsApp the user is online or has read a message,
// so the phone keeps notifying and senders don't see reads.
package whatsapp

import (
	"cmp"
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"

	"github.com/drevantonder/the-one-app/sdk"
)

var Plugin = sdk.Plugin{ID: "whatsapp", Name: "WhatsApp", Version: "0.1.0", Setup: setup}

const released = "link" // the released link's file in the bucket

type signInIn struct {
	Phone string `json:"phone,omitempty" jsonschema:"the account's phone number with its country code, e.g. +61 400 000 000; not needed to take a released link"`
	Force bool   `json:"force,omitempty" jsonschema:"link this device even though another holds the link, as when that device is gone"`
}

type chatsIn struct {
	Unread bool `json:"unread,omitempty" jsonschema:"only unread chats"`
	Limit  int  `json:"limit,omitempty" jsonschema:"at most this many, most recent first; default 30"`
}

type readIn struct {
	Chat  string `json:"chat" jsonschema:"the chat's name, a part of it only one chat has, or a phone number"`
	Limit int    `json:"limit,omitempty" jsonschema:"the most recent this many; default 30"`
}

type searchIn struct {
	Query string `json:"query" jsonschema:"text to look for, case-insensitive"`
	Chat  string `json:"chat,omitempty" jsonschema:"only in this chat"`
	Limit int    `json:"limit,omitempty" jsonschema:"at most this many, newest first; default 30"`
}

type sendIn struct {
	Chat string `json:"chat" jsonschema:"the chat's name, a part of it only one chat has, a contact's exact name, or a phone number with its country code"`
	Text string `json:"text"`
}

type linked struct {
	Host  string `json:"host"`
	Phone string `json:"phone,omitempty"`
}

// Chat is a chat as the operations show it.
type Chat struct {
	Name   string    `json:"name"`
	JID    string    `json:"jid"`
	Unread int       `json:"unread,omitempty"`
	Last   string    `json:"last,omitempty"`
	Time   time.Time `json:"time"`
}

// Message is a message as the operations show it.
type Message struct {
	Chat   string    `json:"chat,omitempty"`
	Sender string    `json:"sender"` // "You" for the user
	Time   time.Time `json:"time"`
	Text   string    `json:"text"`
}

type plugin struct {
	c    *sdk.Ctx
	dir  string
	call *sdk.Call // for appending outside an operation

	mu     sync.Mutex
	holder linked // from the log
	db     *sql.DB
	client *whatsmeow.Client
}

func setup(c *sdk.Ctx) error {
	dir, err := c.Dir()
	if err != nil {
		return err
	}
	p := &plugin{c: c, dir: dir}
	c.Entry("linked", sdk.Available)
	c.Entry("unlinked", sdk.Available)
	c.OnReset(func() { p.mu.Lock(); p.holder = linked{}; p.mu.Unlock() })
	sdk.Handle(c, "linked", func(_ sdk.Entry, in linked) error { p.mu.Lock(); p.holder = in; p.mu.Unlock(); return nil })
	sdk.Handle(c, "unlinked", func(_ sdk.Entry, _ struct{}) error { p.mu.Lock(); p.holder = linked{}; p.mu.Unlock(); return nil })
	store.SetOSInfo("The One App", [3]uint32{0, 1, 0})

	sdk.Op(c, "sign_in", "Link WhatsApp on this device, as WhatsApp Desktop is linked: with a code to enter on the phone, or by taking the link another device released.", sdk.Write, p.signIn)
	c.OnReady(func(call *sdk.Call) {
		p.call = call.Detached()
		p.mu.Lock()
		defer p.mu.Unlock()
		if _, err := os.Stat(p.path()); err != nil {
			return
		}
		if err := p.open(); err != nil {
			c.Logf("whatsapp: %v", err)
			return
		}
		if p.client.Store.ID == nil {
			p.close()
			return
		}
		if err := p.client.Connect(); err != nil {
			c.Logf("whatsapp: connecting: %v", err)
		}
		p.serve(true)
	})
	return nil
}

func (p *plugin) path() string { return filepath.Join(p.dir, "whatsapp.db") }

// open opens the database and a client for the link in it, if any.
func (p *plugin) open() error {
	db, err := sql.Open("sqlite", "file:"+p.path()+"?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)")
	if err != nil {
		return err
	}
	container := sqlstore.NewWithDB(db, "sqlite", logger{"whatsmeow"})
	ctx := context.Background()
	if err := container.Upgrade(ctx); err != nil {
		db.Close()
		return err
	}
	if _, err := db.Exec(schema); err != nil {
		db.Close()
		return err
	}
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		db.Close()
		return err
	}
	p.db, p.client = db, whatsmeow.NewClient(device, logger{"whatsmeow"})
	p.client.AddEventHandler(p.handle)
	return nil
}

// close disconnects and closes the database, with everything in its main
// file.
func (p *plugin) close() {
	if p.client != nil {
		p.client.Disconnect()
	}
	if p.db != nil {
		_, _ = p.db.Exec("PRAGMA wal_checkpoint(TRUNCATE)")
		p.db.Close()
	}
	p.db, p.client = nil, nil
}

// forget closes and deletes the link and its messages on this device.
func (p *plugin) forget() {
	p.close()
	for _, suffix := range []string{"", "-wal", "-shm"} {
		_ = os.Remove(p.path() + suffix)
	}
}

func host() string {
	h, _ := os.Hostname()
	h, _, _ = strings.Cut(h, ".")
	return strings.ToLower(h)
}

func (p *plugin) signIn(call *sdk.Call, in signInIn) (sdk.Result, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.client != nil && p.client.Store.ID != nil {
		return sdk.Text(fmt.Sprintf("Already linked here, as +%s.", p.client.Store.ID.User)), nil
	}
	if p.holder.Host != "" && p.holder.Host != host() && !in.Force {
		return sdk.Result{}, fmt.Errorf("WhatsApp is linked on %s: release it there to move it here, or pass force if that device is gone", p.holder.Host)
	}
	if files, _ := call.Files(released); len(files) > 0 {
		p.forget()
		if err := call.GetFile(released, p.path()); err != nil {
			return sdk.Result{}, err
		}
		if err := p.open(); err != nil {
			return sdk.Result{}, err
		}
		if p.client.Store.ID == nil {
			p.forget()
			return sdk.Result{}, errors.New("the released link was empty: sign in with phone")
		}
		if err := p.client.Connect(); err != nil {
			return sdk.Result{}, err
		}
		if err := call.DeleteFile(released); err != nil {
			return sdk.Result{}, err
		}
		if _, err := call.Append("linked", linked{Host: host(), Phone: p.client.Store.ID.User}); err != nil {
			return sdk.Result{}, err
		}
		p.serve(true)
		return sdk.Text(fmt.Sprintf("Took the released link, as +%s.", p.client.Store.ID.User)), nil
	}
	if in.Phone == "" {
		return sdk.Result{}, errors.New("pass phone: the account's number, with its country code")
	}
	p.forget()
	if err := p.open(); err != nil {
		return sdk.Result{}, err
	}
	qr, err := p.client.GetQRChannel(context.Background())
	if err != nil {
		return sdk.Result{}, err
	}
	if err := p.client.Connect(); err != nil {
		return sdk.Result{}, err
	}
	select { // the first QR code means the connection is ready to pair
	case <-qr:
	case <-time.After(15 * time.Second):
		p.forget()
		return sdk.Result{}, errors.New("WhatsApp didn't answer")
	}
	code, err := p.client.PairPhone(call.Context, in.Phone, true, whatsmeow.PairClientChrome, "Chrome (Linux)")
	if err != nil {
		p.forget()
		return sdk.Result{}, err
	}
	go func() { // PairSuccess, handled in handle, finishes the link
		for item := range qr {
			if item.Event == whatsmeow.QRChannelTimeout.Event || item.Error != nil {
				p.mu.Lock()
				if p.client != nil && p.client.Store.ID == nil {
					p.forget()
				}
				p.mu.Unlock()
			}
		}
	}()
	return sdk.Text(fmt.Sprintf("On the phone, open WhatsApp > Settings > Linked devices > Link a device > Link with phone number instead, and enter %s within 2 minutes. WhatsApp then sends recent chats here, which can take a few minutes.", code)), nil
}

// serve offers the operations that use the link, or takes them away.
func (p *plugin) serve(on bool) {
	for _, name := range []string{"chats", "read", "search", "send", "release", "sign_out"} {
		p.c.RemoveOp(name)
	}
	if on {
		sdk.Op(p.c, "chats", "List recent WhatsApp chats: who, the last message, when, and how many are unread.", sdk.Read, p.chats)
		sdk.Op(p.c, "read", "Read a WhatsApp chat's recent messages, oldest first. WhatsApp isn't told they were read.", sdk.Read, p.read)
		sdk.Op(p.c, "search", "Search WhatsApp messages, across chats or in one.", sdk.Read, p.search)
		sdk.Op(p.c, "send", "Send a WhatsApp message, as the signed-in account, to a chat, a contact or a phone number. This sends a real message to a real person.", sdk.Write, p.send)
		sdk.Op(p.c, "release", "Free the WhatsApp link for another device: it is stored in the bucket, encrypted, and removed here; then sign_in on the other device takes it.", sdk.Write, p.release)
		sdk.Op(p.c, "sign_out", "Unlink WhatsApp: remove this device from the account's linked devices and delete its messages here.", sdk.Write, p.signOut)
	}
	p.c.Changed()
}

func (p *plugin) handle(evt any) {
	ctx := context.Background()
	switch e := evt.(type) {
	case *events.PairSuccess:
		go func() {
			if _, err := p.call.Append("linked", linked{Host: host(), Phone: e.ID.User}); err != nil {
				p.c.Logf("whatsapp: %v", err)
			}
			p.mu.Lock()
			p.serve(true)
			p.mu.Unlock()
		}()
	case *events.LoggedOut:
		p.c.Logf("whatsapp: unlinked by WhatsApp or the phone (reason %v)", e.Reason)
		go func() {
			p.mu.Lock()
			defer p.mu.Unlock()
			p.forget()
			p.serve(false)
			_, _ = p.call.Append("unlinked", struct{}{})
		}()
	case *events.StreamReplaced:
		p.c.Logf("whatsapp: another copy of this link connected; this one stopped")
	case *events.TemporaryBan:
		p.c.Logf("whatsapp: %v", e)
	case *events.Message:
		p.message(ctx, e, false)
	case *events.HistorySync:
		for _, conv := range e.Data.GetConversations() {
			jid, err := types.ParseJID(conv.GetID())
			if err != nil {
				continue
			}
			chat := p.key(ctx, jid)
			for _, m := range conv.GetMessages() {
				if msg, err := p.client.ParseWebMessage(jid, m.GetMessage()); err == nil {
					p.message(ctx, msg, true)
				}
			}
			name := ""
			if jid.Server == types.GroupServer {
				name = conv.GetName()
			}
			if err := setChat(p.db, chat, name, int(conv.GetUnreadCount()), time.Unix(int64(conv.GetConversationTimestamp()), 0)); err != nil {
				p.c.Logf("whatsapp: %v", err)
			}
		}
	case *events.MarkChatAsRead:
		unread := 0
		if !e.Action.GetRead() {
			unread = 1
		}
		_ = setUnread(p.db, p.key(ctx, e.JID), unread)
	case *events.GroupInfo:
		if e.Name != nil {
			_ = setName(p.db, e.JID.String(), e.Name.Name)
		}
	case *events.JoinedGroup:
		_ = setName(p.db, e.JID.String(), e.GroupName.Name)
	}
}

func (p *plugin) message(ctx context.Context, e *events.Message, history bool) {
	chat := p.key(ctx, e.Info.Chat)
	if pm := e.Message.GetProtocolMessage(); pm != nil {
		switch pm.GetType() {
		case waE2E.ProtocolMessage_REVOKE:
			_ = edit(p.db, chat, pm.GetKey().GetID(), "(deleted)")
		case waE2E.ProtocolMessage_MESSAGE_EDIT:
			if t := text(pm.GetEditedMessage()); t != "" {
				_ = edit(p.db, chat, pm.GetKey().GetID(), t)
			}
		}
		return
	}
	t := text(e.Message)
	if t == "" || e.Info.Chat.Server == types.BroadcastServer {
		return
	}
	if err := put(p.db, stored{Chat: chat, ID: e.Info.ID, Sender: p.key(ctx, e.Info.Sender), SenderName: e.Info.PushName,
		FromMe: e.Info.IsFromMe, Time: e.Info.Timestamp, Text: t}, history); err != nil {
		p.c.Logf("whatsapp: storing a message: %v", err)
	}
	if e.Info.IsGroup && !history {
		var name string
		_ = p.db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, chat).Scan(&name)
		if name == "" {
			go func() {
				if info, err := p.client.GetGroupInfo(ctx, e.Info.Chat); err == nil {
					_ = setName(p.db, chat, info.Name)
				}
			}()
		}
	}
}

// key is how a chat or person is stored: by phone number where WhatsApp
// gave a private id (a LID) whose number is known, so each person is one
// chat.
func (p *plugin) key(ctx context.Context, jid types.JID) string {
	jid = jid.ToNonAD()
	if jid.Server == types.HiddenUserServer {
		if pn, err := p.client.Store.LIDs.GetPNForLID(ctx, jid); err == nil && !pn.IsEmpty() {
			jid = pn.ToNonAD()
		}
	}
	return jid.String()
}

// name is a chat's or person's name: a group's subject, or the contact's
// name, else the name they set, else their number.
func (p *plugin) name(ctx context.Context, jid, group, pushName string) string {
	if group != "" {
		return group
	}
	j, err := types.ParseJID(jid)
	if err != nil {
		return jid
	}
	if j.Server == types.GroupServer {
		return "a group"
	}
	if c, err := p.client.Store.Contacts.GetContact(ctx, j); err == nil {
		for _, n := range []string{c.FullName, c.FirstName, c.BusinessName, c.PushName} {
			if n != "" {
				return n
			}
		}
	}
	if pushName != "" {
		return pushName
	}
	if j.Server == types.DefaultUserServer {
		return "+" + j.User
	}
	return j.User
}

func (p *plugin) linkedDB() (*sql.DB, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.db == nil || p.client == nil || p.client.Store.ID == nil {
		return nil, errors.New("WhatsApp isn't linked on this device")
	}
	return p.db, nil
}

func (p *plugin) allChats(ctx context.Context) ([]Chat, error) {
	db, err := p.linkedDB()
	if err != nil {
		return nil, err
	}
	rows, err := chats(db)
	if err != nil {
		return nil, err
	}
	out := make([]Chat, 0, len(rows))
	for _, r := range rows {
		c := Chat{Name: p.name(ctx, r.JID, r.Name, ""), JID: r.JID, Unread: r.Unread, Time: r.Last}
		if m := r.Message; m != nil {
			c.Last = m.Text
			switch {
			case m.FromMe:
				c.Last = "You: " + m.Text
			case strings.HasSuffix(r.JID, "@"+types.GroupServer):
				c.Last = p.name(ctx, m.Sender, "", m.SenderName) + ": " + m.Text
			}
		}
		out = append(out, c)
	}
	return out, nil
}

func (p *plugin) chats(call *sdk.Call, in chatsIn) (sdk.Result, error) {
	all, err := p.allChats(call.Context)
	if err != nil {
		return sdk.Result{}, err
	}
	limit := cmp.Or(in.Limit, 30)
	var shown []Chat
	var lines []string
	for _, c := range all {
		if in.Unread && c.Unread == 0 {
			continue
		}
		if len(shown) == limit {
			break
		}
		shown = append(shown, c)
		mark := ""
		if c.Unread > 0 {
			mark = fmt.Sprintf(" (%d unread)", c.Unread)
		}
		lines = append(lines, fmt.Sprintf("%s%s, %s: %s", c.Name, mark, when(c.Time), strings.Join(strings.Fields(c.Last), " ")))
	}
	if len(shown) == 0 {
		if in.Unread {
			return sdk.Result{Text: "No unread chats.", Data: []Chat{}}, nil
		}
		return sdk.Result{Text: "No chats yet: after linking, WhatsApp takes a few minutes to send them.", Data: []Chat{}}, nil
	}
	return sdk.Result{Text: strings.Join(lines, "\n"), Data: shown}, nil
}

var phone = regexp.MustCompile(`^\+?[\d\s().-]{7,}$`)

// find resolves a chat by name, a part of one, or a phone number. With
// contacts, a contact's exact name also counts, for people without a chat.
func (p *plugin) find(ctx context.Context, want string, contacts bool) (Chat, error) {
	want = strings.TrimSpace(want)
	if phone.MatchString(want) {
		digits := strings.Map(func(r rune) rune {
			if r >= '0' && r <= '9' {
				return r
			}
			return -1
		}, want)
		if !strings.HasPrefix(want, "+") || strings.HasPrefix(digits, "0") {
			return Chat{}, fmt.Errorf("give %q with its country code, e.g. +61…", want)
		}
		found, err := p.client.IsOnWhatsApp(ctx, []string{"+" + digits})
		if err != nil {
			return Chat{}, err
		}
		if len(found) == 0 || !found[0].IsIn {
			return Chat{}, fmt.Errorf("%s isn't on WhatsApp", want)
		}
		jid := found[0].JID.ToNonAD().String()
		return Chat{Name: p.name(ctx, jid, "", ""), JID: jid}, nil
	}
	all, err := p.allChats(ctx)
	if err != nil {
		return Chat{}, err
	}
	w := strings.ToLower(want)
	var partial []Chat
	for _, c := range all {
		n := strings.ToLower(c.Name)
		if n == w {
			return c, nil
		}
		if strings.Contains(n, w) {
			partial = append(partial, c)
		}
	}
	if len(partial) == 1 && !contacts {
		return partial[0], nil
	}
	if contacts {
		var exact []Chat
		if known, err := p.client.Store.Contacts.GetAllContacts(ctx); err == nil {
			for jid, c := range known {
				if jid.Server == types.DefaultUserServer && strings.EqualFold(c.FullName, want) {
					exact = append(exact, Chat{Name: c.FullName, JID: jid.String()})
				}
			}
		}
		switch {
		case len(exact) == 1:
			return exact[0], nil
		case len(exact) > 1:
			return Chat{}, fmt.Errorf("several contacts are called %q: give a phone number instead", want)
		case len(partial) == 1:
			return partial[0], nil
		}
	}
	names := func(cs []Chat) string {
		var out []string
		for _, c := range cs {
			out = append(out, c.Name)
		}
		return strings.Join(out, "; ")
	}
	if len(partial) > 1 {
		return Chat{}, fmt.Errorf("%q matches several chats: %s", want, names(partial))
	}
	return Chat{}, fmt.Errorf("no chat called %q", want)
}

func (p *plugin) shown(ctx context.Context, ms []stored, withChat bool) ([]Message, []string) {
	var out []Message
	var lines []string
	names := map[string]string{}
	for _, m := range ms {
		sender := "You"
		if !m.FromMe {
			sender = p.name(ctx, m.Sender, "", m.SenderName)
		}
		msg := Message{Sender: sender, Time: m.Time, Text: m.Text}
		line := fmt.Sprintf("[%s] %s: %s", m.Time.Local().Format("2 Jan 2006 15:04"), sender, m.Text)
		if withChat {
			if _, ok := names[m.Chat]; !ok {
				var group string
				_ = p.db.QueryRow(`SELECT name FROM chats WHERE jid = ?`, m.Chat).Scan(&group)
				names[m.Chat] = p.name(ctx, m.Chat, group, "")
			}
			msg.Chat = names[m.Chat]
			switch {
			case sender == "You":
				line = fmt.Sprintf("[%s] You to %s: %s", m.Time.Local().Format("2 Jan 2006 15:04"), msg.Chat, m.Text)
			case sender != msg.Chat:
				line = fmt.Sprintf("[%s] %s in %s: %s", m.Time.Local().Format("2 Jan 2006 15:04"), sender, msg.Chat, m.Text)
			}
		}
		out = append(out, msg)
		lines = append(lines, line)
	}
	return out, lines
}

func (p *plugin) read(call *sdk.Call, in readIn) (sdk.Result, error) {
	db, err := p.linkedDB()
	if err != nil {
		return sdk.Result{}, err
	}
	c, err := p.find(call.Context, in.Chat, false)
	if err != nil {
		return sdk.Result{}, err
	}
	ms, err := messages(db, c.JID, "", cmp.Or(in.Limit, 30))
	if err != nil {
		return sdk.Result{}, err
	}
	if len(ms) == 0 {
		return sdk.Result{Text: fmt.Sprintf("No messages with %s here yet.", c.Name), Data: []Message{}}, nil
	}
	out, lines := p.shown(call.Context, ms, false)
	return sdk.Result{Text: strings.Join(lines, "\n"), Data: out}, nil
}

func (p *plugin) search(call *sdk.Call, in searchIn) (sdk.Result, error) {
	if strings.TrimSpace(in.Query) == "" {
		return sdk.Result{}, errors.New("query is required")
	}
	db, err := p.linkedDB()
	if err != nil {
		return sdk.Result{}, err
	}
	chat := ""
	if in.Chat != "" {
		c, err := p.find(call.Context, in.Chat, false)
		if err != nil {
			return sdk.Result{}, err
		}
		chat = c.JID
	}
	ms, err := messages(db, chat, in.Query, cmp.Or(in.Limit, 30))
	if err != nil {
		return sdk.Result{}, err
	}
	if len(ms) == 0 {
		return sdk.Result{Text: "Nothing found.", Data: []Message{}}, nil
	}
	out, lines := p.shown(call.Context, ms, chat == "")
	return sdk.Result{Text: strings.Join(lines, "\n"), Data: out}, nil
}

func (p *plugin) send(call *sdk.Call, in sendIn) (sdk.Result, error) {
	if strings.TrimSpace(in.Text) == "" || strings.TrimSpace(in.Chat) == "" {
		return sdk.Result{}, errors.New("chat and text are required")
	}
	db, err := p.linkedDB()
	if err != nil {
		return sdk.Result{}, err
	}
	c, err := p.find(call.Context, in.Chat, true)
	if err != nil {
		return sdk.Result{}, err
	}
	to, err := types.ParseJID(c.JID)
	if err != nil {
		return sdk.Result{}, err
	}
	resp, err := p.client.SendMessage(call.Context, to, &waE2E.Message{Conversation: proto.String(in.Text)})
	if err != nil {
		return sdk.Result{}, err
	}
	me := p.client.Store.ID.ToNonAD().String()
	if err := put(db, stored{Chat: c.JID, ID: resp.ID, Sender: me, FromMe: true, Time: resp.Timestamp, Text: in.Text}, false); err != nil {
		p.c.Logf("whatsapp: storing a sent message: %v", err)
	}
	return sdk.Result{Text: fmt.Sprintf("Sent to %s.", c.Name), Data: Message{Chat: c.Name, Sender: "You", Time: resp.Timestamp, Text: in.Text}}, nil
}

func (p *plugin) release(call *sdk.Call, _ struct{}) (sdk.Result, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.client == nil || p.client.Store.ID == nil {
		return sdk.Result{}, errors.New("WhatsApp isn't linked on this device")
	}
	p.close()
	if err := call.PutFile(released, p.path()); err != nil {
		if err2 := p.open(); err2 == nil {
			_ = p.client.Connect()
		}
		return sdk.Result{}, fmt.Errorf("not released: storing the link failed: %w", err)
	}
	p.forget()
	p.serve(false)
	if _, err := call.Append("unlinked", struct{}{}); err != nil {
		return sdk.Result{}, err
	}
	return sdk.Text("Released WhatsApp: sign_in on another device to take it there. Messages sent meanwhile arrive when it does."), nil
}

func (p *plugin) signOut(call *sdk.Call, _ struct{}) (sdk.Result, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.client == nil || p.client.Store.ID == nil {
		return sdk.Result{}, errors.New("WhatsApp isn't linked on this device")
	}
	if err := p.client.Logout(call.Context); err != nil {
		return sdk.Result{}, err
	}
	p.forget()
	p.serve(false)
	if _, err := call.Append("unlinked", struct{}{}); err != nil {
		return sdk.Result{}, err
	}
	return sdk.Text("Signed out of WhatsApp, and deleted its messages here."), nil
}

func when(t time.Time) string {
	t = t.Local()
	now := time.Now()
	switch {
	case t.YearDay() == now.YearDay() && t.Year() == now.Year():
		return t.Format("15:04")
	case now.Sub(t) < 6*24*time.Hour:
		return t.Format("Mon 15:04")
	case t.Year() == now.Year():
		return t.Format("2 Jan")
	}
	return t.Format("2 Jan 2006")
}

// logger passes whatsmeow's warnings and errors to the node's log, through
// stderr: stdout is the node's connection.
type logger struct{ module string }

func (l logger) Errorf(msg string, args ...any) {
	fmt.Fprintf(os.Stderr, l.module+": "+msg+"\n", args...)
}
func (l logger) Warnf(msg string, args ...any) {
	fmt.Fprintf(os.Stderr, l.module+": "+msg+"\n", args...)
}
func (logger) Infof(string, ...any)        {}
func (logger) Debugf(string, ...any)       {}
func (l logger) Sub(m string) waLog.Logger { return logger{l.module + "/" + m} }
