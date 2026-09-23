use whantv;

use crate::voice_recorder;

/// JID of WhatsApp's own Meta AI bot. Sending to a `@bot`-server JID makes
/// whatsmeow's `SendMessage` automatically wrap the message with the bot
/// metadata WhatsApp expects - see `open_chat_for_jid` callers.
pub const META_AI_JID: &str = "867051314767696@bot";

#[derive(Clone)]
pub enum UiCommand {
	RequestPairing(String),
	SendMessage(String),
	ConversationSelectionChanged(usize),
	ProcessWebSocketMessage(String),
	ExitApp,
	NewContact {
		name: String,
		phone: String,
		sync: bool,
	},
	PrepareNewGroup,
	PrepareAddMembers,
	AddGroupMembers(Vec<String>),
	PrepareAttachContact,
	SendContacts(Vec<String>),
	PrepareContactPicker(ContactPickerMode),
	FilterContactPicker(String),
	SelectContactPickerResult(usize),
	CreateGroup {
		name: String,
		participants: Vec<String>,
	},
	RequestQr,
	CancelQr,
	CancelPairing,
	NavSelected(usize),
	FilterConversations(String),
	OpenConversation(usize),
	PromptLockedChats,
	CloseLockedChats,
	SearchInConversation(String),
	ToggleSearchPanel,
	CloseSearch,
	JumpToSearchResult {
		message_id: String,
		message_text: String,
	},
	StartRecording,
	TogglePauseRecording,
	StopRecording,
	DiscardRecording,
	ForceSync,
	SendVoiceMessage,
	DiscardVoiceMessage,
	AddAttachment,
	PasteAttachment,
	SendAttachment {
		caption: String,
	},
	DiscardAttachment,
	DownloadMedia,
	OpenMedia,
	SaveMediaAs,
	CancelEdit,
	RemoveQuote,
	/// Puts the composer into edit mode for an existing message: fills
	/// `message_field` with its current text instead of opening a separate
	/// dialog. Saving (Enter, or the Send button) while in this mode sends
	/// `EditMessage` instead of a new `SendMessage`.
	BeginEditMessage {
		message_id: String,
		text: String,
	},
	EditMessage {
		message_id: String,
		new_text: String,
	},
	DeleteMessage {
		message_id: String,
		sender_jid: String,
		for_everyone: bool,
	},
	StarMessage {
		message_id: String,
		sender_jid: String,
		from_me: bool,
		starred: bool,
	},
	ExportChat {
		jid: String,
		dest_path: String,
	},
	ExportChatText {
		jid: String,
		dest_path: String,
	},
	PostStatus {
		text: String,
	},
	PostMediaStatus,
	GetMessageInfo {
		message_id: String,
	},
	RetryMessage {
		message_id: String,
	},
	RetrySelectedMessage,
	RequestMessageResend {
		message_id: String,
	},
	CopyAlbumToClipboard {
		message_ids: Vec<String>,
	},
	SendTyping {
		jid: String,
		state: String,
		media: String,
	},
	SubscribePresence(String),
	/// The main window gained (true) or lost (false) OS focus.
	SetPresence(bool),
	/// The main window was hidden to the tray - go unavailable right away
	/// rather than after the focus-loss grace period.
	WindowHidden,
	ActivateWindow,
	MarkRead {
		jid: String,
		message_ids: Vec<String>,
	},
	LoadMoreMessages,
	PlayVoiceMessage(String),
	RefreshMessages,
	DownloadAttachment(String), // message ID
	ChangeAudioSpeed,
	SeekAudio(f32),
	PlaceCall,
	/// Places a call that offers video from the start (`CallOptions{Video:
	/// true}` on the backend) - as opposed to `ToggleCamera` turning the
	/// camera on mid-call, which upgrades a plain voice call instead.
	PlaceVideoCall,
	AnswerCall,
	RejectCall,
	MuteCall,
	LeaveCall,
	/// Turns the local camera on or off - the button, and Ctrl+Shift+V.
	ToggleCamera,
	/// `None` means "System Default" (matches Settings' device combos).
	SetCallInputDevice(Option<String>),
	SetCallOutputDevice(Option<String>),
	SetCallCameraDevice(Option<String>),
	/// The camera capture/encode thread couldn't open a camera at all (no
	/// camera present, access denied, etc) - see `call_video_send.rs`.
	CameraFailed(String),
	/// Windows reported the system default audio input or output device
	/// changed (headphones plugged/unplugged, a new default picked in
	/// Windows Settings) - see `audio_device_watch.rs`. Reopens any live
	/// playback/call audio on the newly-current device instead of waiting
	/// for the next unrelated action to notice.
	AudioDeviceChanged,
	UpdateGroupSettings {
		name: String,
		topic: String,
		announce: bool,
		locked: bool,
	},
	SeekAudioAbsolute(i32), // slider value 0..1000
	CloseConversation,
	ReactToMessage {
		message_id: String,
		sender_jid: String,
		emoji: String,
	},
	ToggleAudioSpeed,
	ShowConversationData,
	ForwardMessage {
		message_id: String,
		to_jids: Vec<String>,
	},
	TogglePinChat {
		jid: String,
		pin: bool,
	},
	ToggleArchiveChat {
		jid: String,
		archive: bool,
	},
	ToggleLockChat {
		jid: String,
		lock: bool,
	},
	ToggleMuteChat {
		jid: String,
		mute: bool,
	},
	/// Per-chat alert level for an archived chat, from the chat-list context
	/// menu. Stored locally in settings.json rather than pushed to WhatsApp -
	/// there's no protocol-level equivalent, unlike mute.
	SetArchivedAlert {
		jid: String,
		level: whantv::ArchivedAlert,
	},
	ToggleReadChat {
		jid: String,
		read: bool,
	},
	DeleteChat {
		jid: String,
	},
	/// "Delete chat and exit group": leaving then deleting as two
	/// independent commands raced the group's own "You left" system message
	/// against the delete, sometimes resurrecting the chat right after this
	/// removed it - see `handleLeaveAndDeleteGroup` on the backend, which
	/// runs both in order under one guard instead.
	DeleteChatAndLeaveGroup {
		jid: String,
	},
	ClearChat {
		jid: String,
	},
	LeaveGroup {
		jid: String,
	},
	FilterGroupMembers(String),
	GroupMemberSelectionChanged,
	ToggleGroupMemberAdmin(u32),
	RemoveGroupMember(u32),
	JoinCommunitySubgroup(u32),
	OpenGroupInvite(String), // message id of a group-invite message
	JoinGroupInvite(String), // message id, confirmed via the invite-details dialog
	DownloadMediaMessage(String),
	OpenMediaMessage(String),
	SaveMediaMessageAs(String),
	OpenChatByPhone(String), // digits-only phone number, no '@s.whatsapp.net' suffix
	OpenChatByJid(String),   // full jid, used to jump to a group message sender's 1:1 chat
	ToggleSelectionMode,
	DeleteSelectedMessages {
		for_everyone: bool,
	},
	/// Posted by `RecordingSession::start_async`'s supervisor thread once a
	/// voice-recording device open (initial start, or a pause's resume)
	/// finishes - the outcome itself is collected from
	/// `AppState::voice_recording_pending` rather than carried on this
	/// variant, since `RecordingSession` isn't `Clone`.
	VoiceRecordingReady,
}

