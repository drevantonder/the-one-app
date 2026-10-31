package main

import (
	"bytes"
	"context"
	"encoding/binary"
	"fmt"
	"io"
	"net/http"
	"strings"
)

// viewerPath mints a viewer ticket: a credential that only lets a browser
// watch and use the desktop (with the clipboard and the home folder), for
// ttl seconds. It is SystemService.CreateViewerTicket over gRPC-Web; the
// two messages are small enough to encode by hand.
func viewerPath(ctx context.Context, d desktop, ttl int) (string, error) {
	var msg []byte
	ttlMsg := append([]byte{0x08}, binary.AppendUvarint(nil, uint64(ttl))...) // Duration.seconds
	msg = append(msg, 0x0a, byte(len(ttlMsg)))                                // 1: ttl
	msg = append(msg, ttlMsg...)
	msg = append(msg, 0x18, 0x01)      // 3: clipboard
	msg = append(msg, 0x22, 0x01, '~') // 4: files_root
	frame := append([]byte{0, 0, 0, 0, byte(len(msg))}, msg...)

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, d.local()+"/cua.env.v1.SystemService/CreateViewerTicket", bytes.NewReader(frame))
	if err != nil {
		return "", err
	}
	req.Header.Set("content-type", "application/grpc-web+proto")
	req.Header.Set("x-grpc-web", "1")
	req.Header.Set("authorization", "Bearer "+d.token)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("viewer ticket: %s", resp.Status)
	}
	var path, status string
	for len(body) >= 5 {
		flag, n := body[0], int(binary.BigEndian.Uint32(body[1:5]))
		if len(body) < 5+n {
			break
		}
		m := body[5 : 5+n]
		body = body[5+n:]
		if flag&0x80 != 0 { // trailers
			status = string(m)
			continue
		}
		path = stringField(m, 3)
	}
	if path == "" {
		return "", fmt.Errorf("viewer ticket: %s", strings.TrimSpace(status))
	}
	return path, nil
}

// stringField reads a top-level length-delimited field from a protobuf
// message.
func stringField(m []byte, field uint64) string {
	for len(m) > 0 {
		key, n := binary.Uvarint(m)
		if n <= 0 {
			return ""
		}
		m = m[n:]
		switch key & 7 {
		case 0:
			_, n = binary.Uvarint(m)
			if n <= 0 {
				return ""
			}
			m = m[n:]
		case 2:
			l, n := binary.Uvarint(m)
			if n <= 0 || uint64(len(m)-n) < l {
				return ""
			}
			v := m[n : n+int(l)]
			m = m[n+int(l):]
			if key>>3 == field {
				return string(v)
			}
		default:
			return ""
		}
	}
	return ""
}
