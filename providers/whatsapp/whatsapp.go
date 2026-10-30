package main

import (
	"cmp"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/skip2/go-qrcode"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

// link is this device's link to the account: whatsmeow's client, and the
// database it shares with the messages.
type link struct {
	db        *sql.DB
	container *sqlstore.Container

	mu     sync.Mutex
	client *whatsmeow.Client
	qr     string // the current QR code, while pairing
}

func open(path string) (*link, error) {
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)")
	if err != nil {
		return nil, err
	}
	l := &link{db: db, container: sqlstore.NewWithDB(db, "sqlite", logger{"whatsmeow"})}
	if err := l.container.Upgrade(context.Background()); err != nil {
		return nil, err
	}
	if _, err := db.Exec(schema); err != nil {
		return nil, err
	}
	store.SetOSInfo("The One App", [3]uint32{0, 2, 0})
	if err := l.fresh(); err != nil {
		return nil, err
	}
	if l.client.Store.ID != nil {
		if err := l.client.Connect(); err != nil {
			log.Printf("connecting: %v", err)
		}
	}
	return l, nil
}

// fresh makes a client for the stored device, or a new one to pair.
func (l *link) fresh() error {
	device, err := l.container.GetFirstDevice(context.Background())
	if err != nil {
		return err
	}
	l.client = whatsmeow.NewClient(device, logger{"whatsmeow"})
	l.client.AddEventHandler(l.handle)
	return nil
}

func (l *link) run(ctx context.Context, op string, raw json.RawMessage) (any, error) {
	var in struct {
		Phone, Chat, Query, Text string
		Unread                   bool
		Limit, Skip              int
	}
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &in); err != nil {
			return nil, err
		}
	}
	in.Limit = cmp.Or(in.Limit, 30)
	switch op {
	case "status":
		return l.status()
	case "sign_in":
		return l.signIn(ctx, in.Phone)
	}
	if err := l.linked(); err != nil {
		return nil, err
	}
	switch op {
	case "chats":
		return l.chats(ctx, in.Unread, in.Limit, in.Skip)
	case "read":
		c, err := l.find(ctx, in.Chat, false)
		if err != nil {
			return nil, err
		}
		ms, err := messages(l.db, c.jid, "", in.Limit, in.Skip)
		if len(ms) == 0 && err == nil {
			return fmt.Sprintf("No messages with %s here.", c.name), nil
		}
		return l.lines(ctx, ms, false), err
	case "search":
		if strings.TrimSpace(in.Query) == "" {
			return nil, errors.New("give a query")
		}
		chat := ""
		if in.Chat != "" {
			c, err := l.find(ctx, in.Chat, false)
			if err != nil {
				return nil, err
			}
			chat = c.jid
		}
		ms, err := messages(l.db, chat, in.Query, in.Limit, in.Skip)
		if len(ms) == 0 && err == nil {
			return "Nothing found.", nil
		}
		return l.lines(ctx, ms, chat == ""), err
	case "send":
		return l.send(ctx, in.Chat, in.Text)
	}
	return nil, fmt.Errorf("whatsapp has no operation %s", op)
}

func (l *link) linked() error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.client.Store.ID == nil {
		return errors.New("WhatsApp isn't linked here: whatsapp_sign_in links it")
	}
	return nil
}

func (l *link) status() (string, error) {
	l.mu.Lock()
	c := l.client
	l.mu.Unlock()
	var chats, msgs int
	_ = l.db.QueryRow(`SELECT (SELECT count(*) FROM chats), (SELECT count(*) FROM messages)`).Scan(&chats, &msgs)
	if c.Store.ID == nil {
		return fmt.Sprintf("Not linked: whatsapp_sign_in links it. %d messages in %d chats are kept here.", msgs, chats), nil
	}
	state := "connected"
	if !c.IsConnected() || !c.IsLoggedIn() {
		state = "not connected right now (it reconnects by itself)"
	}
	return fmt.Sprintf("Linked as +%s, %s. %d messages in %d chats are kept here.", c.Store.ID.User, state, msgs, chats), nil
}

