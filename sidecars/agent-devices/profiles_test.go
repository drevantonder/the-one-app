package main

import (
	"bytes"
	"testing"
)

func TestSealedProfileOpensOnlyWithItsKeyAndSalt(t *testing.T) {
	plain := []byte("a profile's tarball")
	sealed, err := seal("key", "salt", plain)
	if err != nil || bytes.Contains(sealed, plain) {
		t.Fatalf("%v %q", err, sealed)
	}
	if got, err := unseal("key", "salt", sealed); err != nil || !bytes.Equal(got, plain) {
		t.Errorf("%v %q", err, got)
	}
	if _, err := unseal("key", "other salt", sealed); err == nil {
		t.Error("each save's salt makes its own key")
	}
	if _, err := unseal("other key", "salt", sealed); err == nil {
		t.Error("only its item's key opens it")
	}
}
