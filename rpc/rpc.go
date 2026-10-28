// Package rpc is a minimal bidirectional JSON-RPC 2.0 connection over
// newline-delimited JSON. Either side can send requests.
package rpc

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sync"
	"sync/atomic"
)

type Handler func(ctx context.Context, method string, params json.RawMessage) (any, error)

// Error is a JSON-RPC error. Handlers can return one to control the code.
type Error struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

func (e *Error) Error() string { return e.Message }

type message struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      *int64          `json:"id,omitempty"`
	Method  string          `json:"method,omitempty"`
	Params  json.RawMessage `json:"params,omitempty"`
	Result  json.RawMessage `json:"result,omitempty"`
	Error   *Error          `json:"error,omitempty"`
}

type Conn struct {
	r          *bufio.Reader
	w          io.Writer
	wmu        sync.Mutex
	handler    Handler
	sequential bool
	queue      chan message

	next    atomic.Int64
	mu      sync.Mutex
	pending map[int64]chan message
	closed  chan struct{}
	err     error
}

// New creates a connection. If sequential is true, incoming requests are
// handled one at a time in arrival order; otherwise each gets a goroutine.
// Responses to our own calls are always delivered immediately, so a handler
// may call back into the peer without deadlocking.
func New(r io.Reader, w io.Writer, h Handler, sequential bool) *Conn {
	return &Conn{
		r:          bufio.NewReaderSize(r, 1<<20),
		w:          w,
		handler:    h,
		sequential: sequential,
		queue:      make(chan message, 64),
		pending:    map[int64]chan message{},
		closed:     make(chan struct{}),
	}
}

// Run reads messages until the reader closes or ctx ends.
func (c *Conn) Run(ctx context.Context) error {
	if c.sequential {
		go func() {
			for m := range c.queue {
				c.serve(ctx, m)
			}
		}()
		defer close(c.queue)
	}
	err := c.read(ctx)
	c.mu.Lock()
	c.err = err
	for id, ch := range c.pending {
		close(ch)
		delete(c.pending, id)
	}
	c.mu.Unlock()
	close(c.closed)
	return err
}

func (c *Conn) read(ctx context.Context) error {
	for {
		line, err := c.r.ReadBytes('\n')
		if len(line) > 0 {
			var m message
			if jerr := json.Unmarshal(line, &m); jerr != nil {
				return fmt.Errorf("rpc: bad message: %w", jerr)
			}
			switch {
			case m.Method != "":
				if c.sequential {
					select {
					case c.queue <- m:
					case <-ctx.Done():
						return ctx.Err()
					}
				} else {
					go c.serve(ctx, m)
				}
			case m.ID != nil:
				c.mu.Lock()
				ch := c.pending[*m.ID]
				delete(c.pending, *m.ID)
				c.mu.Unlock()
				if ch != nil {
					ch <- m
				}
			}
		}
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}
	}
}

func (c *Conn) serve(ctx context.Context, m message) {
	res, err := c.handler(ctx, m.Method, m.Params)
	if m.ID == nil {
		return
	}
	out := message{JSONRPC: "2.0", ID: m.ID}
	if err != nil {
		var re *Error
		if !errors.As(err, &re) {
			re = &Error{Code: -32000, Message: err.Error()}
		}
		out.Error = re
	} else {
		b, merr := json.Marshal(res)
		if merr != nil {
			out.Error = &Error{Code: -32603, Message: merr.Error()}
		} else {
			out.Result = b
		}
	}
	_ = c.write(out)
}

func (c *Conn) write(m message) error {
	b, err := json.Marshal(m)
	if err != nil {
		return err
	}
	b = append(b, '\n')
	c.wmu.Lock()
	defer c.wmu.Unlock()
	_, err = c.w.Write(b)
	return err
}

// Call sends a request and decodes the result into out (if non-nil).
func (c *Conn) Call(ctx context.Context, method string, params, out any) error {
	id := c.next.Add(1)
	raw, err := json.Marshal(params)
	if err != nil {
		return err
	}
	ch := make(chan message, 1)
	c.mu.Lock()
	select {
	case <-c.closed:
		c.mu.Unlock()
		return errors.New("rpc: connection closed")
	default:
	}
	c.pending[id] = ch
	c.mu.Unlock()
	if err := c.write(message{JSONRPC: "2.0", ID: &id, Method: method, Params: raw}); err != nil {
		c.mu.Lock()
		delete(c.pending, id)
		c.mu.Unlock()
		return err
	}
	select {
	case m, ok := <-ch:
		if !ok {
			return errors.New("rpc: connection closed")
		}
		if m.Error != nil {
			return m.Error
		}
		if out != nil && len(m.Result) > 0 {
			return json.Unmarshal(m.Result, out)
		}
		return nil
	case <-ctx.Done():
		c.mu.Lock()
		delete(c.pending, id)
		c.mu.Unlock()
		return ctx.Err()
	}
}

// Done is closed when the connection stops reading.
func (c *Conn) Done() <-chan struct{} { return c.closed }
