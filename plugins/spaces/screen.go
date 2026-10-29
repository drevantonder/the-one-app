package spaces

import (
	"bytes"
	"encoding/json"
	"errors"
	picture "image"
	"image/jpeg"
	_ "image/png"

	"github.com/drevantonder/the-one-app/sdk"
)

type screenIn struct {
	Space string `json:"space" jsonschema:"the Space"`
	Width int    `json:"width,omitempty" jsonschema:"the picture's width in pixels; default 480"`
}

// screen is a small JPEG of a Space's desktop, for thumbnails.
func (t *tools) screen(call *sdk.Call, in screenIn) (sdk.Image, error) {
	w := in.Width
	if w <= 0 {
		w = 480
	}
	raw, _ := json.Marshal(map[string]any{"space": in.Space, "max_image_dimension": min(w, 1920)})
	res, err := t.callAs(call, "get_desktop_state", raw, false)
	if err != nil {
		return sdk.Image{}, err
	}
	if len(res.Images) == 0 {
		return sdk.Image{}, errors.New("the Space sent no picture")
	}
	img, _, err := picture.Decode(bytes.NewReader(res.Images[0].Data))
	if err != nil {
		return sdk.Image{}, err
	}
	var b bytes.Buffer
	if err := jpeg.Encode(&b, img, &jpeg.Options{Quality: 70}); err != nil {
		return sdk.Image{}, err
	}
	return sdk.Image{MIMEType: "image/jpeg", Data: b.Bytes()}, nil
}
