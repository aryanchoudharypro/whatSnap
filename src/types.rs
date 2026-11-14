use crate::audio::voice_recorder;

pub const META_AI_JID: &str = "867051314767696@bot";

/// The "WhatsApp" news account (the PSA service chat). Its JID is the number
/// 0, which isn't a phone number, so it gets a name rather than "+0".
pub const WHATSAPP_SERVICE_JID: &str = "0@s.whatsapp.net";

/// One person in the open group's Group Info member list.
#[derive(Clone, Default)]
pub struct GroupMember {
	/// The address the group lists them under (may be a @lid).
	pub jid: String,
	/// Their phone-number JID, empty when the group hides it.
	pub phone_jid: String,
	/// Plain name, no role or tag, for menus and confirmations.
	pub name: String,
	/// Formatted international number, empty when hidden.
	pub phone: String,
	pub admin: bool,
	pub super_admin: bool,
	pub me: bool,
	pub saved: bool,
	/// How many recent messages "Send message history" would send them; 0
	/// when it doesn't apply.
	pub history_count: u32,
}

impl GroupMember {
	/// The JID to open a chat or contact info with: the phone JID when known,
	/// since chats are keyed by it.
	pub fn chat_jid(&self) -> &str {
		if self.phone_jid.is_empty() { &self.jid } else { &self.phone_jid }
	}
}

/// The items of a group member's context menu in Group Info (WA Web's
/// participant menu).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemberAction {
	Message,
	Info,
	AddContact,
	MakeAdmin,
	DismissAdmin,
	Remove,
	VerifyCode,
	SendHistory,
}

