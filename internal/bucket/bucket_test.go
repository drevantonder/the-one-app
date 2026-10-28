package bucket

import (
	"crypto/rand"
	"encoding/json"
	"testing"

	"github.com/drevantonder/the-one-app/protocol"
)

func TestSegmentsRoundTripAndRejectWrongKey(t *testing.T) {
	key := make([]byte, 32)
	rand.Read(key)
	b := &Bucket{key: key}
	in := []protocol.Entry{{ID: "d:1", Device: "d", Seq: 1, HLC: 5, Plugin: "memory", Type: "note", Payload: json.RawMessage(`{"text":"hi"}`)}}
	sealed, err := b.encode(in)
	if err != nil {
		t.Fatal(err)
	}
	out, err := b.decode(sealed)
	if err != nil || len(out) != 1 || string(out[0].Payload) != `{"text":"hi"}` {
		t.Fatalf("decode = %v, %v", out, err)
	}
	other := make([]byte, 32)
	rand.Read(other)
	if _, err := (&Bucket{key: other}).decode(sealed); err == nil {
		t.Fatal("a different key must not decrypt")
	}
	if kit, err := DecodeKit("not a kit"); err == nil {
		t.Fatalf("DecodeKit accepted garbage: %v", kit)
	}
}
