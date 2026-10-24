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
		first_name: String,
		/// As typed, keeping a leading + (or 00) so the backend can tell an
		/// international number from one of your own country.
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
	/// Opens the file picker for one of the attach-menu options.
	AttachFiles(whatsnap::AttachKind),
	/// "Add more files" in the attachment dialog: same option as the batch.
	AddAttachment,
	PasteAttachment,
	OpenPollDialog,
	/// "Sticker" in the attach menu: pick a sticker to send.
	OpenStickerPicker,
	/// The context menu of a row in the Status tab.
	ShowStatusMenu(usize),
	/// The context menu of a row in the Starred Messages tab.
	ShowStarredMenu(usize),
	/// The context menu of a row in the Calls tab.
	ShowCallsMenu(usize),
	/// The Conversations filter dropdown changed.
	SetChatFilter(usize),
	/// A chat's context menu: add it to the favorites or take it off.
	SetFavorite {
		jid: String,
		favorite: bool,
	},
	/// A chat's context menu: which of our lists it's in.
	ChooseChatLists(String),
	/// Chat menu > "Manage Lists...": make, rename or delete lists, and pick
	/// the chats in each.
	ManageChatLists,
	/// Enter on one of contact info's groups in common.
	OpenCommonGroup(usize),
	/// Call > "Create Call Link..." and "Join Call Link...".
	CreateCallLink,
	AskCallLink,
	/// The call window's "Raise hand" and "React...".
	ToggleCallHand,
	CallReact,
	/// The call window's "Let in" (true) and "Don't let in" (false).
	AdmitCallPerson(bool),
	/// The call window's "Approve new participants".
	SetCallApproval(bool),
	/// "Event" in the attach menu: create an event in the chat.
	OpenEventDialog,
	/// `V` on a message: vote in it if it's a poll, answer it if it's an event.
	OpenPollVote(String),
	SendPoll {
		question: String,
		options: Vec<String>,
		multiple_answers: bool,
		/// Set for a quiz: the option that is the right answer.
		correct_answer: Option<String>,
	},
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
	/// A WhatsApp link to act on (a share, a chat, or a group invite).
	OpenWhatsAppLink(String),
	/// Show the selected group's invite link.
	ShowGroupInviteLink,
	/// Open the "Find channels" directory.
	FindChannels,
	/// Find channels, listing the ones like this channel.
	SimilarChannels {
		jid: String,
		name: String,
	},
	/// Ask for a new channel's details and create it.
	CreateChannel,
	/// Join a group from an invite link's code, once confirmed.
	JoinGroupLink(String),
	MarkRead {
		jid: String,
		message_ids: Vec<String>,
	},
	LoadMoreMessages,
	/// Reached the last loaded row of a window that stops short of the
	/// newest message (a search hit): page in the next newer messages.
	LoadNewerMessages,
	/// End in the message list: go to the chat's actual newest message, even
	/// from a window that doesn't hold it.
	JumpToLatestMessage,
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
	/// Alt+Shift+Y or Alt+Shift+R, pressed anywhere (see call_hotkeys.rs).
	CallHotkey(crate::call_hotkeys::CallHotkey),
	MuteCall,
	LeaveCall,
	/// Turns the local camera on or off - the button, and Ctrl+Shift+V.
	ToggleCamera,
	/// Starts or stops sharing the screen - the button, and Ctrl+Shift+S.
	ToggleScreenShare,
	/// The computer's sound couldn't be captured for a screen share - see
	/// `system_audio.rs`.
	SystemAudioFailed(String),
	/// `None` means "System Default" (matches Settings' device combos).
	SetCallInputDevice(Option<String>),
	SetCallOutputDevice(Option<String>),
	SetCallCameraDevice(Option<String>),
	/// The camera capture/encode thread couldn't open a camera at all (no
	/// camera present, access denied, etc) - see `call_video_send.rs`.
	CameraFailed(String),
	/// The same, for a screen share whose capture couldn't start.
	ScreenShareFailed(String),
	/// The shared screen started (`true`) or stopped coming out completely
	/// black - what a screen reader's screen curtain does to a capture.
	ScreenShareBlack(bool),
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
	/// Contact info's "Add contact" / "Edit contact".
	AddOrEditContact,
	OpenNewContactForm,
	DeleteContact,
	ShowChatPrivacy,
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
	/// Chat menu's Disappearing Messages: pick a timer for the open chat.
	ChooseDisappearingTimer {
		jid: String,
	},
	/// "Report..." on a message: report it to WhatsApp.
	ReportMessage {
		message_id: String,
	},
	/// "Report to group admins" on a message in a group.
	ReportToAdmins {
		message_id: String,
	},
	/// "Report..." on a chat: report the contact or group to WhatsApp.
	ReportChat {
		jid: String,
	},
	/// "Block" / "Unblock" on a chat with one person.
	ToggleBlockChat {
		jid: String,
	},
	/// Account > "Blocked contacts...".
	ShowBlockedContacts,
	/// Group Info's moderation switches, saved with the other settings.
	UpdateGroupModeration {
		no_frequently_forwarded: bool,
		allow_admin_reports: bool,
		group_history: bool,
		admins_add_only: bool,
		approve_members: bool,
	},
	/// One of Group Info's join request and community buttons, by name.
	GroupAction(&'static str),
	/// Chat menu > "New Community...".
	NewCommunity,
	/// Group Info's "Reported messages...": what members reported to admins.
	ShowReportedMessages,
	/// Group Info's "Set tag": our own member tag in the open group ("" clears).
	SetMemberLabel(String),
	/// Per-chat alert level for an archived chat, from the chat-list context
	/// menu. Stored locally in settings.json rather than pushed to WhatsApp -
	/// there's no protocol-level equivalent, unlike mute.
	SetArchivedAlert {
		jid: String,
		level: whatsnap::ArchivedAlert,
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
	/// Open a community group we're in, or join one we aren't.
	JoinCommunitySubgroup(u32),
	CommunitySubgroupSelectionChanged,
	OpenGroupInvite(String), // message id of a group-invite message
	/// Enter on a channel admin invite message (its id): accept or cancel it.
	OpenChannelAdminInvite(String),
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
	pub(crate) result: crate::error::Result<voice_recorder::RecordingSession>,
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
	/// "Block a contact..." in Blocked contacts.
	BlockContact,
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
	pub(crate) sender_jid: String,
	pub(crate) from_me: bool,
	pub(crate) label: String,
}

/// One call history record, as shown in the Calls nav view.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct CallLogEntry {
	pub(crate) id: String,
	pub(crate) jid: String,
	pub(crate) peer_name: String,
	pub(crate) from_me: bool,
	pub(crate) text: String,
	pub(crate) timestamp: i64,
	pub(crate) is_video: bool,
	pub(crate) is_group: bool,
	/// Backend's name for how the call ended ("missed", "connected", ...).
	pub(crate) result: String,
	/// Seconds the call was connected for; 0 when it never connected.
	pub(crate) duration: i64,
}