#[derive(Clone)]
pub enum UiCommand {
	RequestPairing(String),
	SendMessage(String),
	ConversationSelectionChanged(usize),
	ProcessWebSocketMessage(String),
	/// A line from the backend of a linked account.
	BackendEvent {
		account: std::sync::Arc<str>,
		payload: String,
	},
	/// Account, Accounts... (with the account to put the focus on).
	OpenAccounts(Option<String>),
	/// The login window was closed while another account is linked.
	LeaveLogin,
	/// A removed account's backend has stopped, so its files can go.
	AccountStopped(String),
	/// A removed account's logout took too long: stop it anyway.
	FinishAccountRemoval(String),
	ExitApp,
	RestartApp,
	NewContact {
		name: String,
		first_name: String,
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
	/// The chat list's search box settled: search message text too.
	SearchAllMessages,
	/// The chat list reached its last message result: ask for the next page.
	MoreMessageResults,
	/// A channel's details from the chat list, without opening the channel.
	ShowChannelInfo(String),
	OpenConversation(usize),
	PromptLockedChats,
	/// The Windows Hello prompt for locked chats finished: `outcome` is
	/// the verification outcome.
	ChatLockHello {
		outcome: crate::platform::windows_hello::VerifyOutcome,
	},
	CloseLockedChats,
	SearchInConversation(String),
	ToggleSearchPanel,
	CloseSearch,
	JumpToSearchResult {
		message_id: String,
		message_text: String,
	},
	OpenChatAtMessage {
		jid: String,
		message_id: String,
	},
	StartRecording,
	TogglePauseRecording,
	ToggleVoicePreview,
	StopRecording,
	DiscardRecording,
	ForceSync,
	SendVoiceMessage,
	DiscardVoiceMessage,
	AttachFiles(whatsnap::AttachKind),
	AddAttachment,
	PasteAttachment,
	OpenPollDialog,
	OpenEmojiPanel {
		stickers: bool,
	},
	ShowStatusMenu(usize),
	ShowStarredMenu(usize),
	UnstarStarredMessage(usize),
	CopyStarredMessage(usize),
	ShowCallsMenu(usize),
	DeleteCall(usize),
	SetChatFilter(usize),
	SetFavorite {
		jid: String,
		favorite: bool,
	},
	ChooseChatLists(String),
	ManageChatLists,
	OpenCommonGroup(usize),
	CreateCallLink,
	AskCallLink,
	ToggleCallHand,
	CallReact,
	/// The call window's "Let in" (true) and "Don't let in" (false).
	AdmitCallPerson(bool),
	MuteCallPerson,
	RemoveCallPerson,
	CallPersonSelected,
	SetCallApproval(bool),
	DismissIncomingCall,
	RestoreIncomingCall,
	RedialCall,
	CloseCallWindow,
	OpenEventDialog,
	OpenScheduleDialog,
	ShowScheduledMessages,
	OpenPollVote(String),
	OpenMessageOptions(String),
	OpenContactCard {
		message_id: String,
		intent: String,
	},
	OpenOrderDetails(String),
	OpenCatalog(Option<String>),
	SendPoll {
		question: String,
		options: Vec<String>,
		multiple_answers: bool,
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
	PostCustomStatus {
		text: String,
		background: u32,
		text_color: u32,
		font: i32,
	},
	PostMediaStatus,
	OpenStatusPrivacy,
	OpenStatusComposer,
	GetMessageInfo {
		message_id: String,
	},
	RetryMessage {
		message_id: String,
	},
	RetrySelectedMessage,
	CancelSelectedUpload,
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
	SetPresence(bool),
	WindowHidden,
	ActivateWindow,
	ToggleTrayWindow,
	OpenWhatsAppLink(String),
	/// Text typed into a message notification's reply box.
	NotificationReply {
		jid: String,
		account: Option<String>,
		text: String,
	},
	ShareFiles(Vec<String>),
	ShareMenuResult {
		ok: bool,
		text: String,
	},
	ShowGroupInviteLink,
	FindChannels,
	SimilarChannels {
		jid: String,
		name: String,
	},
	CreateChannel,
	JoinGroupLink(String),
	MarkRead {
		jid: String,
		message_ids: Vec<String>,
	},
	LoadMoreMessages,
	LoadNewerMessages,
	JumpToLatestMessage,
	JumpToOldestMessage,
	PlayVoiceMessage(String),
	RefreshMessages,
	DownloadAttachment(String),
	ChangeAudioSpeed,
	SeekAudio(f32),
	StopVoiceMessage,
	IncreaseAudioSpeed,
	DecreaseAudioSpeed,
	PlaceCall,
	PlaceVideoCall,
	CallContact {
		jid: String,
		video: bool,
	},
	JoinGroupCall(Option<String>),
	AnswerCall,
	RejectCall,
	CallHotkey(crate::calls::hotkeys::CallHotkey),
	MuteCall,
	LeaveCall,
	ToggleCamera,
	ToggleScreenShare,
	ToggleCallDeviceSettings,
	SystemAudioFailed(String),
	SetCallInputDevice(Option<String>),
	SetCallOutputDevice(Option<String>),
	SetCallCameraDevice(Option<String>),
	CameraFailed(String),
	ScreenShareFailed(String),
	/// The shared screen started (`true`) or stopped coming out completely
	/// black - what a screen reader's screen curtain does to a capture.
	ScreenShareBlack(bool),
	AudioDeviceChanged,
	UpdateGroupSettings {
		name: String,
		topic: String,
		announce: bool,
		locked: bool,
	},
	SeekAudioAbsolute(i32),
	CloseConversation,
	ReactToMessage {
		message_id: String,
		sender_jid: String,
		emoji: String,
	},
	ToggleAudioSpeed,
	ShowConversationData,
	ShowContactInfo(String),
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
		seconds: u64,
	},
	ChooseMuteDuration {
		jid: String,
	},
	ChooseDisappearingTimer {
		jid: String,
	},
	ReportMessage {
		message_id: String,
	},
	ReportToAdmins {
		message_id: String,
	},
	/// Ask for a date and show the open chat around it.
	GoToDate,
	ReportChat {
		jid: String,
	},
	ToggleBlockChat {
		jid: String,
	},
	ShowBlockedContacts,
	UpdateGroupModeration {
		no_frequently_forwarded: bool,
		allow_admin_reports: bool,
		group_history: bool,
		admins_add_only: bool,
		approve_members: bool,
	},
	GroupAction(&'static str),
	NewCommunity,
	ShowReportedMessages,
	SetMemberLabel(String),
	SetArchivedAlert {
		jid: String,
		level: whatsnap::ArchivedAlert,
	},
	/// A chat's notification choices on this computer, from its
	/// "Notifications" menu. Kept in settings.json, never sent to WhatsApp.
	SetChatNotify {
		jid: String,
		notify: whatsnap::ChatNotify,
	},
	ShowChatNotify {
		jid: String,
	},
	ToggleReadChat {
		jid: String,
		read: bool,
	},
	DeleteChat {
		jid: String,
	},
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
	ShowMemberChanges,
	GroupMemberAction {
		jid: String,
		action: MemberAction,
	},
	JoinCommunitySubgroup(u32),
	CommunitySubgroupSelectionChanged,
	OpenGroupInvite(String),
	OpenChannelAdminInvite(String),
	JoinGroupInvite(String),
	DownloadMediaMessage(String),
	OpenMediaMessage(String),
	SaveMediaMessageAs(String),
	OpenChatByPhone(String),
	OpenChatByJid(String),
	ToggleSelectionMode,
	/// One of the select bar's buttons.
	SelectionAction(crate::ui_commands::selection::BarAction),
	/// Delete these messages of the open chat, asking how first (the
	/// Delete key or menu item on one row).
	AskDeleteMessages(Vec<String>),
	VoiceRecordingReady,
}

pub(crate) struct PendingVoiceRecording {
	pub(crate) generation: u64,
	pub(crate) wav_path: String,
	pub(crate) is_resume: bool,
	pub(crate) result: crate::error::Result<voice_recorder::RecordingSession>,
}

#[derive(Clone)]
pub struct Contact {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) phone: String,
	pub(crate) saved: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContactPickerMode {
	OpenChat,
	AddCallParticipant,
	BlockContact,
}

#[derive(Clone, Copy)]
pub(crate) enum NotificationSound {
	MessageCurrent,
	MessageBackground,
	MessageSent,
	VoiceMessageSent,
	VoiceSequenceNext,
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

#[derive(Clone, Debug)]
pub(crate) struct StarredEntry {
	pub(crate) jid: String,
	pub(crate) message_id: String,
	pub(crate) sender_jid: String,
	pub(crate) from_me: bool,
	pub(crate) chat_name: String,
	pub(crate) sender_name: String,
	pub(crate) text: String,
	pub(crate) media_type: String,
	pub(crate) timestamp: i64,
	pub(crate) label: String,
}

/// What else the chat list's search finds a chat by, besides its name: push
/// and business names, and its phone number's digits (see `whatsnap::search`).
#[derive(Clone, Default)]
pub(crate) struct ChatSearchKeys {
	pub(crate) names: Vec<String>,
	pub(crate) phone: String,
}

/// One message found by the chat list's search, under its Messages heading.
#[derive(Clone)]
pub(crate) struct MessageHit {
	pub(crate) jid: String,
	pub(crate) message_id: String,
	pub(crate) chat_name: String,
	pub(crate) sender: String,
	pub(crate) body: String,
	pub(crate) date: String,
}

impl MessageHit {
	/// The row's text, which is also what a screen reader says for it.
	pub(crate) fn label(&self) -> String {
		let mut label = format!("{} | {}: {}", self.chat_name, self.sender, self.body);
		if !self.date.is_empty() {
			label.push_str(" | ");
			label.push_str(&self.date);
		}
		label
	}
}

/// The chat list search's message results: WA Web searches message text too,
/// once the query is long enough, newest first and 30 at a time.
#[derive(Default)]
pub(crate) struct MessageSearch {
	/// The query the results are for, as typed.
	pub(crate) query: String,
	pub(crate) hits: Vec<MessageHit>,
	/// A page was asked for and hasn't come back yet.
	pub(crate) pending: bool,
	pub(crate) has_more: bool,
}

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
	pub(crate) result: String,
	/// Seconds the call was connected for; 0 when it never connected.
	pub(crate) duration: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct CallTarget {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) phone: String,
}

#[derive(Clone)]
pub(crate) struct StatusItem {
	pub(crate) id: String,
	pub(crate) text: String,
	pub(crate) media_type: String,
	pub(crate) timestamp: i64,
	pub(crate) viewed: bool,
	pub(crate) viewers: i64,
	pub(crate) my_reaction: String,
	pub(crate) background: u32,
	pub(crate) text_color: u32,
	pub(crate) font: i32,
}

#[derive(Clone)]
pub(crate) struct StatusGroup {
	pub(crate) poster_jid: String,
	pub(crate) poster_name: String,
	pub(crate) all_viewed: bool,
	pub(crate) from_me: bool,
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
	pub(crate) is_video: bool,
	pub(crate) peer_name: Option<String>,
	/// The other person's phone number (digits) on a 1:1 call we placed,
	/// shown under their name while it rings.
	pub(crate) peer_phone: Option<String>,
	pub(crate) peer_camera_on: Option<bool>,
	pub(crate) local_camera_on: bool,
	/// Whether we're sharing the screen. Over the call's one video stream
	/// this and `local_camera_on` are never both true; a dual-stream share
	/// (`screen_send`) leaves the camera as it was.
	pub(crate) local_screen_on: bool,
	pub(crate) screen_port: Option<u16>,
	pub(crate) screen_send: Option<crate::calls::video_send::CallVideoSendState>,
	pub(crate) camera_before_share: bool,
	pub(crate) video_call: bool,
	/// Set by `PlaceVideoCall` - once this outgoing call connects, the
	/// camera turns on automatically instead of waiting for an explicit
	/// toggle. Never set for an incoming call (those always answer
	/// camera-off, see [[video_calls_work_status]] Phase 1).
	pub(crate) autostart_camera: bool,
	pub(crate) hand_raised: bool,
	pub(crate) roster_seen: bool,
	pub(crate) link_admin: bool,
	pub(crate) can_remove: bool,
	pub(crate) approval_required: bool,
	pub(crate) waiting: Vec<CallParticipant>,
}

#[derive(Clone)]
pub(crate) struct CallParticipant {
	pub(crate) jid: String,
	pub(crate) name: String,
	pub(crate) state: String,
}
