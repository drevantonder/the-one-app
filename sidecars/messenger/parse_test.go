package main

import (
	"reflect"
	"testing"
)

// The outlines follow messenger.com's, with made-up chats.

const chatList = `- grid "Chats"
- row "Group chat: 🕊️ Walkers 💡 More options for 🕊️ Walkers 💡"
  - gridcell "Group chat: 🕊️ Walkers 💡" [required=false]
          - link "Group chat: 🕊️ Walkers 💡" [focusable=true]
- statictext "Active now"
- image "🕊️"
- statictext "Walkers"
- image "💡. To get missing image descriptions, open the context menu."
- statictext "Unread message:"
- statictext "Ana: 8am Saturday, \"bring water\""
- abbr "15 hours ago"
  - statictext "15h"
- button "More options for 🕊️ Walkers 💡" [expanded=false,focusable=true]
- row "Kim Park You: see you then 19 hours ago Seen by Kim Park More options for Kim Park"
          - link "Kim Park You: see you then 19 hours ago Seen by Kim Park" [focusable=true]
- statictext "Kim Park"
- statictext "You: see you then"
- abbr "19 hours ago"
  - statictext "19h"
- image "Seen by Kim Park"
- button "More options for Kim Park" [expanded=false,focusable=true]`

func TestChats(t *testing.T) {
	got := parseChats(parseOutline(chatList))
	want := []Chat{
		{Name: "🕊️ Walkers 💡", Last: `Ana: 8am Saturday, "bring water"`, Time: "15 hours ago", Unread: true, Group: true, link: "Group chat: 🕊️ Walkers 💡"},
		{Name: "Kim Park", Last: "You: see you then", Time: "19 hours ago", link: "Kim Park You: see you then 19 hours ago Seen by Kim Park"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

func TestChatCutBetweenParts(t *testing.T) {
	first := parseOutline(`- row "Kim Park You: hi 2 hours ago More options for Kim Park"
          - link "Kim Park You: hi 2 hours ago" [focusable=true]
- statictext "Kim Park"`)
	second := parseOutline(`- row "Kim Park You: hi 2 hours ago More options for Kim Park"
- statictext "Kim Park"
- statictext "You: hi"
- abbr "2 hours ago"`)
	got := page{parts: [][]node{first, second}}.chats()
	want := []Chat{{Name: "Kim Park", Last: "You: hi", Time: "2 hours ago", link: "Kim Park You: hi 2 hours ago"}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

func TestMessages(t *testing.T) {
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

func TestMatches(t *testing.T) {
	outline := `  - row "Kim Park the tailscale link broke 19 hours ago"
            - link "Kim Park the tailscale link broke 19 hours ago" [focusable=true]
- statictext "Kim Park"
- statictext "the"
- statictext "tailscale"
- statictext "link broke"
- abbr "19 hours ago"
  - row "Walkers 4 matched messages"
            - link "Walkers 4 matched messages" [focusable=true]
- statictext "Walkers"
- statictext "4 matched messages"`
	got := parseMatches(parseOutline(outline))
	want := []Match{
		{Chat: "Kim Park", Text: "the tailscale link broke", Time: "19 hours ago"},
		{Chat: "Walkers", Matches: 4},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

func TestPick(t *testing.T) {
	chats := []Chat{{Name: "Kim Park"}, {Name: "Kim Lee"}, {Name: "Walkers"}}
	if c, err := pick(chats, "walk"); err != nil || c.Name != "Walkers" {
		t.Errorf("a part only one chat has: %v %v", c, err)
	}
	if _, err := pick(chats, "kim"); err == nil {
		t.Error("a part several chats have is refused")
	}
	if c, err := pick(chats, "kim park"); err != nil || c.Name != "Kim Park" {
		t.Errorf("a whole name: %v %v", c, err)
	}
	if _, err := pick(chats, "Sam"); err == nil {
		t.Error("no chat")
	}
}

func TestSignedOut(t *testing.T) {
	if !(page{refs: []ref{{Ref: "p1:3", Role: "button", Name: "Log In"}}}).signedOut() {
		t.Error("messenger.com's sign-in page")
	}
	if (page{url: home}).signedOut() {
		t.Error("the chat list")
	}
}