func (l *link) signIn(ctx context.Context, phone string) (string, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.client.Store.ID != nil {
		return fmt.Sprintf("Already linked, as +%s.", l.client.Store.ID.User), nil
	}
	if l.qr == "" {
		l.client.Disconnect()
		if err := l.fresh(); err != nil {
			return "", err
		}
		qr, err := l.client.GetQRChannel(context.Background())
		if err != nil {
			return "", err
		}
		if err := l.client.Connect(); err != nil {
			return "", err
		}
		select {
		case first := <-qr:
			if first.Event != whatsmeow.QRChannelEventCode {
				return "", fmt.Errorf("WhatsApp didn't give a code: %s", first.Event)
			}
			l.qr = first.Code
		case <-time.After(20 * time.Second):
			l.client.Disconnect()
			return "", errors.New("WhatsApp didn't answer")
		}
		go func() { // new codes until the phone pairs or it times out
			for item := range qr {
				l.mu.Lock()
				l.qr = ""
				if item.Event == whatsmeow.QRChannelEventCode {
					l.qr = item.Code
				}
				l.mu.Unlock()
			}
		}()
	}
	if phone != "" {
		code, err := l.client.PairPhone(ctx, phone, true, whatsmeow.PairClientChrome, "Chrome (Linux)")
		if err != nil {
			return "", err
		}
		return fmt.Sprintf("On the phone, open WhatsApp > Settings > Linked devices > Link a device > Link with phone number instead, and enter %s. WhatsApp then sends recent chats here, which takes a few minutes.", code), nil
	}
	q, err := qrcode.New(l.qr, qrcode.Low)
	if err != nil {
		return "", err
	}
	return "On the phone, open WhatsApp > Settings > Linked devices > Link a device, and scan this within 20 seconds (sign_in again for a new one). WhatsApp then sends recent chats here, which takes a few minutes.\n\n" + q.ToSmallString(false), nil
}

func (l *link) handle(evt any) {
	ctx := context.Background()
	switch e := evt.(type) {
	case *events.PairSuccess:
		log.Printf("linked as +%s", e.ID.User)
	case *events.LoggedOut:
		log.Printf("unlinked by WhatsApp or the phone (%v)", e.Reason)
	case *events.StreamReplaced:
		log.Printf("another copy of this link connected; this one stopped")
	case *events.TemporaryBan:
		log.Printf("%v", e)
	case *events.Message:
		l.message(ctx, e, false)
	case *events.HistorySync:
		for _, conv := range e.Data.GetConversations() {
			jid, err := types.ParseJID(conv.GetID())
			if err != nil {
				continue
			}
			for _, m := range conv.GetMessages() {
				if msg, err := l.client.ParseWebMessage(jid, m.GetMessage()); err == nil {
					l.message(ctx, msg, true)
				}
			}
			name := ""
			if jid.Server == types.GroupServer {
				name = conv.GetName()
			}
			if err := setChat(l.db, l.key(ctx, jid), name, int(conv.GetUnreadCount()), time.Unix(int64(conv.GetConversationTimestamp()), 0)); err != nil {
				log.Printf("storing a chat: %v", err)
			}
		}
	case *events.MarkChatAsRead:
		unread := 0
		if !e.Action.GetRead() {
			unread = 1
		}
		_ = setUnread(l.db, l.key(ctx, e.JID), unread)
	case *events.GroupInfo:
		if e.Name != nil {
			_ = setName(l.db, e.JID.String(), e.Name.Name)
		}
	case *events.JoinedGroup:
		_ = setName(l.db, e.JID.String(), e.GroupName.Name)
	}
}

