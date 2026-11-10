package main

import "testing"

func TestEditContentCarriesOnlyTheNewCaptionForMedia(t *testing.T) {
	if got := editContent("image", "new", nil).GetImageMessage(); got.GetCaption() != "new" || got.GetContextInfo() != nil || got.GetMediaKey() != nil {
		t.Errorf("image: %v", got)
	}
	if got := editContent("video", "new", []string{"1@s.whatsapp.net"}).GetVideoMessage(); got.GetCaption() != "new" ||
		len(got.GetContextInfo().GetMentionedJID()) != 1 {
		t.Errorf("video: %v", got)
	}
	doc := editContent("document", "new", nil)
	if doc.GetDocumentMessage() != nil || doc.GetDocumentWithCaptionMessage().GetMessage().GetDocumentMessage().GetCaption() != "new" {
		t.Errorf("document: %v", doc)
	}
	if got := editContent("", "hi", nil); got.GetConversation() != "hi" {
		t.Errorf("text: %v", got)
	}
	if got := editContent("", "hi @1", []string{"1@s.whatsapp.net"}).GetExtendedTextMessage(); got.GetText() != "hi @1" ||
		len(got.GetContextInfo().GetMentionedJID()) != 1 {
		t.Errorf("text with mention: %v", got)
	}
}