/// Outcome of an async `RecordingSession::start_async` call, handed back to
/// the UI thread through `AppState::voice_recording_pending`. See
/// `AppState::voice_recording_generation` for how a stale result (the
/// recording was discarded/sent before the device finished opening) is
/// detected.
pub(crate) struct PendingVoiceRecording {
	pub(crate) generation: u64,
	pub(crate) wav_path: String,
	pub(crate) is_resume: bool,
	pub(crate) result: anyhow::Result<voice_recorder::RecordingSession>,
}

/// A synced WhatsApp contact, as returned by the backend's `get_contacts`.
#[derive(Clone)]
pub struct Contact {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) phone: String,
}

/// What selecting a contact from the shared contact-picker dialog
/// (`new_conv_dialog`, reused for both remaining "pick someone" flows)
/// should do. Adding group members used to be a third mode here, but it
/// wanted checkbox multi-select and an explicit Add button rather than
/// "search, Enter, done" - see `add_members.rs` and `AddMembersState`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContactPickerMode {
	OpenChat,
	AddCallParticipant,
}

#[derive(Clone, Copy)]
pub(crate) enum NotificationSound {
	MessageCurrent,
	MessageBackground,
	MessageSent,
	VoiceMessageSent,
	/// WhatsApp's own tone as one voice message rolls on into the next.
	VoiceSequenceNext,
	/// WhatsApp's own tone when a run of voice messages has played out.
	VoiceSequenceEnd,
}

