package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"image"
	"image/jpeg"
	"log"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

// The viewer: a page to watch and use a device of another kind than linux (Linux devices have Cua's), served on
// viewerPort over HTTPS on the tailnet, as theirs is. It shows the device's screen as fast as it can be taken, and
// sends clicks, typing and keys back, where the kind takes them. A link's ticket is what lets someone in, for as long
// as the link works.
const viewerPort = 32099

type ticket struct {
	device string
	until  time.Time
}

var (
	ticketsMu sync.Mutex
	tickets   = map[string]ticket{}
)

// vmViewURL is a link to watch and use a device for minutes.
func vmViewURL(ctx context.Context, v *vm, minutes int) (string, error) {
	port := strconv.Itoa(viewerPort)
	if _, err := tailscale(ctx, "serve", "--bg", "--https="+port, "http://127.0.0.1:"+port); err != nil {
		return "", err
	}
	out, err := tailscale(ctx, "status", "--json")
	if err != nil {
		return "", err
	}
	var st struct{ Self struct{ DNSName string } }
	if err := json.Unmarshal([]byte(out), &st); err != nil {
		return "", err
	}
	b := make([]byte, 24)
	rand.Read(b)
	t := hex.EncodeToString(b)
	ticketsMu.Lock()
	tickets[t] = ticket{device: v.Name, until: time.Now().Add(time.Duration(minutes) * time.Minute)}
	ticketsMu.Unlock()
	return fmt.Sprintf("https://%s:%s/%s/", strings.TrimSuffix(st.Self.DNSName, "."), port, t), nil
}

// serveViewer serves the viewer for as long as this runs.
func (ds *devices) serveViewer() {
	mux := http.NewServeMux()
	mux.HandleFunc("/{ticket}/{what...}", func(w http.ResponseWriter, r *http.Request) {
		ticketsMu.Lock()
		t, ok := tickets[r.PathValue("ticket")]
		ticketsMu.Unlock()
		if !ok || time.Now().After(t.until) {
			http.Error(w, "This link has expired: ask for another.", http.StatusForbidden)
			return
		}
		v := ds.vmNamed(t.device)
		if v == nil {
			http.Error(w, "That device is gone.", http.StatusNotFound)
			return
		}
		ds.touch(v.Name) // someone's watching it
		k := v.kind()
		switch r.PathValue("what") {
		case "":
			w.Header().Set("Content-Type", "text/html; charset=utf-8")
			w.Header().Set("Cache-Control", "no-store")
			fmt.Fprintf(w, viewerPage, v.Name, k.input(), k.limits())
		case "screen":
			if !k.running(r.Context(), v) {
				http.Error(w, v.Name+" is stopped.", http.StatusConflict)
				return
			}
			img, err := k.screen(r.Context(), v)
			if err != nil {
				http.Error(w, err.Error(), http.StatusBadGateway)
				return
			}
			b, _, _ := jpegOf(img, 0)
			w.Header().Set("Content-Type", "image/jpeg")
			w.Header().Set("Cache-Control", "no-store")
			w.Header().Set("X-Width", strconv.Itoa(img.Bounds().Dx()))
			w.Header().Set("X-Height", strconv.Itoa(img.Bounds().Dy()))
			w.Write(b)
		case "act":
			var a action
			if r.Method != http.MethodPost || json.NewDecoder(r.Body).Decode(&a) != nil {
				http.Error(w, "an action, posted", http.StatusBadRequest)
				return
			}
			if err := k.act(r.Context(), v, a); err != nil {
				http.Error(w, err.Error(), http.StatusBadRequest)
			}
		default:
			http.NotFound(w, r)
		}
	})
	srv := &http.Server{Addr: "127.0.0.1:" + strconv.Itoa(viewerPort), Handler: mux, ReadHeaderTimeout: 10 * time.Second}
	if err := srv.ListenAndServe(); err != nil {
		log.Printf("the viewer: %v", err)
	}
}

