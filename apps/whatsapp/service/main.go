// The WhatsApp service: links this device to a WhatsApp account, as
// WhatsApp Desktop is linked, keeps the messages it sees, and offers them to
// the person's devices through this device's node.
//
// It never tells WhatsApp the user is online or has read a message, so the
// phone keeps notifying and senders don't see reads.
package main

import (
	"bytes"
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

const app = "whatsapp"

type call struct {
	ID       uint64          `json:"id"`
	Function string          `json:"op"`
	Input    json.RawMessage `json:"input"`
}

func main() {
	home, _ := os.UserHomeDir()
	dir := cmp.Or(os.Getenv("WHATSAPP_DATA"), filepath.Join(home, ".local", "share", "one", "whatsapp"))
	node := "http://" + nodeAddr()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		log.Fatal(err)
	}
	w, err := openAll(dir, node)
	if err != nil {
		log.Fatal(err)
	}
	serve(node, w)
}

// serve registers with the node and answers its calls, registering again
// whenever the node forgets (it keeps services in memory) or restarts.
func serve(node string, w *whatsapp) {
	poll := &http.Client{Timeout: 40 * time.Second, Transport: toNode}
	for {
		if err := post(node+"/v1/services/"+app, describe); err != nil {
			log.Printf("registering: %v", err)
			time.Sleep(3 * time.Second)
			continue
		}
		for {
			c, err := next(poll, node)
			if err != nil {
				log.Printf("waiting for calls: %v", err)
				time.Sleep(time.Second)
				break
			}
			if c == nil {
				continue
			}
			go func() {
				result, err := w.run(context.Background(), c.Function, c.Input)
				reply := map[string]any{"ok": result}
				var away elsewhere
				switch {
				case errors.As(err, &away):
					reply = map[string]any{"elsewhere": err.Error()}
				case err != nil:
					reply = map[string]any{"error": err.Error()}
				}
				if err := post(fmt.Sprintf("%s/v1/services/%s/calls/%d", node, app, c.ID), reply); err != nil {
					log.Printf("answering %s: %v", c.Function, err)
				}
			}()
		}
	}
}

func next(client *http.Client, node string) (*call, error) {
	resp, err := client.Get(node + "/v1/services/" + app + "/calls")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	switch resp.StatusCode {
	case http.StatusNoContent:
		return nil, nil
	case http.StatusOK:
		var c call
		return &c, json.Unmarshal(body, &c)
	}
	return nil, fmt.Errorf("%s: %s", resp.Status, body)
}

// elsewhere is an error saying the call should go to another device, as
// when WhatsApp isn't linked here; the node then tries the next.
type elsewhere string

func (e elsewhere) Error() string { return string(e) }

func post(url string, body any) error {
	_, err := request(http.DefaultClient, url, body)
	return err
}

func request(client *http.Client, url string, body any) ([]byte, error) {
	var resp *http.Response
	var err error
	if body == nil {
		resp, err = client.Get(url)
	} else {
		b, merr := json.Marshal(body)
		if merr != nil {
			return nil, merr
		}
		resp, err = client.Post(url, "application/json", bytes.NewReader(b))
	}
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	msg, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		var e struct{ Error string }
		if json.Unmarshal(msg, &e) == nil && e.Error != "" {
			return nil, errors.New(e.Error)
		}
		return nil, errors.New(resp.Status + ": " + string(msg))
	}
	return msg, nil
}

func callOn(node, device, function string, input, out any) error {
	body, err := request(&http.Client{Timeout: 28 * time.Second, Transport: toNode}, node+"/v1/call",
		map[string]any{"app": app, "service": app, "op": function, "input": input, "device": device})
	if err != nil {
		return err
	}
	var reply struct{ Ok json.RawMessage }
	if err := json.Unmarshal(body, &reply); err != nil {
		return err
	}
	return json.Unmarshal(reply.Ok, out)
}

// describe is all the node is told of this service: what's about this machine. Its title, instructions and tools are
// its app's.
var describe = map[string]any{"version": built}
