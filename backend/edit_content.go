package main

import (
	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// editContent is the edited message WhatsApp Web puts in an edit's
// protocolMessage (WAWebBuildEditedMessageProto): only the new caption for a
// photo, video or document, with the document's in a
// documentWithCaptionMessage, and the new text otherwise. Mentions go in the
// contextInfo as they do when sending.
func editContent(mediaType, text string, mentionJIDs []string) *waE2E.Message {
	var ctx *waE2E.ContextInfo
	if len(mentionJIDs) > 0 {
		ctx = &waE2E.ContextInfo{MentionedJID: mentionJIDs}
	}
	switch mediaType {
	case "image":
		return &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String(text), ContextInfo: ctx}}
	case "video":
		return &waE2E.Message{VideoMessage: &waE2E.VideoMessage{Caption: proto.String(text), ContextInfo: ctx}}
	case "document":
		return &waE2E.Message{DocumentWithCaptionMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{
			DocumentMessage: &waE2E.DocumentMessage{Caption: proto.String(text), ContextInfo: ctx},
		}}}
	}
	if ctx != nil {
		return &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String(text), ContextInfo: ctx}}
	}
	return &waE2E.Message{Conversation: proto.String(text)}
}
