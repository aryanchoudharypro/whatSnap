package main

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"google.golang.org/protobuf/proto"
)

type PostVoiceStatusData struct {
	Path           string `json:"path"`
	Duration       int    `json:"duration"`
	Waveform       []int  `json:"waveform"`
	BackgroundArgb uint32 `json:"background_argb"`
}

// voiceStatusMessage builds a voice status the way the phone sends one: WA Web
// can't post them, so every field here is copied from a phone-posted voice
// status (posterStatusID and statusSourceType are phone-only).
func voiceStatusMessage(uploaded whatsmeow.UploadResponse, duration int, waveform []byte, background uint32) *waE2E.Message {
	posterID := make([]byte, 32)
	rand.Read(posterID)
	secret := make([]byte, 32)
	rand.Read(secret)
	msg := &waE2E.Message{
		AudioMessage: &waE2E.AudioMessage{
			URL:           proto.String(uploaded.URL),
			Mimetype:      proto.String("audio/ogg; codecs=opus"),
			FileSHA256:    uploaded.FileSHA256,
			FileLength:    proto.Uint64(uploaded.FileLength),
			Seconds:       proto.Uint32(uint32(duration)),
			PTT:           proto.Bool(true),
			MediaKey:      uploaded.MediaKey,
			FileEncSHA256: uploaded.FileEncSHA256,
			DirectPath:    proto.String(uploaded.DirectPath),
			ContextInfo: &waE2E.ContextInfo{
				ForwardingScore: proto.Uint32(0),
				FeatureEligibilities: &waE2E.ContextInfo_FeatureEligibilities{
					CannotBeRanked:       proto.Bool(false),
					CanBeReshared:        proto.Bool(false),
					CanReceiveMultiReact: proto.Bool(true),
				},
				StatusSourceType: waE2E.ContextInfo_AUDIO.Enum(),
				PosterStatusID:   proto.String(base64.StdEncoding.EncodeToString(posterID)),
			},
			Waveform:       waveform,
			BackgroundArgb: proto.Uint32(background),
		},
		MessageContextInfo: &waE2E.MessageContextInfo{MessageSecret: secret},
	}
	stampUploadedMedia(msg, uploaded)
	return msg
}

func (acc *Account) handlePostVoiceStatus(raw json.RawMessage) {
	var data PostVoiceStatusData
	if err := json.Unmarshal(raw, &data); err != nil || data.Path == "" {
		return
	}
	if data.BackgroundArgb == 0 {
		data.BackgroundArgb = 0xFF7ACBA5
	}
	go func() {
		fail := func(text string) {
			acc.broadcastEvent("error", map[string]string{"message": "Failed to post status: " + text})
		}
		if reason := acc.statusPostBlocker(context.Background()); reason != "" {
			acc.broadcastEvent("error", map[string]string{"message": reason})
			return
		}
		const mimetype = "audio/ogg; codecs=opus"
		id := acc.wa().GenerateMessageID()
		fileBytes, err := os.ReadFile(data.Path)
		if err != nil {
			fail(err.Error())
			return
		}
		mediaPath, err := acc.saveMediaBytes("outgoing", id, mimetype, fileBytes)
		if err != nil {
			fail(err.Error())
			return
		}
		if isInTempDir(data.Path) {
			os.Remove(data.Path)
		}
		uploaded, err := acc.wa().Upload(context.Background(), fileBytes, whatsmeow.MediaAudio)
		if err != nil {
			fail(err.Error())
			return
		}
		msg := voiceStatusMessage(uploaded, data.Duration, voiceWaveformBytes(data.Waveform), data.BackgroundArgb)
		recipients := acc.statusRecipients(context.Background())
		resp, err := acc.wa().SendMessage(context.Background(), types.StatusBroadcastJID, msg, whatsmeow.SendRequestExtra{
			ID: id, MediaHandle: uploaded.Handle, StatusRecipients: recipients, StatusSetting: acc.statusSetting(context.Background()),
		})
		if err != nil {
			fail(sendErrorText(err))
			return
		}
		acc.rememberStatusRecipients(resp.ID, recipients)

		ownJID := ""
		if acc.wa().Store.ID != nil {
			ownJID = acc.wa().Store.ID.ToNonAD().String()
		}
		text := fmt.Sprintf("[Voice Message] (%d:%02d)", data.Duration/60, data.Duration%60)
		rawBytes, _ := proto.Marshal(msg)
		acc.saveMessage(StoredMessage{
			ID: resp.ID, JID: "status@broadcast", SenderJID: ownJID,
			FromMe: true, Text: text, Timestamp: resp.Timestamp.Unix(), Raw: rawBytes, Status: "sent",
			MediaType: "voice", MediaMimetype: mimetype, MediaPath: mediaPath,
		})
		acc.broadcastEvent("status_update", map[string]interface{}{
			"id": resp.ID, "posterJid": ownJID, "fromMe": true, "text": text,
			"mediaType": "voice", "timestamp": resp.Timestamp.Unix(),
		})
	}()
}