impl NotificationSound {
	pub(crate) const fn ogg(self) -> &'static [u8] {
		match self {
			Self::MessageCurrent => include_bytes!("../sounds/message_current.ogg"),
			Self::MessageBackground => include_bytes!("../sounds/message_background.ogg"),
			Self::MessageSent => include_bytes!("../sounds/message_sent.ogg"),
			Self::VoiceMessageSent => include_bytes!("../sounds/voicemsg_send.ogg"),
			Self::VoiceSequenceNext => include_bytes!("../sounds/voice_seq_next.ogg"),
			Self::VoiceSequenceEnd => include_bytes!("../sounds/voice_seq_end.ogg"),
		}
	}
}

/// One row of the flat, cross-chat "Starred Messages" nav view: enough to
/// show a label and to jump into the right chat at the right message when
/// selected (reuses the same search_target_id jump mechanism as in-chat
/// search results).
#[derive(Clone)]
pub(crate) struct StarredEntry {
	pub(crate) jid: String,
	pub(crate) message_id: String,
	pub(crate) label: String,
}

/// One posted Status/Stories update, as shown in the Status nav view.
#[derive(Clone)]
pub(crate) struct StatusItem {
	pub(crate) id: String,
	pub(crate) text: String,
	pub(crate) media_type: String,
	pub(crate) timestamp: i64,
	pub(crate) viewed: bool,
}

/// All of one contact's (or our own) live status updates, in post order.
#[derive(Clone)]
pub(crate) struct StatusGroup {
	pub(crate) poster_jid: String,
	pub(crate) poster_name: String,
	pub(crate) all_viewed: bool,
	pub(crate) items: Vec<StatusItem>,
}

/// One other person in a group call, as sent by the backend's
/// `call_participants` event. `state` is WhatsApp's roster state, e.g.
/// Video details of the active call. We never send or show video; this only
/// drives what's announced.
#[derive(Default)]
pub(crate) struct CallVideo {
	/// The call was offered as a video call (the backend answers those camera-off).
	pub(crate) is_video: bool,
	/// The other person's name on a 1:1 call, None on a group call.
	pub(crate) peer_name: Option<String>,
	/// Their last known camera state, None when unknown.
	pub(crate) peer_camera_on: Option<bool>,
	/// Whether OUR camera is currently on - drives the toggle button/menu
	/// label and whether `active_call_video_send` should exist.
	pub(crate) local_camera_on: bool,
	/// Set by `PlaceVideoCall` - once this outgoing call connects, the
	/// camera turns on automatically instead of waiting for an explicit
	/// toggle. Never set for an incoming call (those always answer
	/// camera-off, see [[video_calls_work_status]] Phase 1).
	pub(crate) autostart_camera: bool,
}

/// "connected", "outgoing", "ringing", "invited" or "rejected".
#[derive(Clone)]
pub(crate) struct CallParticipant {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) state: String,
}