func (l *link) message(ctx context.Context, e *events.Message, history bool) {
	chat := l.key(ctx, e.Info.Chat)
	if pm := e.Message.GetProtocolMessage(); pm != nil {
		switch pm.GetType() {
		case waE2E.ProtocolMessage_REVOKE:
			_ = edit(l.db, chat, pm.GetKey().GetID(), "(deleted)")
		case waE2E.ProtocolMessage_MESSAGE_EDIT:
			if t := text(pm.GetEditedMessage()); t != "" {
				_ = edit(l.db, chat, pm.GetKey().GetID(), t)
			}
		}
		return
	}
	t := text(e.Message)
	if t == "" || e.Info.Chat.Server == types.BroadcastServer {
		return
	}
	if err := put(l.db, stored{Chat: chat, ID: e.Info.ID, Sender: l.key(ctx, e.Info.Sender), SenderName: e.Info.PushName,
		FromMe: e.Info.IsFromMe, Time: e.Info.Timestamp, Text: t}, history); err != nil {
		log.Printf("storing a message: %v", err)
	}
	if e.Info.IsGroup && !history && groupName(l.db, chat) == "" {
		go func() {
			if info, err := l.client.GetGroupInfo(ctx, e.Info.Chat); err == nil {
				_ = setName(l.db, chat, info.Name)
			}
		}()
	}
}

// key is how a chat or person is stored: by phone number where WhatsApp
// gave a private id (a LID) whose number is known, so each person is one
// chat.
func (l *link) key(ctx context.Context, jid types.JID) string {
	jid = jid.ToNonAD()
	if jid.Server == types.HiddenUserServer {
		if pn, err := l.client.Store.LIDs.GetPNForLID(ctx, jid); err == nil && !pn.IsEmpty() {
			jid = pn.ToNonAD()
		}
	}
	return jid.String()
}

