package main

import (
	"bytes"
	"image"
	"image/color"
	"image/png"
	"log"

	"github.com/egoist/mygo"
)

// trayIcon is a 32-point template circle: the menu bar item's (a Mac's) or the notification area's icon.
func trayIcon() []byte {
	img := image.NewNRGBA(image.Rect(0, 0, 32, 32))
	for y := range 32 {
		for x := range 32 {
			dx, dy := x-16, y-16
			if r := dx*dx + dy*dy; r <= 14*14 && r >= 10*10 {
				img.Set(x, y, color.NRGBA{0, 0, 0, 255})
			}
		}
	}
	var b bytes.Buffer
	png.Encode(&b, img)
	return b.Bytes()
}

// trayProbe puts an item in the menu bar (a Mac's) or the notification area, with the home window and Quit, as the
// Mac app's menu bar item does.
func trayProbe() {
	mygo.App.WhenReady(func() {
		_, err := mygo.NewTray(mygo.TrayOptions{
			Icon: trayIcon(), IconIsTemplate: true, ToolTip: "The One App",
			Menu: mygo.NewMenu([]*mygo.MenuItem{
				{Label: "Open The One App", Click: func(*mygo.MenuItem, *mygo.Window) {
					if homeWin != nil {
						homeWin.Focus()
					}
				}},
				mygo.Separator(),
				{Role: mygo.RoleQuit},
			}),
		})
		if err != nil {
			log.Printf("probe tray: %v", err)
		} else {
			log.Printf("probe tray: shown")
		}
	})
}
