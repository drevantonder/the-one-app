package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"

	"github.com/drevantonder/the-one-app/internal/bucket"
)

// setupR2 creates (or reuses) an R2 bucket and configures the node with S3
// credentials derived from a Cloudflare API token: the access key id is the
// token's id, and the secret is the SHA-256 of the token.
func setupR2(name string) error {
	token := os.Getenv("CLOUDFLARE_API_TOKEN")
	if token == "" {
		return errors.New("CLOUDFLARE_API_TOKEN is not set; it needs R2 read and write permission")
	}
	cf := func(method, path string, body any, out any) error {
		var rd io.Reader
		if body != nil {
			b, _ := json.Marshal(body)
			rd = bytes.NewReader(b)
		}
		req, _ := http.NewRequest(method, "https://api.cloudflare.com/client/v4"+path, rd)
		req.Header.Set("Authorization", "Bearer "+token)
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return err
		}
		defer resp.Body.Close()
		var env struct {
			Success bool                       `json:"success"`
			Errors  []struct{ Message string } `json:"errors"`
			Result  json.RawMessage            `json:"result"`
		}
		if err := json.NewDecoder(resp.Body).Decode(&env); err != nil {
			return err
		}
		if !env.Success {
			var msgs []string
			for _, e := range env.Errors {
				msgs = append(msgs, e.Message)
			}
			return fmt.Errorf("cloudflare %s %s: %s", method, path, strings.Join(msgs, "; "))
		}
		if out != nil {
			return json.Unmarshal(env.Result, out)
		}
		return nil
	}

	var tok struct {
		ID string `json:"id"`
	}
	if err := cf("GET", "/user/tokens/verify", nil, &tok); err != nil {
		return err
	}
	account := os.Getenv("CLOUDFLARE_ACCOUNT_ID")
	if account == "" {
		var accounts []struct{ ID, Name string }
		if err := cf("GET", "/accounts", nil, &accounts); err != nil {
			return err
		}
		if len(accounts) != 1 {
			return fmt.Errorf("the token can reach %d accounts; set CLOUDFLARE_ACCOUNT_ID", len(accounts))
		}
		account = accounts[0].ID
	}
	if err := cf("GET", "/accounts/"+account+"/r2/buckets/"+name, nil, nil); err != nil {
		if err := cf("POST", "/accounts/"+account+"/r2/buckets", map[string]string{"name": name, "locationHint": "oc"}, nil); err != nil {
			return err
		}
		fmt.Printf("Created R2 bucket %s.\n", name)
	}
	secret := sha256.Sum256([]byte(token))
	cfg := bucket.Config{
		Endpoint:        "https://" + account + ".r2.cloudflarestorage.com",
		Bucket:          name,
		Region:          "auto",
		AccessKeyID:     tok.ID,
		SecretAccessKey: hex.EncodeToString(secret[:]),
	}
	input, _ := json.Marshal(cfg)
	return run("one", "bucket_configure", input)
}
