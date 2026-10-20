package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"image"
	"image/jpeg"
	"net"
	"os"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// Our own profile photo, and the default disappearing message timer new
// chats start with.

// profilePhotoEdge is the size WhatsApp keeps profile photos at.
const profilePhotoEdge = 640

type SetOwnPhotoData struct {
	Path   string `json:"path"`
	Remove bool   `json:"remove"`
}

type DefaultTimerData struct {
	Seconds int `json:"seconds"`
}

// profilePhotoJPEG makes a picture into a profile photo: its middle square,
// at most profilePhotoEdge across, as a JPEG.
func profilePhotoJPEG(path string) ([]byte, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	img, _, err := image.Decode(bytes.NewReader(data))
	if err != nil {
		return nil, fmt.Errorf("that picture can't be read")
	}
	img = applyEXIFOrientation(img, jpegOrientation(data))
	b := img.Bounds()
	side := min(b.Dx(), b.Dy())
	square := image.Rect(0, 0, side, side)
	x0, y0 := b.Min.X+(b.Dx()-side)/2, b.Min.Y+(b.Dy()-side)/2
	edge := min(side, profilePhotoEdge)
	out := scaleImage(img, square.Add(image.Pt(x0, y0)), edge, edge, catmullRom)
	var buf bytes.Buffer
	if err := jpeg.Encode(&buf, onWhite(out), &jpeg.Options{Quality: 85}); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

// handleSetOwnPhoto changes or removes our profile photo.
func handleSetOwnPhoto(conn net.Conn, raw json.RawMessage) {
	var data SetOwnPhotoData
	if err := json.Unmarshal(raw, &data); err != nil {
		return
	}
	go func() {
		var photo []byte
		if !data.Remove {
			var err error
			if photo, err = profilePhotoJPEG(data.Path); err != nil {
				sendEvent(conn, "error", map[string]string{"message": "Couldn't use that photo: " + err.Error()})
				return
			}
		}
		// An empty target is our own photo.
		pictureID, err := wa().SetGroupPhoto(context.Background(), types.EmptyJID, photo)
		if err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't change your profile photo: " + err.Error()})
			return
		}
		own := ownJIDString()
		if data.Remove {
			pictureID = ""
		}
		updateContactAvatarCache(own, "", pictureID)
		sendEvent(conn, "own_photo_changed", map[string]interface{}{"removed": data.Remove})
	}()
}

// handleGetDefaultTimer tells the UI the default disappearing message timer.
func handleGetDefaultTimer(conn net.Conn) {
	timer, err := wa().GetDefaultDisappearingTimer(context.Background())
	if err != nil {
		sendEvent(conn, "default_timer", map[string]interface{}{"error": err.Error()})
		return
	}
	sendEvent(conn, "default_timer", map[string]interface{}{"seconds": int(timer.Seconds())})
}

func handleSetDefaultTimer(conn net.Conn, raw json.RawMessage) {
	var data DefaultTimerData
	if err := json.Unmarshal(raw, &data); err != nil || data.Seconds < 0 {
		return
	}
	go func() {
		if err := wa().SetDefaultDisappearingTimer(context.Background(), time.Duration(data.Seconds)*time.Second); err != nil {
			sendEvent(conn, "error", map[string]string{"message": "Couldn't change the default message timer: " + err.Error()})
			return
		}
		sendEvent(conn, "default_timer_saved", map[string]interface{}{"seconds": data.Seconds})
	}()
}
