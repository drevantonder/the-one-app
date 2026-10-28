package bucket

import (
	"bufio"
	"bytes"
	"crypto/rand"
	"testing"

	"golang.org/x/crypto/chacha20poly1305"
)

func TestStreamRoundTrip(t *testing.T) {
	key := make([]byte, chacha20poly1305.KeySize)
	_, _ = rand.Read(key)
	aead, _ := chacha20poly1305.NewX(key)
	for _, size := range []int{0, 1, chunkSize - 1, chunkSize, chunkSize + 1, 3*chunkSize + 7} {
		plain := make([]byte, size)
		_, _ = rand.Read(plain)
		var sealed bytes.Buffer
		if err := sealStream(aead, bufio.NewReader(bytes.NewReader(plain)), &sealed); err != nil {
			t.Fatal(err)
		}
		var got bytes.Buffer
		if err := openStream(aead, bytes.NewReader(sealed.Bytes()), &got); err != nil {
			t.Fatalf("%d bytes: %v", size, err)
		}
		if !bytes.Equal(got.Bytes(), plain) {
			t.Fatalf("%d bytes: content differs", size)
		}
		if size > chunkSize { // dropping the last chunk must be noticed
			cut := sealed.Bytes()[:16+4+chunkSize+aead.Overhead()]
			if err := openStream(aead, bytes.NewReader(cut), &bytes.Buffer{}); err == nil {
				t.Errorf("%d bytes: a truncated file opened", size)
			}
		}
		tampered := bytes.Clone(sealed.Bytes())
		tampered[len(tampered)-1] ^= 1
		if err := openStream(aead, bytes.NewReader(tampered), &bytes.Buffer{}); err == nil {
			t.Errorf("%d bytes: a tampered file opened", size)
		}
	}
}
