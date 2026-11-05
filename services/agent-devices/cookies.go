package main

import (
	"bytes"
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/pbkdf2"
	"crypto/sha1"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// A browser profile is a Linux Chrome's, its cookies sealed with Linux's basic key (--password-store=basic), which a
// Mac's Chrome, keyed by its keychain, can't open and drops. So on a Mac, takeCookies opens them here, before Chrome
// starts, and Chrome is handed them over DevTools once it has (Storage.setCookies). Chrome's own sqlite3 file is
// read with the Mac's own sqlite3: no dependency.

// linuxKeys are the keys Linux's basic store seals with: v10's is "peanuts", v11's an empty password.
var linuxKeys = map[string][]byte{"v10": linuxKey("peanuts"), "v11": linuxKey("")}

func linuxKey(password string) []byte {
	k, _ := pbkdf2.Key(sha1.New, password, []byte("saltysalt"), 1, 16)
	return k
}

// openCookie is a sealed cookie value in the clear, or "" if it isn't one Linux's basic store sealed.
func openCookie(sealed []byte, host string, version int) (string, bool) {
	if len(sealed) < 3+aes.BlockSize || (len(sealed)-3)%aes.BlockSize != 0 {
		return "", false
	}
	key, ok := linuxKeys[string(sealed[:3])]
	if !ok {
		return "", false
	}
	block, _ := aes.NewCipher(key)
	plain := make([]byte, len(sealed)-3)
	cipher.NewCBCDecrypter(block, bytes.Repeat([]byte{' '}, aes.BlockSize)).CryptBlocks(plain, sealed[3:])
	pad := int(plain[len(plain)-1])
	if pad < 1 || pad > aes.BlockSize || !bytes.Equal(plain[len(plain)-pad:], bytes.Repeat([]byte{byte(pad)}, pad)) {
		return "", false
	}
	plain = plain[:len(plain)-pad]
	// From the database's version 24, a value starts with its domain's SHA-256.
	if h := sha256.Sum256([]byte(host)); version >= 24 && bytes.HasPrefix(plain, h[:]) {
		plain = plain[len(h):]
	}
	return string(plain), true
}

// takeCookies opens the cookies in a chrome device's profile and deletes its cookie files, for Chrome to start
// without them. Partitioned cookies, and ones expired or that won't open, are left out.
func takeCookies(ctx context.Context, dir string) ([]map[string]any, error) {
	var db string
	for _, f := range []string{"Default/Network/Cookies", "Default/Cookies"} {
		if _, err := os.Stat(filepath.Join(dir, f)); err == nil {
			db = filepath.Join(dir, f)
			break
		}
	}
	if db == "" {
		return nil, nil
	}
	defer func() {
		for _, f := range []string{"Default/Network/Cookies", "Default/Cookies"} {
			os.Remove(filepath.Join(dir, f))
			os.Remove(filepath.Join(dir, f+"-journal"))
		}
	}()
	query := func(sql string) ([]map[string]any, error) {
		out, err := exec.CommandContext(ctx, "sqlite3", "-readonly", "-json", db, sql).Output()
		if err != nil {
			var e *exec.ExitError
			if errors.As(err, &e) {
				err = errors.New(strings.TrimSpace(string(e.Stderr)))
			}
			return nil, fmt.Errorf("reading the profile's sign-ins: %v", err)
		}
		var rows []map[string]any
		if len(bytes.TrimSpace(out)) > 0 {
			if err := json.Unmarshal(out, &rows); err != nil {
				return nil, fmt.Errorf("reading the profile's sign-ins: %v", err)
			}
		}
		return rows, nil
	}
	meta, err := query(`select value from meta where key = 'version'`)
	if err != nil {
		return nil, err
	}
	version := 0
	if len(meta) > 0 {
		version, _ = strconv.Atoi(fmt.Sprint(meta[0]["value"]))
	}
	rows, err := query(`select *, hex(encrypted_value) as sealed from cookies`)
	if err != nil {
		return nil, err
	}
	str := func(r map[string]any, k string) string { s, _ := r[k].(string); return s }
	num := func(r map[string]any, k string) int64 { n, _ := r[k].(float64); return int64(n) }
	now := time.Now().Unix()
	var cookies []map[string]any
	for _, r := range rows {
		if str(r, "top_frame_site_key") != "" {
			continue
		}
		host, value := str(r, "host_key"), str(r, "value")
		if sealed, _ := hex.DecodeString(str(r, "sealed")); len(sealed) > 0 {
			var ok bool
			if value, ok = openCookie(sealed, host, version); !ok {
				continue
			}
		}
		c := map[string]any{"name": str(r, "name"), "value": value, "domain": host, "path": str(r, "path"),
			"secure": num(r, "is_secure") == 1, "httpOnly": num(r, "is_httponly") == 1}
		// Chrome counts microseconds from 1601.
		if _, has := r["has_expires"]; num(r, "has_expires") == 1 || !has && num(r, "expires_utc") > 0 {
			expires := num(r, "expires_utc")/1e6 - 11644473600
			if expires <= now {
				continue
			}
			c["expires"] = expires
		}
		if s, ok := map[int64]string{0: "None", 1: "Lax", 2: "Strict"}[num(r, "samesite")]; ok {
			c["sameSite"] = s
		}
		if p, ok := map[int64]string{0: "Low", 1: "Medium", 2: "High"}[num(r, "priority")]; ok {
			c["priority"] = p
		}
		cookies = append(cookies, c)
	}
	return cookies, nil
}

// giveCookies hands Chrome the cookies takeCookies opened: all at once, or, if one of them is refused, each alone.
func (b *browser) giveCookies(ctx context.Context, cookies []map[string]any) {
	if len(cookies) == 0 {
		return
	}
	if _, err := b.call(ctx, "", "Storage.setCookies", map[string]any{"cookies": cookies}); err == nil {
		return
	}
	for _, c := range cookies {
		b.call(ctx, "", "Storage.setCookies", map[string]any{"cookies": []map[string]any{c}})
	}
}