// The page: the screen, filling the window, and what the person does on it sent back as actions.
const viewerPage = `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>%[1]s</title>
<style>
  html, body { margin: 0; height: 100%%; background: #111; overflow: hidden; }
  img { position: absolute; inset: 0; width: 100%%; height: 100%%; object-fit: contain; user-select: none; -webkit-user-drag: none; outline: none; }
  #note { position: absolute; left: 0; right: 0; bottom: 0; padding: 6px 12px; font: 13px system-ui; color: #ccc; background: #000a; display: none; }
</style>
<img id="s" tabindex="0" alt="">
<div id="note"></div>
<script>
  const input = %[2]t, s = document.getElementById("s"), note = document.getElementById("note");
  let w = 0, h = 0;
  if (!input) { note.textContent = %[3]q; note.style.display = "block"; }
  async function loop() {
    for (;;) {
      try {
        const r = await fetch("screen", { cache: "no-store" });
        if (r.ok) {
          w = +r.headers.get("X-Width"); h = +r.headers.get("X-Height");
          const url = URL.createObjectURL(await r.blob());
          await new Promise((done) => { s.onload = s.onerror = done; s.src = url; });
          URL.revokeObjectURL(s.dataset.was || "");
          s.dataset.was = url;
        } else {
          note.textContent = await r.text(); note.style.display = "block";
          await new Promise((t) => setTimeout(t, 2000));
        }
      } catch { await new Promise((t) => setTimeout(t, 2000)); }
    }
  }
  loop();
  // A point on the page, in the device's pixels.
  function at(e) {
    const r = s.getBoundingClientRect(), scale = Math.min(r.width / w, r.height / h);
    const x = (e.clientX - r.left - (r.width - w * scale) / 2) / scale, y = (e.clientY - r.top - (r.height - h * scale) / 2) / scale;
    return { x: Math.max(0, Math.min(w - 1, x)), y: Math.max(0, Math.min(h - 1, y)) };
  }
  const act = (a) => input && fetch("act", { method: "POST", body: JSON.stringify(a) });
  let down = null;
  s.addEventListener("pointerdown", (e) => { s.focus(); down = at(e); e.preventDefault(); });
  s.addEventListener("pointerup", (e) => {
    if (!down) return;
    const up = at(e), moved = Math.hypot(up.x - down.x, up.y - down.y) > 12;
    act(moved ? { do: "swipe", x: down.x, y: down.y, to_x: up.x, to_y: up.y } : { do: e.button === 2 ? "right_click" : "click", ...up });
    down = null;
  });
  s.addEventListener("contextmenu", (e) => e.preventDefault());
  s.addEventListener("wheel", (e) => { e.preventDefault(); const p = at(e); act({ do: "scroll", ...p, to_x: p.x, to_y: p.y - e.deltaY }); }, { passive: false });
  const named = { Enter: "enter", Backspace: "backspace", Tab: "tab", Escape: "escape", ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Delete: "delete", Home: "home", End: "end" };
  s.addEventListener("keydown", (e) => {
    const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.metaKey && "win"].filter(Boolean);
    if (e.key.length === 1 && !mods.length) act({ do: "type", text: e.key });
    else if (named[e.key] || (e.key.length === 1 && mods.length)) act({ do: "key", key: [...mods, named[e.key] || e.key.toLowerCase()].join("-") });
    else return;
    e.preventDefault();
  });
</script>`

// jpegOf is an image as a JPEG, scaled to width (0: its own size, at most 1600 wide), with the size it is.
func jpegOf(img image.Image, width int) ([]byte, int, int) {
	b := img.Bounds()
	if width <= 0 {
		width = min(b.Dx(), 1600)
	}
	if width < b.Dx() {
		img = shrink(img, width, b.Dy()*width/b.Dx())
	}
	var out bytes.Buffer
	jpeg.Encode(&out, img, &jpeg.Options{Quality: 75})
	return out.Bytes(), img.Bounds().Dx(), img.Bounds().Dy()
}

func b64(b []byte) string { return base64.StdEncoding.EncodeToString(b) }

func decodeFile(path string) (image.Image, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	img, _, err := image.Decode(f)
	return img, err
}

// listening reports whether something here listens on the port.
func listening(port int) bool {
	c, err := net.DialTimeout("tcp", "127.0.0.1:"+strconv.Itoa(port), 200*time.Millisecond)
	if err == nil {
		c.Close()
	}
	return err == nil
}

// shrink scales an image down to w by h, each pixel the average of those it covers.
func shrink(img image.Image, w, h int) image.Image {
	b := img.Bounds()
	out := image.NewRGBA(image.Rect(0, 0, w, h))
	for y := range h {
		y0, y1 := b.Min.Y+y*b.Dy()/h, b.Min.Y+max((y+1)*b.Dy()/h, y*b.Dy()/h+1)
		for x := range w {
			x0, x1 := b.Min.X+x*b.Dx()/w, b.Min.X+max((x+1)*b.Dx()/w, x*b.Dx()/w+1)
			var r, g, bl, n uint32
			for sy := y0; sy < y1; sy++ {
				for sx := x0; sx < x1; sx++ {
					cr, cg, cb, _ := img.At(sx, sy).RGBA()
					r, g, bl, n = r+cr, g+cg, bl+cb, n+1
				}
			}
			i := out.PixOffset(x, y)
			out.Pix[i], out.Pix[i+1], out.Pix[i+2], out.Pix[i+3] = uint8(r/n>>8), uint8(g/n>>8), uint8(bl/n>>8), 255
		}
	}
	return out
}