// name is a chat's or person's name: a group's subject, or the contact's
// name, else the name they set, else their number.
func (l *link) name(ctx context.Context, jid, group, pushName string) string {
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
	if c, err := l.client.Store.Contacts.GetContact(ctx, j); err == nil {
		if n := cmp.Or(c.FullName, c.FirstName, c.BusinessName, c.PushName); n != "" {
			return n
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

func (l *link) chats(ctx context.Context, unread bool, limit, skip int) (string, error) {
	rows, err := chats(l.db)
	if err != nil {
		return "", err
	}
	var lines []string
	for _, r := range rows {
		if unread && r.Unread == 0 {
			continue
		}
		if skip > 0 {
			skip--
			continue
		}
		if len(lines) == limit {
			break
		}
		name := l.name(ctx, r.JID, r.Name, "")
		if r.Unread > 0 {
			name += fmt.Sprintf(" (%d unread)", r.Unread)
		}
		last := r.Last.Text
		switch {
		case r.Last.FromMe:
			last = "You: " + last
		case strings.HasSuffix(r.JID, "@"+types.GroupServer):
			last = l.name(ctx, r.Last.Sender, "", r.Last.SenderName) + ": " + last
		}
		lines = append(lines, fmt.Sprintf("%s, %s: %s", name, when(r.Last.Time), strings.Join(strings.Fields(last), " ")))
	}
	switch {
	case len(lines) > 0:
		return strings.Join(lines, "\n"), nil
	case unread:
		return "No unread chats.", nil
	case len(rows) == 0:
		return "No chats yet: after linking, WhatsApp takes a few minutes to send them.", nil
	}
	return "No more chats.", nil
}

type chat struct{ name, jid string }

var phone = regexp.MustCompile(`^\+?[\d\s().-]{7,}$`)

// find resolves a chat by JID, phone number, name, or a part of a name only
// one chat has. With contacts, a contact's exact name also counts, for
// people without a chat yet.
func (l *link) find(ctx context.Context, want string, contacts bool) (chat, error) {
	want = strings.TrimSpace(want)
	if want == "" {
		return chat{}, errors.New("give a chat")
	}
	if strings.Contains(want, "@") {
		jid, err := types.ParseJID(want)
		if err != nil {
			return chat{}, err
		}
		key := l.key(ctx, jid)
		return chat{l.name(ctx, key, groupName(l.db, key), ""), key}, nil
	}
	if phone.MatchString(want) {
		digits := strings.Map(func(r rune) rune {
			if r >= '0' && r <= '9' {
				return r
			}
			return -1
		}, want)
		if !strings.HasPrefix(want, "+") || strings.HasPrefix(digits, "0") {
			return chat{}, fmt.Errorf("give %q with its country code, e.g. +61…", want)
		}
		found, err := l.client.IsOnWhatsApp(ctx, []string{"+" + digits})
		if err != nil {
			return chat{}, err
		}
		if len(found) == 0 || !found[0].IsIn {
			return chat{}, fmt.Errorf("%s isn't on WhatsApp", want)
		}
		jid := found[0].JID.ToNonAD().String()
		return chat{l.name(ctx, jid, "", ""), jid}, nil
	}
	rows, err := chats(l.db)
	if err != nil {
		return chat{}, err
	}
	w := strings.ToLower(want)
	var partial []chat
	for _, r := range rows {
		c := chat{l.name(ctx, r.JID, r.Name, ""), r.JID}
		n := strings.ToLower(c.name)
		if n == w {
			return c, nil
		}
		if strings.Contains(n, w) {
			partial = append(partial, c)
		}
	}
	if contacts {
		var exact []chat
		if known, err := l.client.Store.Contacts.GetAllContacts(ctx); err == nil {
			for jid, c := range known {
				if jid.Server == types.DefaultUserServer && strings.EqualFold(c.FullName, want) {
					exact = append(exact, chat{c.FullName, jid.String()})
				}
			}
		}
		switch {
		case len(exact) == 1:
			return exact[0], nil
		case len(exact) > 1:
			return chat{}, fmt.Errorf("several contacts are called %q: give a phone number instead", want)
		}
	}
	switch len(partial) {
	case 1:
		return partial[0], nil
	case 0:
		return chat{}, fmt.Errorf("no chat called %q", want)
	}
	var names []string
	for _, c := range partial {
		names = append(names, c.name)
	}
	return chat{}, fmt.Errorf("%q matches several chats: %s", want, strings.Join(names, "; "))
}

// lines shows messages one to a line; withChat says in which chat each was.
func (l *link) lines(ctx context.Context, ms []stored, withChat bool) string {
	var out []string
	chats := map[string]string{}
	for _, m := range ms {
		at := m.Time.Local().Format("2 Jan 2006 15:04")
		sender := "You"
		if !m.FromMe {
			sender = l.name(ctx, m.Sender, "", m.SenderName)
		}
		line := fmt.Sprintf("[%s] %s: %s", at, sender, m.Text)
		if withChat {
			if _, ok := chats[m.Chat]; !ok {
				chats[m.Chat] = l.name(ctx, m.Chat, groupName(l.db, m.Chat), "")
			}
			switch c := chats[m.Chat]; {
			case m.FromMe:
				line = fmt.Sprintf("[%s] You to %s: %s", at, c, m.Text)
			case sender != c:
				line = fmt.Sprintf("[%s] %s in %s: %s", at, sender, c, m.Text)
			}
		}
		out = append(out, line)
	}
	return strings.Join(out, "\n")
}

func (l *link) send(ctx context.Context, to, msg string) (string, error) {
	if strings.TrimSpace(msg) == "" {
		return "", errors.New("give the text to send")
	}
	c, err := l.find(ctx, to, true)
	if err != nil {
		return "", err
	}
	jid, err := types.ParseJID(c.jid)
	if err != nil {
		return "", err
	}
	resp, err := l.client.SendMessage(ctx, jid, &waE2E.Message{Conversation: proto.String(msg)})
	if err != nil {
		return "", err
	}
	me := l.client.Store.ID.ToNonAD().String()
	if err := put(l.db, stored{Chat: c.jid, ID: resp.ID, Sender: me, FromMe: true, Time: resp.Timestamp, Text: msg}, false); err != nil {
		log.Printf("storing a sent message: %v", err)
	}
	return fmt.Sprintf("Sent to %s.", c.name), nil
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

// logger passes whatsmeow's warnings and errors to the journal.
type logger struct{ module string }

func (l logger) Errorf(msg string, args ...any) { log.Printf(l.module+": "+msg, args...) }
func (l logger) Warnf(msg string, args ...any)  { log.Printf(l.module+": "+msg, args...) }
func (logger) Infof(string, ...any)             {}
func (logger) Debugf(string, ...any)            {}
func (l logger) Sub(m string) waLog.Logger      { return logger{l.module + "/" + m} }