/// A person the Calls tab can dial, from the "call_targets" backend event.
#[derive(Clone, Debug)]
pub(crate) struct CallTarget {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) phone: String,
}

/// One posted Status/Stories update, as shown in the Status nav view.
#[derive(Clone)]
pub(crate) struct StatusItem {
	pub(crate) id: String,
	pub(crate) text: String,
	pub(crate) media_type: String,
	pub(crate) timestamp: i64,
	pub(crate) viewed: bool,
	/// How many have seen it (our own updates).
	pub(crate) viewers: i64,
	/// Our reaction to it, if any.
	pub(crate) my_reaction: String,
}

/// All of one contact's (or our own) live status updates, in post order.
#[derive(Clone)]
pub(crate) struct StatusGroup {
	pub(crate) poster_jid: String,
	pub(crate) poster_name: String,
	pub(crate) all_viewed: bool,
	/// Ours.
	pub(crate) from_me: bool,
	/// Their status updates are muted.
	pub(crate) muted: bool,
	pub(crate) items: Vec<StatusItem>,
}

/// One other person in a group call, as sent by the backend's
/// `call_participants` event. `state` is WhatsApp's roster state, e.g.
/// Video details of the active call: what we're sending (camera or screen)
/// and what's known about the other side, which drives what's announced.
/// Incoming video is never shown.
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
	/// Whether we're sharing the screen. Over the call's one video stream
	/// this and `local_camera_on` are never both true; a dual-stream share
	/// (`screen_send`) leaves the camera as it was.
	pub(crate) local_screen_on: bool,
	/// The backend's port for the screen's own stream, there when the
	/// experimental dual-stream setting was on as the call connected.
	pub(crate) screen_port: Option<u16>,
	/// The screen session of a dual-stream share, sending to `screen_port`
	/// while the camera keeps `active_call_video_send`. None for a share over
	/// the camera's stream.
	pub(crate) screen_send: Option<crate::call_video_send::CallVideoSendState>,
	/// The camera was on when the screen share started, so it comes back on
	/// when the share stops.
	pub(crate) camera_before_share: bool,
	/// A video call: offered, placed or joined as one, or turned into one by
	/// a camera coming on. Screen sharing is only offered on these; a voice
	/// call has no video to share the screen over.
	pub(crate) video_call: bool,
	/// Set by `PlaceVideoCall` - once this outgoing call connects, the
	/// camera turns on automatically instead of waiting for an explicit
	/// toggle. Never set for an incoming call (those always answer
	/// camera-off, see [[video_calls_work_status]] Phase 1).
	pub(crate) autostart_camera: bool,
	/// Whether our hand is raised.
	pub(crate) hand_raised: bool,
	/// A roster has arrived, so later ones are announced as changes. A call
	/// link's first roster is just us, which leaves `call_participants` empty.
	pub(crate) roster_seen: bool,
	/// We're an admin of this call link, so we can let people in.
	pub(crate) link_admin: bool,
	/// The call link asks an admin before letting anyone in.
	pub(crate) approval_required: bool,
	/// People in the call link's waiting room.
	pub(crate) waiting: Vec<CallParticipant>,
}

/// "connected", "outgoing", "ringing", "invited" or "rejected".
#[derive(Clone)]
pub(crate) struct CallParticipant {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) state: String,
}
