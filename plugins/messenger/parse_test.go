package messenger

import (
	"reflect"
	"testing"
)

// The outlines follow messenger.com's, with made-up chats.

const chatList = `- grid "Chats"
- row "Group chat: 🕊️ Walkers 💡 More options for 🕊️ Walkers 💡"
  - gridcell "Group chat: 🕊️ Walkers 💡" [required=false]
    - generic
          - link "Group chat: 🕊️ Walkers 💡" [focusable=true]
- generic
  - statictext "Active now"
- generic
  - image "🕊️"
- statictext "Walkers"
- generic
  - image "💡. To get missing image descriptions, open the context menu."
- generic
  - statictext "Unread message:"
- statictext "Ana: 8am Saturday, \"bring water\""
- abbr "15 hours ago"
  - statictext "15h"
    - gridcell "More options for 🕊️ Walkers 💡" [required=false]
- button "More options for 🕊️ Walkers 💡" [expanded=false,focusable=true]
- row "Kim Park You: see you then 19 hours ago Seen by Kim Park More options for Kim Park"
  - gridcell "Kim Park You: see you then 19 hours ago Seen by Kim Park" [required=false]
          - link "Kim Park You: see you then 19 hours ago Seen by Kim Park" [focusable=true]
- statictext "Kim Park"
- statictext "You: see you then"
- abbr "19 hours ago"
  - statictext "19h"
- image "Seen by Kim Park"
- button "More options for Kim Park" [expanded=false,focusable=true]`

func TestParseChats(t *testing.T) {
	got := parseChats(parseOutline(chatList))
	want := []Chat{
		{Name: "🕊️ Walkers 💡", Last: `Ana: 8am Saturday, "bring water"`, Time: "15 hours ago", Unread: true, Group: true, link: "Group chat: 🕊️ Walkers 💡"},
		{Name: "Kim Park", Last: "You: see you then", Time: "19 hours ago", link: "Kim Park You: see you then 19 hours ago Seen by Kim Park"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

func TestParseMessages(t *testing.T) {
	outline := `- article
  - button "Enter, Message sent 2:51 AM by Kim Park: Lunch? It's 1:30 by the way" [focusable=true]
- statictext "Message sent 2:51 AM by Kim Park: Lunch? It's 1:30 by the way"
- article
  - button "Enter, Message sent 2:53 AM by You: Sure" [focusable=true]
- article
  - button "Enter, Message sent Mon 9:02 AM by Kim Park" [focusable=true]`
	got := parseMessages(parseOutline(outline))
	want := []Message{
		{Sender: "Kim Park", Time: "2:51 AM", Text: "Lunch? It's 1:30 by the way"},
		{Sender: "You", Time: "2:53 AM", Text: "Sure"},
		{Sender: "Kim Park", Time: "Mon 9:02 AM", Text: "(attachment)"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

func TestParseMatches(t *testing.T) {
	outline := `  - row "Kim Park the tailscale link broke 19 hours ago"
    - gridcell "Kim Park the tailscale link broke 19 hours ago" [required=false]
            - link "Kim Park the tailscale link broke 19 hours ago" [focusable=true]
- statictext "Kim Park"
- statictext "the"
- statictext "tailscale"
- statictext "link broke"
- abbr "19 hours ago"
  - statictext "19h"
  - row "Walkers 4 matched messages"
            - link "Walkers 4 matched messages" [focusable=true]
- statictext "Walkers"
- statictext "4 matched messages"`
	got := parseMatches(parseOutline(outline))
	want := []Match{{Chat: "Kim Park", Text: "the tailscale link broke", Time: "19 hours ago"}, {Chat: "Walkers", Matches: 4}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

func TestPick(t *testing.T) {
	chats := []Chat{{Name: "Kim Park"}, {Name: "Kim Lee"}, {Name: "Sam"}}
	if c, err := pick(chats, "sam"); err != nil || c.Name != "Sam" {
		t.Errorf("exact: %v %v", c, err)
	}
	if c, err := pick(chats, "Lee"); err != nil || c.Name != "Kim Lee" {
		t.Errorf("unique part: %v %v", c, err)
	}
	if _, err := pick(chats, "Kim"); err == nil {
		t.Error("an ambiguous part must fail")
	}
}

func TestChatsAcrossParts(t *testing.T) {
	// The first part ends inside Sam's row, after its link; the second
	// repeats the rows, Sam's without its link.
	first := chatList + `
- row "Sam Hi 2 hours ago More options for Sam"
  - link "Sam Hi 2 hours ago" [focusable=true]
- statictext "Sam"
- button "More options for Sam"`
	second := `- row "Kim Park More options for Kim Park"
- button "More options for Kim Park"
- row "Sam Hi 2 hours ago More options for Sam"
- statictext "Sam"
- statictext "Hi"
- abbr "2 hours ago"
- button "More options for Sam"`
	p := page{parts: [][]node{parseOutline(first), parseOutline(second)}}
	var got []string
	for _, c := range p.chats() {
		got = append(got, c.Name+"|"+c.Last+"|"+c.Time+"|"+c.link)
	}
	want := []string{"🕊️ Walkers 💡|Ana: 8am Saturday, \"bring water\"|15 hours ago|Group chat: 🕊️ Walkers 💡", "Kim Park|You: see you then|19 hours ago|Kim Park You: see you then 19 hours ago Seen by Kim Park", "Sam|Hi|2 hours ago|Sam Hi 2 hours ago"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %q\nwant %q", got, want)
	}
}
