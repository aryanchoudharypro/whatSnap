#![windows_subsystem = "windows"]

mod audio_devices;
mod autostart;
mod backend_process;
mod call_audio;
mod countries;
mod paths;
mod settings;
mod single_instance;
mod ui;
mod ui_wake;
mod voice_recorder;
mod win_job;

use std::{
	cell::RefCell,
	io::{BufRead, BufReader, Write},
	rc::Rc,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	thread,
	time::Duration,
};

use interprocess::{
	TryClone,
	local_socket::{GenericNamespaced, ToNsName, prelude::*},
};
use wxdragon::prelude::*;

use crate::{
	ui::window::{WindowParts, bind_input_handlers, build_main_window},
	ui_wake::{ID_UI_WAKE, UiCommandSender, UiWaker},
};

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
	},
	PrepareNewGroup,
	PrepareAddMembers,
	AddGroupMembers(Vec<String>),
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
	SetPresence(bool),
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
	AnswerCall,
	RejectCall,
	MuteCall,
	LeaveCall,
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
struct PendingVoiceRecording {
	generation: u64,
	wav_path: String,
	is_resume: bool,
	result: anyhow::Result<voice_recorder::RecordingSession>,
}

use std::fs::File;

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use rodio_wsola::WsolaSourceExt;
use whantv::ChatMessage;

/// A synced WhatsApp contact, as returned by the backend's `get_contacts`.
#[derive(Clone)]
pub struct Contact {
	jid: String,
	name: String,
	phone: String,
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
pub enum NotificationSound {
	MessageCurrent,
	MessageBackground,
	MessageSent,
	VoiceMessageSent,
}

impl NotificationSound {
	const fn ogg(self) -> &'static [u8] {
		match self {
			Self::MessageCurrent => include_bytes!("../sounds/message_current.ogg"),
			Self::MessageBackground => include_bytes!("../sounds/message_background.ogg"),
			Self::MessageSent => include_bytes!("../sounds/message_sent.ogg"),
			Self::VoiceMessageSent => include_bytes!("../sounds/voicemsg_send.ogg"),
		}
	}
}

use std::{
	sync::atomic::AtomicU64,
	time::{SystemTime, UNIX_EPOCH},
};

static LAST_SOUND_TIME: AtomicU64 = AtomicU64::new(0);

/// Opens a playback stream on the user's preferred output device (Settings ->
/// Input & Output), falling back to the system default if none is set or the
/// named device is no longer present.
fn open_output_stream(preferred_name: Option<&str>) -> Result<MixerDeviceSink, rodio::DeviceSinkError> {
	match audio_devices::resolve_output_device(preferred_name) {
		Some(device) => DeviceSinkBuilder::from_device(device).and_then(|b| b.open_stream()),
		None => DeviceSinkBuilder::open_default_sink(),
	}
}

/// Makes sure `s.audio_stream` is a live stream on whatever device the
/// user's output preference currently resolves to, (re)opening it if it's
/// missing or if that resolution has changed since it was last opened.
///
/// A cpal/WASAPI stream is bound to one concrete endpoint for its whole
/// lifetime - it doesn't follow the OS's "default device" pointer moving,
/// so a stream opened before unplugging headphones stays silently pointed
/// at that now-gone endpoint even after the headphones (or the fallback
/// speakers) come back. Without this re-check, every sound the app plays
/// through `audio_stream` (call-ended tone, voice message playback) would
/// stay dead until the app was restarted.
fn ensure_audio_stream(s: &mut AppState) {
	let current_name = audio_devices::resolve_output_device_name(s.settings.output_device_name.as_deref());
	if s.audio_stream.is_some() && s.audio_stream_device_name == current_name {
		return;
	}
	match open_output_stream(s.settings.output_device_name.as_deref()) {
		Ok(stream) => {
			s.audio_stream = Some(stream);
			s.audio_stream_device_name = current_name;
		}
		Err(e) => {
			eprintln!("[AUDIO] Failed to (re)open output stream: {:?}", e);
			s.audio_stream = None;
			s.audio_stream_device_name = None;
		}
	}
}

fn play_notification_sound(sound: NotificationSound, sounds_enabled: bool) {
	if !sounds_enabled {
		return;
	}
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_SOUND_TIME.swap(now, Ordering::Relaxed);
	if now - last < 500 {
		return;
	}
	std::thread::spawn(move || {
		let Ok(stream) = DeviceSinkBuilder::open_default_sink() else {
			return;
		};
		let sink = Player::connect_new(&stream.mixer());
		let cursor = std::io::Cursor::new(sound.ogg());
		if let Ok(decoder) = rodio::Decoder::new(cursor) {
			sink.append(decoder);
			sink.play();
			// Block this worker thread until playback finishes, so `stream` and
			// `sink` (and the device they hold open) stay alive for the duration.
			sink.sleep_until_end();
		}
	});
}

fn play_call_fail_sound(state: &mut AppState) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(&stream_handle.mixer());
		let cursor = std::io::Cursor::new(include_bytes!("../sounds/whatsapp_windows_hangup_01_705kbps.ogg"));
		match rodio::Decoder::new(cursor) {
			Ok(decoder) => {
				sink.append(decoder);
				sink.play();
				state.call_ringing_sink = Some(sink);
			}
			Err(e) => eprintln!("[AUDIO] Error decoding fail sound: {:?}", e),
		}
	} else {
		eprintln!("[AUDIO] No audio stream available");
	}
}

fn start_call_ringing_sound(state: &mut AppState, sound_bytes: &'static [u8]) {
	ensure_audio_stream(state);
	if let Some(ref stream_handle) = state.audio_stream {
		let sink = Player::connect_new(&stream_handle.mixer());
		let cursor = std::io::Cursor::new(sound_bytes);
		match rodio::Decoder::new(cursor) {
			Ok(decoder) => {
				use rodio::Source;
				// Fully decode into a SamplesBuffer before looping: rodio's
				// symphonia-backed Decoder reports current_span_len() == 0 until
				// its internal buffer is first populated by a next() call, and
				// repeat_infinite()'s underlying buffered() checks that length
				// immediately at construction time - seeing 0, it treats the
				// decoder as already exhausted and Repeat plays silence forever.
				// SamplesBuffer reports its real length from the start, so
				// wrapping that instead avoids the false-exhausted state.
				let channels = decoder.channels();
				let sample_rate = decoder.sample_rate();
				let samples: Vec<f32> = decoder.collect();
				let buffer = rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples);
				sink.append(buffer.repeat_infinite());
				sink.play();
				state.call_ringing_sink = Some(sink);
			}
			Err(e) => eprintln!("[AUDIO] Error decoding ringing sound: {:?}", e),
		}
	} else {
		eprintln!("[AUDIO] No audio stream available");
	}
}

fn stop_call_ringing_sound(state: &mut AppState) {
	if let Some(sink) = state.call_ringing_sink.take() {
		sink.pause();
	}
}

/// Shows a native desktop notification. Must be called on the wx main/GUI
/// thread - wx (and the underlying WinRT toast COM calls) aren't thread-safe.
/// Returns the shown notification (kept alive by the caller and closeable
/// later via `NotificationMessage::close`, e.g. once the user opens the chat
/// it was for) so it can be dismissed from Action Center on demand, the way
/// official WhatsApp clears a chat's notifications the moment you read it.
fn show_toast_notification(title: &str, body: &str) -> Option<wxdragon::prelude::NotificationMessage> {
	// Debounce toast notifications globally to max 1 per second to avoid flooding on startup
	static LAST_TOAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
	let last = LAST_TOAST.swap(now, std::sync::atomic::Ordering::Relaxed);
	if now - last < 1000 {
		return None;
	}

	// `with_generic(false)` opts into the native backend, which uses the modern
	// WinRT toast (registered via `msw_use_toasts` above) rather than wxdragon's
	// default generic popup on Windows.
	match wxdragon::prelude::NotificationMessage::builder()
		.with_title(title)
		.with_message(body)
		.with_generic(false)
		.build()
	{
		Ok(notification) => {
			notification.show(wxdragon::prelude::TIMEOUT_AUTO);
			Some(notification)
		}
		Err(err) => {
			eprintln!("[NOTIFY] Failed to build notification: {:?}", err);
			None
		}
	}
}

/// Concatenates the PCM audio from one or more same-format WAV segments
/// (produced across a recording's pause/resume cycles) into a single WAV
/// file at `out_path`.
fn concat_wav_segments(segments: &[String], out_path: &str) -> anyhow::Result<()> {
	use hound::{WavReader, WavWriter};
	let mut spec = None;
	let mut writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>> = None;
	for seg in segments {
		let mut reader = WavReader::open(seg)?;
		let seg_spec = reader.spec();
		if writer.is_none() {
			spec = Some(seg_spec);
			writer = Some(WavWriter::create(out_path, seg_spec)?);
		} else if spec != Some(seg_spec) {
			eprintln!("[LOG UI] Voice recording segment {} has a mismatched format, skipping it", seg);
			continue;
		}
		if let Some(ref mut w) = writer {
			for sample in reader.samples::<i16>() {
				w.write_sample(sample?)?;
			}
		}
	}
	match writer {
		Some(w) => {
			w.finalize()?;
			Ok(())
		}
		None => Err(anyhow::anyhow!("no valid voice recording segments to concatenate")),
	}
}

/// One row of the flat, cross-chat "Starred Messages" nav view: enough to
/// show a label and to jump into the right chat at the right message when
/// selected (reuses the same search_target_id jump mechanism as in-chat
/// search results).
#[derive(Clone)]
struct StarredEntry {
	jid: String,
	message_id: String,
	label: String,
}

/// One posted Status/Stories update, as shown in the Status nav view.
#[derive(Clone)]
struct StatusItem {
	id: String,
	text: String,
	media_type: String,
	timestamp: i64,
	viewed: bool,
}

/// All of one contact's (or our own) live status updates, in post order.
#[derive(Clone)]
struct StatusGroup {
	poster_jid: String,
	poster_name: String,
	all_viewed: bool,
	items: Vec<StatusItem>,
}

struct AppState {
	settings: crate::settings::Settings,
	ws_tx: mpsc::Sender<String>,
	ui_tx: UiCommandSender,
	audio_stream: Option<MixerDeviceSink>,
	/// Friendly name of the device `audio_stream` was actually opened
	/// against (as resolved at open time, even for "System Default") - lets
	/// `ensure_audio_stream` notice the resolved device has since changed
	/// (headphones unplugged/replugged) and reopen instead of silently
	/// writing to a now-stale stream until the app is restarted.
	audio_stream_device_name: Option<String>,
	audio_sink: Option<Player>,
	playing_msg_id: Option<String>,
	audio_process: Option<std::process::Child>,
	audio_stdin: Option<std::process::ChildStdin>,
	is_audio_paused: bool,
	audio_path: Option<String>,
	audio_total_secs: f32,
	audio_elapsed_secs: f32,
	audio_speed: f32,
	audio_start_instant: Option<std::time::Instant>,
	is_voice_recording: bool,
	is_voice_paused: bool,
	voice_recording_path: Option<String>,
	/// The currently open WASAPI capture session for the in-progress segment,
	/// if any (see `voice_recorder::RecordingSession`).
	voice_recording_session: Option<voice_recorder::RecordingSession>,
	/// Completed segments from earlier pause/resume cycles in the current
	/// recording, in order. WASAPI capture doesn't have a reliable
	/// pause/resume of its own, so each pause closes off a self-contained WAV
	/// segment instead of trying to keep appending to one file; segments are
	/// spliced together on send.
	voice_recording_segments: Vec<String>,
	voice_recording_start: Option<std::time::Instant>,
	voice_paused_duration: std::time::Duration,
	voice_pause_instant: Option<std::time::Instant>,
	/// Bumped on every action that starts (or tears down) a voice-recording
	/// device open - `StartRecording`, a pause's resume, `DiscardRecording`/
	/// `DiscardVoiceMessage`, `SendVoiceMessage`. `RecordingSession::start`
	/// opens the WASAPI device on a background thread (it used to block the
	/// UI thread for up to 3s doing this); the value captured at the moment
	/// a given open was kicked off is compared against the current value
	/// when that open's result comes back via `VoiceRecordingReady`, so a
	/// stale result (the user discarded/sent/paused again before the device
	/// finished opening) is recognized and just cleaned up instead of
	/// resurrecting recording state out from under the user.
	voice_recording_generation: u64,
	/// True while a `RecordingSession::start_async` call kicked off by the
	/// current `voice_recording_generation` hasn't reported back yet - guards
	/// against a second `StartRecording`/resume landing before the first
	/// device open finishes (the record/pause-resume buttons are also
	/// disabled for this, but that's belt-and-suspenders against e.g. a
	/// screen reader's synthesized click bypassing the disabled state).
	voice_recording_opening: bool,
	/// Outcome of the most recent async device-open, if any is waiting to be
	/// collected by `VoiceRecordingReady`. `RecordingSession::start_async`'s
	/// supervisor thread writes here and wakes the UI thread rather than
	/// blocking it.
	voice_recording_pending: Arc<Mutex<Option<PendingVoiceRecording>>>,
	is_pairing_active: bool,
	chats: Vec<String>,
	filtered_chats: Vec<String>,
	archived_jids: std::collections::HashSet<String>,
	locked_jids: std::collections::HashSet<String>,
	chats_unlocked: bool,
	/// Chats that are community parent groups, i.e. the ones the Communities
	/// tab lists. They stay in Conversations too, tagged "[Community]", the
	/// way they always have.
	community_jids: std::collections::HashSet<String>,
	pinned_jids: std::collections::HashSet<String>,
	muted_jids: std::collections::HashSet<String>,
	unread_counts: std::collections::HashMap<String, u64>,
	/// The chat and count an "N unread messages" divider was captured for
	/// when it was opened, so the divider stays put in the message list even
	/// after `mark_read` zeroes out `unread_counts` moments later. Cleared
	/// once the user sends a reply or reopens the chat.
	unread_divider_jid: Option<String>,
	unread_divider_count: u64,
	/// Whether the app window is currently focused at the OS level (see
	/// `ui::window`'s Activate handler). While unfocused, incoming messages
	/// for the open chat aren't marked read — the user isn't actually
	/// looking at them.
	window_focused: bool,
	/// Message IDs of incoming messages queued for the open chat while the
	/// window was unfocused; drained into the unread divider + a batched
	/// mark_read once focus returns.
	background_unread_message_ids: Vec<String>,
	display_names: std::collections::HashMap<String, String>,
	contacts: std::collections::HashMap<String, String>,
	/// Full synced contact list (name + phone), independent of `contacts`
	/// (which only ever covers jids that already appear in `chats`) - backs
	/// the shared contact-picker dialog for New Conversation, Add call
	/// participant, and Add group member.
	all_contacts: Vec<Contact>,
	/// What picking a result from `new_conv_dialog` should currently do.
	contact_picker_mode: ContactPickerMode,
	/// jids aligned with whatever `new_conv_dialog.results_list` is
	/// currently showing (post-filter), so a selected row index can be
	/// resolved back to a jid.
	contact_picker_current_jids: Vec<String>,
	presence_labels: std::collections::HashMap<String, String>,
	group_state: crate::ui::new_group::NewGroupState,
	add_members_state: crate::ui::add_members::AddMembersState,
	selected_chat: Option<String>,
	admin_group_jids: std::collections::HashSet<String>,
	initial_presence_received: bool,
	messages: Vec<ChatMessage>,
	loading_messages: bool,
	no_more_messages: bool,
	history_loading: bool,
	/// Set to "now" on every "history_sync" update from the backend, cleared
	/// (and the status bar reverted) once a timer tick finds it's gone stale.
	/// WhatsApp delivers history in many independent chunks with no signal
	/// for "that was the last one", so idle-timeout is what actually decides
	/// when the "Syncing history..." status goes away.
	history_sync_last_update: Option<std::time::Instant>,
	/// Stays set to "60s from the last real history_sync update" for as long
	/// as one keeps arriving. Arms the db-size fallback below *only* while
	/// we know a bulk sync is (or was very recently) actually in progress -
	/// without this, any ordinary write (sending one message, say) would
	/// change the db's size and could flash a bogus "Syncing..." status.
	history_sync_armed_until: Option<std::time::Instant>,
	/// Last (main db file + WAL sidecar) size seen by the polling timer, and
	/// when it last changed. Fallback for when the backend's "history_sync"
	/// broadcasts stop arriving (or arrive too sparsely) but writes are
	/// still visibly landing on disk - lets the status bar say *something*
	/// is happening instead of settling on "Connected" while a big sync is
	/// still quietly running in the background. Only trusted while armed
	/// (see above).
	db_size_last_seen: u64,
	db_growth_last_seen: Option<std::time::Instant>,
	/// True while the status bar is showing our own db-growth fallback text,
	/// so the timer knows it's safe to clear it later without clobbering
	/// some unrelated status message it doesn't own.
	db_growth_status_shown: bool,
	completed_history_chats: std::collections::HashSet<String>,
	prev_msg_count: usize,
	/// 0 = Conversations, 1 = Status, 2 = Starred Messages, 3 = Archived Chats.
	active_tab: usize,
	starred_entries: Vec<StarredEntry>,
	/// Parallel to `filtered_chats` while `active_tab == 2`: the starred
	/// message to jump to for each row shown in the nav list.
	filtered_starred_message_ids: Vec<String>,
	status_groups: Vec<StatusGroup>,
	/// Parallel to `filtered_chats` while `active_tab == 1`: the status
	/// message id for each row shown in the nav list.
	filtered_status_message_ids: Vec<String>,
	search_query: String,
	displayed_chat_labels: Vec<String>,
	main_ui_shown: bool,
	/// Whether the WhatsApp connection is up, as last reported by the
	/// backend's "connection_state". Keeps the history-sync timer from
	/// painting "Connected" back over an offline status.
	conn_online: bool,
	/// Set once the current outage has been announced, so repeated failed
	/// retries stay quiet and coming back online gets announced.
	conn_outage_announced: bool,
	/// Set by Reconnect Now so the result of that attempt is always spoken.
	conn_manual_reconnect: bool,
	pending_attachments: Vec<String>,
	pending_reply_id: Option<String>,
	pending_reply_sender: Option<String>,
	pending_reply_preview: Option<String>,
	/// Unsent composer text per chat, saved whenever the conversation is
	/// switched away from (closing it, or opening a different chat) and
	/// restored when that chat is opened again - so leaving mid-draft (e.g.
	/// Escape) never loses what was typed. Cleared once the draft is
	/// actually sent.
	draft_messages: std::collections::HashMap<String, String>,
	pending_receipts: std::collections::HashMap<String, String>,
	downloads_in_progress: std::collections::HashMap<String, i64>,
	pending_copy_announcements: std::collections::HashSet<String>,
	search_target_id: Option<String>,
	search_target_text: Option<String>,
	played_sent_sounds: std::collections::HashSet<String>,
	/// Toast notifications currently showing for each background chat, kept
	/// alive so they can be dismissed (`NotificationMessage::close`) once the
	/// user actually opens that chat - matching official WhatsApp's behavior
	/// of clearing a chat's notifications the moment you read it.
	active_notifications: std::collections::HashMap<String, Vec<wxdragon::prelude::NotificationMessage>>,
	display_items: Vec<whantv::DisplayItem>,
	/// Whether the message list is showing checkboxes for a batch action
	/// (currently just delete), entered via `UiCommand::ToggleSelectionMode`.
	selection_mode: bool,
	/// IDs of messages checked in selection mode, keyed by `DisplayItem::id()`
	/// (a `ChatMessage::id` for a single message, or an album's synthetic id)
	/// so the set survives list rebuilds/reindexing.
	selected_message_ids: std::collections::HashSet<String>,
	/// The rows currently shown in `messages_select_list`, in display order -
	/// dividers are excluded since they aren't selectable. Row index -> this
	/// vector is how the CheckListBox's toggle event (which only reports an
	/// index) is mapped back to a message id, same pattern as
	/// `NewGroupState::current_jids`.
	selection_rows: Vec<whantv::DisplayItem>,
	/// (jid, message_id) pairs still expected as "message_deleted" push
	/// events from the batch `DeleteSelectedMessages` just sent, removed as
	/// each arrives so their individual "Message deleted" announcements can
	/// be suppressed in favor of the one summary announcement already made
	/// for the batch. Scoped by message so an unrelated delete (e.g. synced
	/// from another linked device) arriving mid-batch isn't mistaken for
	/// part of it.
	bulk_delete_pending: std::collections::HashSet<(String, String)>,
	typing_timestamps: std::collections::HashMap<String, std::time::Instant>,
	// Last announced typing/recording state per chat (0=none, 1=typing, 2=recording),
	// so a screen reader hears "X is typing" once per state change instead of on
	// every composing stanza WhatsApp resends while the sender keeps typing.
	typing_announce_state: std::collections::HashMap<String, u8>,
	active_call_audio: Option<call_audio::CallAudioState>,
	active_call_id: Option<String>,
	// active_call_id is only known once call_placed arrives - Call() blocks until the
	// offer is actually on the wire to WhatsApp, a real network round trip, so there's a
	// real window after clicking Call where the UI shows a Leave button before any
	// call_id exists to leave. call_placing tracks "we're in that window", and
	// call_pending_cancel records that the user hit Leave during it, so call_placed can
	// immediately hang the call back up instead of leaving it stuck ringing.
	call_placing: bool,
	call_pending_cancel: bool,
	/// Set when we send answer_call for an incoming call, cleared on
	/// call_connected/call_ended/call_answer_failed. If it's still set once
	/// this has been elapsed too long, the periodic timer treats it as a
	/// silently-failed answer (see typing_timer's tick handler) - covers the
	/// case where the backend never hears back at all rather than erroring.
	call_answer_deadline: Option<std::time::Instant>,
	call_ringing_sink: Option<Player>,
	/// Display name of the caller for the current incoming call, set on
	/// `incoming_call` and cleared as soon as it's answered, rejected, or
	/// otherwise handled. Still set when `call_ended` arrives means it rang
	/// out unanswered - that's what drives the "Missed voice call" toast.
	incoming_call_caller: Option<String>,
	/// Latest roster from `call_participants` for the active group call
	/// (excluding ourselves), empty for a 1:1 call. Kept so the next roster
	/// can be diffed against it to announce who joined or left.
	call_participants: Vec<CallParticipant>,
	group_data_members: Vec<String>,
	/// Action JIDs for `group_data_members`, aligned by index (unfiltered).
	group_data_member_jids: Vec<String>,
	/// Whether each entry in `group_data_members` is an admin/super admin, aligned by index (unfiltered).
	group_data_member_admin: Vec<bool>,
	/// Action JIDs for whatever is currently shown in `members_list`, in
	/// display order, so a selection index from the list box can be mapped
	/// back to a JID even while a search filter is narrowing the list.
	group_data_visible_member_jids: Vec<String>,
	/// Media/Documents/Links tabs in the group info dialog, aligned 1:1 with
	/// what's shown in each ListBox (no search/filtering on these, matching
	/// official WhatsApp).
	group_data_media: Vec<ChatMessage>,
	group_data_documents: Vec<ChatMessage>,
	group_data_links: Vec<ChatMessage>,
	/// JIDs of the community subgroups shown in the Groups tab, aligned 1:1
	/// with `groups_list` rows, so a list selection can be joined by JID.
	group_data_subgroup_jids: Vec<String>,
	/// Same as the group_data_* trio above, but for the 1-on-1 conversation
	/// data dialog.
	conversation_data_media: Vec<ChatMessage>,
	conversation_data_documents: Vec<ChatMessage>,
	conversation_data_links: Vec<ChatMessage>,
	own_push_name: String,
	own_about: String,
	privacy_settings: std::collections::HashMap<String, String>,
	/// Whether `privacy_settings` reflects a real server response yet. Until
	/// this is true the settings dialog's Save button stays disabled, so a
	/// user who presses it before the fetch lands can't silently overwrite
	/// their real privacy settings with the dropdowns' placeholder defaults.
	privacy_settings_loaded: bool,
}

fn build_media_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
	messages.iter().filter(|m| m.is_photo_or_video()).cloned().collect()
}

fn build_document_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
	messages.iter().filter(|m| m.is_document()).cloned().collect()
}

fn build_link_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
	messages.iter().filter(|m| m.first_link().is_some()).cloned().collect()
}

fn populate_message_list(list: &ListBox, messages: &[ChatMessage], label: impl Fn(&ChatMessage) -> String) {
	list.clear();
	for msg in messages {
		list.append(&label(msg));
	}
}

/// Whether a chat belongs in the list shown for `tab`. Conversations (0) hides
/// archived chats, Archived (3) shows only those, and Communities (4) shows
/// only the community parent groups. The Status and Starred tabs build their
/// rows from something other than the chat list and never reach this.
///
/// Communities are kept out of Conversations as well: a community parent
/// carries no messages of its own - everything is posted in its announcement
/// and member sub-groups, which appear as ordinary chats - so a row for it
/// there would be permanently empty.
fn chat_visible_in_tab(
	tab: usize,
	jid: &str,
	archived: &std::collections::HashSet<String>,
	communities: &std::collections::HashSet<String>,
	locked: &std::collections::HashSet<String>,
	chats_unlocked: bool,
) -> bool {
	if chats_unlocked {
		return locked.contains(jid);
	}
	if locked.contains(jid) {
		return false;
	}
	match tab {
		0 => !archived.contains(jid) && !communities.contains(jid),
		3 => archived.contains(jid),
		4 => communities.contains(jid),
		_ => true,
	}
}

/// If `new` is exactly `old` with one element removed and a (possibly
/// different) element inserted at the front, returns the index that element
/// held in `old`. Used to detect "one chat bumped to the top" without an
/// O(n^2) scan, so `apply_chat_filter` can patch the list in place instead
/// of rebuilding it on every message sent or received.
fn find_single_removal_offset(old: &[String], new: &[String]) -> Option<usize> {
	if new.is_empty() || old.len() != new.len() {
		return None;
	}
	let new_tail = &new[1..];
	let mut oi = 0usize;
	let mut ni = 0usize;
	let mut skipped: Option<usize> = None;
	while ni < new_tail.len() {
		if old[oi] == new_tail[ni] {
			oi += 1;
			ni += 1;
		} else if skipped.is_none() {
			skipped = Some(oi);
			oi += 1;
		} else {
			return None;
		}
	}
	Some(skipped.unwrap_or(oi))
}

fn apply_chat_filter(s: &mut AppState, parts: &crate::ui::window::WindowParts) {
	let show_locked_btn = s.active_tab == 0 && !s.locked_jids.is_empty();
	if parts.conversations_parts.locked_chats_btn.is_shown() != show_locked_btn {
		parts.conversations_parts.locked_chats_btn.show(show_locked_btn);
		parts.conversations_parts.panel.layout();
	}

	let lower_query = s.search_query.to_lowercase();

	let mut filtered = Vec::new();
	let mut labels = Vec::new();
	let mut filtered_starred_ids = Vec::new();
	let mut filtered_status_ids = Vec::new();

	if s.active_tab == 1 {
		// Status: a flat list, one row per status item (not grouped by
		// contact) - own statuses first, then everyone else newest-post-first
		// (status_groups is already sorted that way by the backend).
		// filtered_chats holds the poster's jid per row, filtered_status_ids
		// the status message id, so OpenConversation can look either up by idx.
		for group in &s.status_groups {
			for item in &group.items {
				let viewed_tag = if item.viewed { "Viewed" } else { "New" };
				let preview = if !item.text.is_empty() {
					item.text.clone()
				} else if !item.media_type.is_empty() {
					"Media".to_string()
				} else {
					String::new()
				};
				let time_str = if item.timestamp > 0 {
					chrono::DateTime::from_timestamp(item.timestamp, 0)
						.map(|dt| {
							let local_dt: chrono::DateTime<chrono::Local> = dt.into();
							local_dt.format("%I:%M %p").to_string()
						})
						.unwrap_or_default()
				} else {
					String::new()
				};
				let label = if time_str.is_empty() {
					format!("{} [{}]: {}", group.poster_name, viewed_tag, preview)
				} else {
					format!("{} [{}]: {}, {}", group.poster_name, viewed_tag, preview, time_str)
				};
				if lower_query.is_empty() || label.to_lowercase().contains(&lower_query) {
					labels.push(label);
					filtered.push(group.poster_jid.clone());
					filtered_status_ids.push(item.id.clone());
				}
			}
		}
	} else if s.active_tab == 2 {
		// Starred Messages: a flat, cross-chat list built from starred_entries
		// rather than s.chats. filtered_chats still holds one jid per row so
		// OpenConversation's existing index -> jid lookup keeps working.
		for entry in &s.starred_entries {
			if lower_query.is_empty() || entry.label.to_lowercase().contains(&lower_query) {
				labels.push(entry.label.clone());
				filtered.push(entry.jid.clone());
				filtered_starred_ids.push(entry.message_id.clone());
			}
		}
	} else {
		for jid in &s.chats {
			if !chat_visible_in_tab(
				s.active_tab,
				jid,
				&s.archived_jids,
				&s.community_jids,
				&s.locked_jids,
				s.chats_unlocked,
			) {
				continue;
			}

			let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
			if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
				labels.push(s.display_names.get(jid).cloned().unwrap_or(name));
				filtered.push(jid.clone());
			}
		}
	}

	s.filtered_chats = filtered;
	s.filtered_starred_message_ids = filtered_starred_ids;
	s.filtered_status_message_ids = filtered_status_ids;

	// Clearing and refilling the list resets the focused item and makes the
	// screen reader re-announce, so leave the control untouched when the
	// backend re-sends chats that produce identical contents.
	if labels == s.displayed_chat_labels {
		return;
	}

	let list = &parts.conversations_parts.conversations_list;
	let had_focus = list.has_focus();

	// The overwhelmingly common reason the label set changes at all is one
	// chat (whichever was just messaged) jumping to the front, with every
	// other row's relative order and text untouched. Detect that shape and
	// apply it as a single delete+insert instead of tearing down and
	// rebuilding the whole native list control - on a chat list with
	// hundreds of entries, a full clear()+append() per row on every single
	// message sent or received was the visible "UI freezes for a bit" hitch.
	let moved_from = find_single_removal_offset(&s.displayed_chat_labels, &labels);

	list.freeze();
	if let Some(idx) = moved_from {
		list.delete(idx as u32);
		list.insert(&labels[0], 0);
	} else {
		list.clear();
		for label in &labels {
			list.append(label);
		}
	}
	list.thaw();

	if let Some(idx) = s.selected_chat.as_ref().and_then(|sel| s.filtered_chats.iter().position(|j| j == sel)) {
		let count = list.get_count();
		if count > 0 && (idx as u32) < count {
			list.set_selection(idx as u32, true);
		}
	}

	if had_focus {
		list.set_focus();
	}

	s.displayed_chat_labels = labels;
}

pub(crate) fn update_action_buttons(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) -> bool {
	parts.action_panel.freeze();
	let result = update_action_buttons_inner(s, parts);
	parts.action_panel.thaw();
	result
}

fn update_action_buttons_inner(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) -> bool {
	let sel = parts.messages_list.get_selection();
	let mut need_layout = false;

	if let Some(idx) = sel {
		if let Some(item) = s.display_items.get(idx as usize) {
			match item {
				whantv::DisplayItem::Single(msg) => {
					let is_deleted = msg.is_deleted || msg.text.contains("deleted");
					let is_voice =
						msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]");
					let is_media = !is_deleted
						&& !is_voice && !msg.is_system()
						&& (!msg.media_type.is_empty()
							|| msg.is_document() || msg.text.starts_with("[Photo]")
							|| msg.text.starts_with("[Image]")
							|| msg.text.starts_with("[Video]")
							|| msg.text.starts_with("[Document]")
							|| msg.text.starts_with("[Media]"));

					let is_failed = msg.from_me && !msg.is_date_divider && msg.status == "failed";
					let is_uploading = msg.from_me && msg.status == "uploading";
					if is_failed || is_uploading {
						return need_layout | show_send_state_buttons(parts, is_failed);
					}
					need_layout |= set_shown(&parts.action_retry_btn, false);

					if is_media {
						if msg.is_media_downloaded() {
							if parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(false);
								need_layout = true;
							}
							if !parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(true);
								need_layout = true;
							}
							parts.action_open_btn.set_label("Open");
							if !parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(true);
								need_layout = true;
							}
							parts.action_save_as_btn.set_label("Save as");
						} else {
							if parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(false);
								need_layout = true;
							}
							if parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(false);
								need_layout = true;
							}
							if !parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(true);
								need_layout = true;
							}
							let label = if let Some(pct) = s.downloads_in_progress.get(&msg.id) {
								format!("Cancel download ({}%)", pct)
							} else {
								"Download".to_string()
							};
							parts.action_download_btn.set_label(&label);
						}
						return need_layout;
					}
				}
				whantv::DisplayItem::Album { messages, .. } => {
					let valid_messages: Vec<_> =
						messages.iter().filter(|m| !m.is_deleted && !m.text.contains("deleted")).collect();
					if !valid_messages.is_empty() {
						let is_failed = valid_messages.iter().any(|m| m.from_me && m.status == "failed");
						let is_uploading = valid_messages.iter().any(|m| m.from_me && m.status == "uploading");
						if is_failed || is_uploading {
							return need_layout | show_send_state_buttons(parts, is_failed);
						}
						need_layout |= set_shown(&parts.action_retry_btn, false);

						let all_dl = valid_messages.iter().all(|m| m.is_media_downloaded());
						if all_dl {
							if parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(false);
								need_layout = true;
							}
							if !parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(true);
								need_layout = true;
							}
							parts.action_open_btn.set_label("Open album");

							let photo_count = valid_messages
								.iter()
								.filter(|m| {
									m.media_type == "image"
										|| m.text.starts_with("[Photo]")
										|| m.text.starts_with("[Image]")
								})
								.count();
							let video_count = valid_messages
								.iter()
								.filter(|m| m.media_type == "video" || m.text.starts_with("[Video]"))
								.count();
							let save_label = if photo_count > 0 && video_count == 0 {
								"Save all photos"
							} else if video_count > 0 && photo_count == 0 {
								"Save all videos"
							} else {
								"Save all media"
							};
							if !parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(true);
								need_layout = true;
							}
							parts.action_save_as_btn.set_label(save_label);
						} else {
							if parts.action_open_btn.is_shown() {
								parts.action_open_btn.show(false);
								need_layout = true;
							}
							if parts.action_save_as_btn.is_shown() {
								parts.action_save_as_btn.show(false);
								need_layout = true;
							}
							if !parts.action_download_btn.is_shown() {
								parts.action_download_btn.show(true);
								need_layout = true;
							}

							let uncollected = valid_messages.iter().filter(|m| !m.is_media_downloaded()).count();
							let is_dl_in_progress =
								valid_messages.iter().any(|m| s.downloads_in_progress.contains_key(&m.id));
							let dl_label = if is_dl_in_progress {
								format!("Downloading album ({} items remaining)", uncollected)
							} else {
								format!("Download album ({} items remaining)", uncollected)
							};
							parts.action_download_btn.set_label(&dl_label);
						}
					}
					return need_layout;
				}
			}
		}
	}

	need_layout |= set_shown(&parts.action_retry_btn, false);
	if parts.action_download_btn.is_shown() {
		parts.action_download_btn.show(false);
		need_layout = true;
	}
	if parts.action_open_btn.is_shown() {
		parts.action_open_btn.show(false);
		need_layout = true;
	}
	if parts.action_save_as_btn.is_shown() {
		parts.action_save_as_btn.show(false);
		need_layout = true;
	}
	need_layout
}

/// Shows or hides `btn`, returning whether that changed anything (and so
/// needs a layout).
fn set_shown(btn: &Button, show: bool) -> bool {
	if btn.is_shown() == show {
		return false;
	}
	btn.show(show);
	true
}

/// Action buttons for an outgoing message that failed to send (Retry is the
/// only thing to do) or is still uploading (nothing to do yet). Returns
/// whether a layout is needed.
fn show_send_state_buttons(parts: &crate::ui::conversations::ConversationsPanelParts, failed: bool) -> bool {
	let mut need_layout = set_shown(&parts.action_retry_btn, failed);
	need_layout |= set_shown(&parts.action_open_btn, false);
	need_layout |= set_shown(&parts.action_save_as_btn, false);
	need_layout |= set_shown(&parts.action_download_btn, false);
	need_layout
}

/// Syncs `play_voice_btn` / `audio_slider` / `action_panel` visibility to
/// whichever row `messages_list` currently has selected.
///
/// This duplicates the logic in `messages_list`'s `on_selection_changed`
/// handler (ui/window.rs) because a *programmatic* `set_selection()` - used
/// throughout this file to land on a particular row after rebuilding the
/// list (e.g. the newest message after a send, the message just jumped to
/// from search) - doesn't fire that wx event. Without this, those buttons
/// keep reflecting whatever row was selected *before* the rebuild: e.g. if
/// the user had a voice message selected, then sent a plain text message,
/// `play_voice_btn` stayed visible and labeled for the old voice message
/// even though the newly-selected row is the text message they just sent.
/// Call this after any `messages_list.set_selection(...)` that isn't
/// immediately followed by a real user-driven selection change.
fn refresh_voice_action_button(s: &AppState, parts: &crate::ui::conversations::ConversationsPanelParts) {
	parts.action_panel.freeze();
	let mut need_layout = update_action_buttons_inner(s, parts);

	let msg_opt = parts.messages_list.get_selection().and_then(|idx| s.display_items.get(idx as usize)).and_then(
		|item| match item {
			whantv::DisplayItem::Single(m) => Some(m.clone()),
			whantv::DisplayItem::Album { messages, .. } => messages.first().cloned(),
		},
	);

	let is_voice = msg_opt.as_ref().is_some_and(|msg| {
		let is_deleted =
			msg.is_deleted || msg.text == "[This message was deleted]" || msg.text == "[You deleted this message]";
		!is_deleted && (msg.is_audio || msg.text.starts_with("[Voice Message]") || msg.text.starts_with("[Audio]"))
	});

	if parts.play_voice_btn.is_shown() != is_voice {
		parts.play_voice_btn.show(is_voice);
		need_layout = true;
	}

	if is_voice {
		let msg = msg_opt.expect("is_voice implies msg_opt is Some");
		let is_playing = s.playing_msg_id.as_deref() == Some(msg.id.as_str());
		if is_playing {
			let is_paused = s
				.audio_sink
				.as_ref()
				.map(|sink| sink.is_paused() || sink.empty())
				.unwrap_or_else(|| s.audio_process.is_none());
			parts.play_voice_btn.set_label(if is_paused { "Play voice message" } else { "Pause voice message" });
			if !parts.audio_slider.is_shown() {
				parts.audio_slider.show(true);
				parts.audio_progress_label.show(true);
				need_layout = true;
			}
		} else {
			parts.play_voice_btn.set_label("Play voice message");
			if parts.audio_slider.is_shown() {
				parts.audio_slider.show(false);
				parts.audio_progress_label.show(false);
				need_layout = true;
			}
		}
	} else if parts.audio_slider.is_shown() {
		parts.audio_slider.show(false);
		parts.audio_progress_label.show(false);
		need_layout = true;
	}

	let any_action_visible = parts.play_voice_btn.is_shown()
		|| parts.action_retry_btn.is_shown()
		|| parts.action_open_btn.is_shown()
		|| parts.action_save_as_btn.is_shown()
		|| parts.action_download_btn.is_shown()
		|| parts.audio_slider.is_shown();
	if parts.action_panel.is_shown() != any_action_visible {
		parts.action_panel.show(any_action_visible);
		need_layout = true;
	}

	parts.action_panel.thaw();
	if need_layout {
		parts.action_panel.layout();
	}
}

#[cfg(target_os = "windows")]
mod win_cb {
	#[link(name = "user32")]
	unsafe extern "system" {
		pub fn OpenClipboard(hwnd: *mut std::ffi::c_void) -> i32;
		pub fn CloseClipboard() -> i32;
		pub fn EmptyClipboard() -> i32;
		pub fn SetClipboardData(uFormat: u32, hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
	}

	#[link(name = "kernel32")]
	unsafe extern "system" {
		pub fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> *mut std::ffi::c_void;
		pub fn GlobalLock(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
		pub fn GlobalUnlock(hMem: *mut std::ffi::c_void) -> i32;
	}
}

/// Disambiguates a generated clipboard filename against others already used
/// in the same copy batch (e.g. photos from one album share the same
/// second-precision timestamp, so `display_filename()` returns identical
/// names for all of them). Collisions get " (2)", " (3)", ... appended before
/// the extension, matching how Windows Explorer disambiguates duplicate
/// names, so every file lands at its own temp path instead of overwriting
/// the previous one.
fn unique_clipboard_filename(name: &str, used: &mut std::collections::HashMap<String, u32>) -> String {
	let count = used.entry(name.to_string()).or_insert(0);
	*count += 1;
	if *count == 1 {
		return name.to_string();
	}
	let path = std::path::Path::new(name);
	let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
	match path.extension().and_then(|e| e.to_str()) {
		Some(ext) => format!("{} ({}).{}", stem, count, ext),
		None => format!("{} ({})", stem, count),
	}
}

#[cfg(target_os = "windows")]
pub fn copy_files_to_clipboard(paths: &[String]) -> bool {
	let valid_paths: Vec<String> =
		paths.iter().filter(|p| !p.is_empty() && std::path::Path::new(p).exists()).cloned().collect();

	if valid_paths.is_empty() {
		return false;
	}

	let mut buffer: Vec<u16> = Vec::new();
	for p in &valid_paths {
		if let Ok(abs_path) = std::fs::canonicalize(p) {
			let clean = abs_path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
			buffer.extend(clean.encode_utf16());
			buffer.push(0);
		} else {
			buffer.extend(p.encode_utf16());
			buffer.push(0);
		}
	}
	buffer.push(0);

	let dropfiles_size = 20;
	let total_bytes = dropfiles_size + buffer.len() * 2;

	unsafe {
		let h_global = win_cb::GlobalAlloc(0x0042, total_bytes);
		if h_global.is_null() {
			return false;
		}
		let ptr = win_cb::GlobalLock(h_global) as *mut u8;
		if ptr.is_null() {
			return false;
		}

		let p_files_ptr = ptr as *mut u32;
		*p_files_ptr = 20;
		let f_wide_ptr = ptr.add(16) as *mut u32;
		*f_wide_ptr = 1;

		let dst_str_ptr = ptr.add(dropfiles_size) as *mut u16;
		std::ptr::copy_nonoverlapping(buffer.as_ptr(), dst_str_ptr, buffer.len());

		let _ = win_cb::GlobalUnlock(h_global);

		if win_cb::OpenClipboard(std::ptr::null_mut()) != 0 {
			let _ = win_cb::EmptyClipboard();
			let res = win_cb::SetClipboardData(15, h_global);
			let _ = win_cb::CloseClipboard();
			return !res.is_null();
		}
	}
	false
}

#[cfg(not(target_os = "windows"))]
pub fn copy_files_to_clipboard(_paths: &[String]) -> bool {
	false
}

use std::os::windows::process::CommandExt;

fn main() {
	std::panic::set_hook(Box::new(|info| {
		let _ = std::fs::write(paths::log_dir().join("panic.log"), format!("{}", info));
	}));

	// If whaNTV is already running, activate its window (restoring it from
	// the tray if needed) instead of launching a second instance.
	let _instance_lock = match single_instance::acquire_or_activate_existing() {
		Some(lock) => lock,
		None => return,
	};

	let exe_dir = std::env::current_exe()
		.ok()
		.and_then(|p| p.parent().map(|d| d.to_path_buf()))
		.unwrap_or_else(|| std::env::current_dir().unwrap());

	let candidate_paths = [
		exe_dir.join("backend").join("backend.exe"),
		exe_dir.join("backend.exe"),
		exe_dir.join("backend/backend.exe"),
		std::path::PathBuf::from("backend").join("backend.exe"),
		std::path::PathBuf::from("backend/backend.exe"),
		std::path::PathBuf::from("backend").join("whantv_backend.exe"),
		std::path::PathBuf::from("backend/whantv_backend.exe"),
	];

	let mut backend_exe = None;
	for candidate in &candidate_paths {
		if candidate.exists() {
			backend_exe = Some(candidate.clone());
			break;
		}
	}

	let backend_dir = backend_exe
		.as_ref()
		.and_then(|p| p.parent().map(|d| d.to_path_buf()))
		.unwrap_or_else(|| std::path::PathBuf::from("backend"));

	let db_path_str = paths::db_path().to_string_lossy().into_owned();

	// Random rather than PID-derived: on Windows the first process to create a
	// pipe name owns it, so a predictable name can be squatted by a local
	// process that would then receive our commands.
	let pipe_id = format!("whantv-{}", uuid::Uuid::new_v4().simple());
	let go_pipe_name = format!(r"\\.\pipe\{}", pipe_id);

	let backend = backend_exe
		.map(|exe| Arc::new(backend_process::BackendProcess::start(exe, backend_dir, db_path_str, go_pipe_name)));
	let pipe_backend = backend.clone();
	let shutdown_pipe_id = pipe_id.clone();

	let res = wxdragon::main(move |_| {
		// Registers (or repairs) a Start Menu shortcut carrying our Application
		// User Model ID. Windows silently drops toasts from any AUMID that
		// isn't backed by such a shortcut, which is why notifications never
		// showed up in Action Center even though sounds played fine - the app
		// wasn't a Windows-recognized notification sender at all. This is the
		// modern WinRT toast path (wx/msw/rt/notifmsgrt.cpp), not the legacy
		// taskbar-balloon fallback that wxdragon's builder avoids by default.
		#[cfg(target_os = "windows")]
		wxdragon::prelude::NotificationMessage::msw_use_toasts("", "whaNTV");

		let parts = build_main_window();
		let frame = parts.frame;

		let (ui_tx_raw, ui_rx) = mpsc::channel();
		let ui_alive = Arc::new(AtomicBool::new(true));
		let ui_waker = UiWaker::new(frame, ui_alive.clone());
		let ui_tx = UiCommandSender::new(ui_tx_raw, ui_waker.clone());

		// Let future launches of the app (see `single_instance::acquire_or_activate_existing`
		// at the top of `main`) hand off to this window instead of starting a second instance.
		let ui_tx_activate = ui_tx.clone();
		single_instance::listen_for_activation(move || {
			let _ = ui_tx_activate.send(UiCommand::ActivateWindow);
		});

		let (ws_out_tx, ws_out_rx) = mpsc::channel::<String>();
		let ui_tx_clone = ui_tx.clone();
		let ui_alive_clone = ui_alive.clone();

		let ws_out_rx = Arc::new(Mutex::new(ws_out_rx));
		let resend_tx_outer = ws_out_tx.clone();

		thread::spawn(move || {
			loop {
				if !ui_alive_clone.load(Ordering::SeqCst) {
					break;
				}

				// Each pass here follows a dropped or failed pipe connection. If
				// that's because the backend process died, start a fresh one
				// instead of retrying a pipe nobody will ever listen on again.
				if pipe_backend.as_ref().is_some_and(|b| b.restart_if_dead()) {
					let notice = serde_json::json!({
						"type": "connection_state",
						"data": {
							"state": "offline",
							"message": "whaNTV's background service stopped unexpectedly and has been restarted."
						}
					});
					let _ = ui_tx_clone.send(UiCommand::ProcessWebSocketMessage(notice.to_string()));
				}

				let stream_name = pipe_id.as_str().to_ns_name::<GenericNamespaced>().unwrap();
				if let Ok(socket) = LocalSocketStream::connect(stream_name) {
					// Confirm we reached the backend we spawned and not a
					// process that beat it to the pipe name.
					let expected_pid = pipe_backend.as_ref().map(|b| b.pid());
					let peer_ok = match (expected_pid, socket.peer_creds().ok().and_then(|c| c.pid())) {
						(Some(Some(expected)), Some(actual)) => actual == expected,
						(None, _) => true, // no backend exe found; nothing to compare against
						_ => false,
					};

					if !peer_ok {
						drop(socket);
						thread::sleep(Duration::from_secs(2));
						continue;
					}

					let mut socket_writer = socket.try_clone().expect("Failed to clone pipe");
					let mut buf_reader = BufReader::new(socket);
					let mut line = String::new();

					let ui_alive_write = ui_alive_clone.clone();
					let rx_clone = ws_out_rx.clone();
					let resend_tx = resend_tx_outer.clone();

					// Write Thread
					thread::spawn(move || {
						while ui_alive_write.load(Ordering::SeqCst) {
							let msg_opt = {
								let rx = rx_clone.lock().unwrap();
								rx.recv_timeout(Duration::from_millis(500)).ok()
							};

							if let Some(out_msg) = msg_opt {
								let msg = format!("{}\n", out_msg);
								if socket_writer.write_all(msg.as_bytes()).is_err() {
									// Writers from prior connections stay parked
									// on the shared receiver, so a command can be
									// claimed by one whose socket is already dead.
									// Hand it back for the live writer to send.
									let _ = resend_tx.send(out_msg);
									break;
								}
							}
						}
					});

					// Read Loop (Current thread)
					loop {
						if !ui_alive_clone.load(Ordering::SeqCst) {
							break;
						}

						match buf_reader.read_line(&mut line) {
							Ok(0) => break, // EOF
							Ok(_) => {
								let _ = ui_tx_clone.send(UiCommand::ProcessWebSocketMessage(line.trim().to_string()));
								line.clear();
							}
							Err(_) => break, // Any read error breaks and triggers reconnect
						}
					}
				}

				thread::sleep(Duration::from_secs(2));
			}
		});

		let state = Rc::new(RefCell::new(AppState {
			settings: crate::settings::Settings::load(),
			ws_tx: ws_out_tx,
			ui_tx: ui_tx.clone(),
			audio_stream: None,
			audio_stream_device_name: None,
			audio_sink: None,
			playing_msg_id: None,
			audio_process: None,
			audio_stdin: None,
			is_audio_paused: false,
			audio_path: None,
			audio_total_secs: 0.0,
			audio_elapsed_secs: 0.0,
			audio_speed: 1.0,
			audio_start_instant: None,
			chats: Vec::new(),
			filtered_chats: Vec::new(),
			archived_jids: std::collections::HashSet::new(),
			locked_jids: std::collections::HashSet::new(),
			chats_unlocked: false,
			community_jids: std::collections::HashSet::new(),
			pinned_jids: std::collections::HashSet::new(),
			muted_jids: std::collections::HashSet::new(),
			unread_counts: std::collections::HashMap::new(),
			unread_divider_jid: None,
			unread_divider_count: 0,
			window_focused: true,
			background_unread_message_ids: Vec::new(),
			display_names: std::collections::HashMap::new(),
			contacts: std::collections::HashMap::new(),
			all_contacts: Vec::new(),
			contact_picker_mode: ContactPickerMode::OpenChat,
			contact_picker_current_jids: Vec::new(),
			presence_labels: std::collections::HashMap::new(),
			typing_timestamps: std::collections::HashMap::new(),
			typing_announce_state: std::collections::HashMap::new(),
			active_call_audio: None,
			active_call_id: None,
			call_placing: false,
			call_pending_cancel: false,
			call_answer_deadline: None,
			call_ringing_sink: None,
			incoming_call_caller: None,
			call_participants: Vec::new(),
			group_data_members: Vec::new(),
			group_data_member_jids: Vec::new(),
			group_data_member_admin: Vec::new(),
			group_data_visible_member_jids: Vec::new(),
			group_data_media: Vec::new(),
			group_data_documents: Vec::new(),
			group_data_links: Vec::new(),
			group_data_subgroup_jids: Vec::new(),
			conversation_data_media: Vec::new(),
			conversation_data_documents: Vec::new(),
			conversation_data_links: Vec::new(),
			own_push_name: String::new(),
			own_about: String::new(),
			privacy_settings: std::collections::HashMap::new(),
			privacy_settings_loaded: false,
			group_state: crate::ui::new_group::NewGroupState {
				all_contact_labels: Vec::new(),
				all_contact_jids: Vec::new(),
				checked_jids: std::collections::HashSet::new(),
				current_jids: Vec::new(),
			},
			add_members_state: crate::ui::add_members::AddMembersState {
				group_jid: None,
				all_contact_labels: Vec::new(),
				all_contact_jids: Vec::new(),
				checked_jids: std::collections::HashSet::new(),
				current_jids: Vec::new(),
			},
			selected_chat: None,
			admin_group_jids: std::collections::HashSet::new(),
			initial_presence_received: false,
			messages: Vec::new(),
			loading_messages: false,
			no_more_messages: false,
			history_loading: false,
			history_sync_last_update: None,
			history_sync_armed_until: None,
			db_size_last_seen: 0,
			db_growth_last_seen: None,
			db_growth_status_shown: false,
			completed_history_chats: std::collections::HashSet::new(),
			prev_msg_count: 0,
			active_tab: 0,
			starred_entries: Vec::new(),
			filtered_starred_message_ids: Vec::new(),
			status_groups: Vec::new(),
			filtered_status_message_ids: Vec::new(),
			search_query: String::new(),
			displayed_chat_labels: Vec::new(),
			main_ui_shown: false,
			conn_online: false,
			conn_outage_announced: false,
			conn_manual_reconnect: false,
			pending_attachments: Vec::new(),
			pending_reply_id: None,
			pending_reply_sender: None,
			pending_reply_preview: None,
			draft_messages: std::collections::HashMap::new(),
			pending_receipts: std::collections::HashMap::new(),
			downloads_in_progress: std::collections::HashMap::new(),
			pending_copy_announcements: std::collections::HashSet::new(),
			search_target_id: None,
			search_target_text: None,
			played_sent_sounds: std::collections::HashSet::new(),
			active_notifications: std::collections::HashMap::new(),
			is_voice_recording: false,
			is_voice_paused: false,
			voice_recording_path: None,
			voice_recording_session: None,
			voice_recording_segments: Vec::new(),
			voice_recording_start: None,
			voice_paused_duration: std::time::Duration::ZERO,
			voice_pause_instant: None,
			voice_recording_generation: 0,
			voice_recording_opening: false,
			voice_recording_pending: Arc::new(Mutex::new(None)),
			is_pairing_active: false,
			display_items: Vec::new(),
			selection_mode: false,
			selected_message_ids: std::collections::HashSet::new(),
			selection_rows: Vec::new(),
			bulk_delete_pending: std::collections::HashSet::new(),
		}));

		let state_clone = state.clone();
		let parts_clone = Rc::new(parts);
		let parts_ui = parts_clone.clone();

		frame.on_menu_selected(move |event| {
			if event.get_id() == ID_UI_WAKE {
				ui_waker.reset();
				while let Ok(cmd) = ui_rx.try_recv() {
					handle_ui_command(cmd, &state_clone, &parts_ui);
				}
			}
		});

		bind_input_handlers(&parts_clone, ui_tx, &state);

		// Setup audio progress timer
		let audio_timer = Timer::new(&frame);
		let state_timer = state.clone();
		let parts_timer = parts_clone.clone();
		audio_timer.on_tick(move |_| {
			let mut s = match state_timer.try_borrow_mut() {
				Ok(s) => s,
				Err(_) => return, // State already borrowed, skip this update
			};
			if s.playing_msg_id.is_none() {
				return;
			}
			// Check if sink finished naturally
			let finished = s.audio_sink.as_ref().map(|sk| sk.empty()).unwrap_or(false);
			if finished && !s.is_audio_paused {
				s.playing_msg_id = None;
				s.audio_start_instant = None;
				s.audio_elapsed_secs = 0.0;
				parts_timer.conversations_parts.play_voice_btn.set_label("Play voice message");
				parts_timer.conversations_parts.audio_slider.set_value(0);
				parts_timer.conversations_parts.audio_slider.show(false);
				parts_timer.conversations_parts.audio_speed_btn.show(false);
				parts_timer.conversations_parts.audio_progress_label.show(false);
				return;
			}
			if !s.is_audio_paused {
				if let Some(inst) = s.audio_start_instant {
					let elapsed = s.audio_elapsed_secs + inst.elapsed().as_secs_f32() * s.audio_speed;
					let total = s.audio_total_secs;
					let max_sec = (total as i32).max(1);
					let pos = (elapsed as i32).max(0).min(max_sec);
					if parts_timer.conversations_parts.audio_slider.is_shown() {
						parts_timer.conversations_parts.audio_slider.set_range(0, max_sec);
						parts_timer.conversations_parts.audio_slider.set_value(pos);
					}
				}
			}
		});
		audio_timer.start(500, false);
		Box::leak(Box::new(audio_timer));

		let typing_timer = Timer::new(&frame);
		let state_typing_timer = state.clone();
		let parts_typing_timer = parts_clone.clone();
		typing_timer.on_tick(move |_| {
			let mut s = match state_typing_timer.try_borrow_mut() {
				Ok(s) => s,
				Err(_) => return,
			};
			let now = std::time::Instant::now();
			let timeout = std::time::Duration::from_secs(30);
			let expired: Vec<String> = s
				.typing_timestamps
				.iter()
				.filter(|(_, ts)| now.duration_since(**ts) >= timeout)
				.map(|(jid, _)| jid.clone())
				.collect();
			if expired.is_empty() {
				return;
			}
			let selected = s.selected_chat.clone();
			for jid in &expired {
				s.typing_timestamps.remove(jid);
				s.typing_announce_state.remove(jid);
			}
			if let Some(ref sel_jid) = selected {
				if expired.iter().any(|j| j == sel_jid) {
					let norm_jid = if let Some((user, server)) = sel_jid.split_once('@') {
						let base = user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
						format!("{}@{}", base, server)
					} else {
						sel_jid.clone()
					};
					let label = s
						.presence_labels
						.get(sel_jid.as_str())
						.or_else(|| s.presence_labels.get(&norm_jid))
						.cloned()
						.unwrap_or_else(|| {
							s.contacts.get(sel_jid).cloned().unwrap_or_else(|| "Conversation data".to_string())
						});
					parts_typing_timer.conversations_parts.conv_data_btn.set_label(&label);
				}
			}

			// Watchdog for a silently-failed call answer: if the backend never
			// sends back call_connected, call_ended, or call_answer_failed at
			// all (e.g. the callee's media path never comes up), the UI would
			// otherwise sit on "Answering..." forever with no way to tell the
			// Accept press did nothing.
			if let Some(deadline) = s.call_answer_deadline {
				if deadline.elapsed() >= std::time::Duration::from_secs(15) {
					s.call_answer_deadline = None;
					// Giving up locally doesn't mean the call is actually gone on
					// the backend/network - if we don't also tell it to hang up,
					// the call can still connect moments later (call_connected
					// doesn't check active_call_id - see below) and reopen the UI
					// for a call the user was just told had failed, with no way
					// to leave it since active_call_id is about to become None.
					if let Some(call_id) = s.active_call_id.take() {
						let msg = serde_json::json!({
							"action": "leave_call",
							"data": { "call_id": call_id }
						});
						let _ = s.ws_tx.send(msg.to_string());
					}
					s.active_call_audio = None;
					s.call_participants.clear();
					parts_typing_timer.call_parts.frame.show(false);
					play_call_fail_sound(&mut s);
					live_region::announce(
						parts_typing_timer.conversations_parts.messages_announcer.clone(),
						"No response - the call could not be answered.",
					);
				}
			}
		});
		typing_timer.start(5000, false);
		Box::leak(Box::new(typing_timer));

		// WhatsApp delivers history sync as many independent chunks with no
		// "that was the last one" signal, so there's no event to hang a
		// completion status on - and on a very large account, the
		// "history_sync" broadcasts themselves have sometimes gone quiet for
		// a stretch while the backend was still visibly writing to disk.
		// This tick does two things: clears the detailed "history_sync"
		// status once 3s pass with no new update (same as before), and -
		// only while "armed" by a real history_sync update within the last
		// 60s, so an ordinary message send doesn't get misread as a bulk
		// sync - falls back to a bare "Syncing..." for up to 5s after the db
		// (or its WAL sidecar) last changed size. `db_growth_status_shown`
		// tracks whether *we* put that fallback text up, so clearing it
		// never clobbers some unrelated status message we don't own.
		let history_sync_timer = Timer::new(&frame);
		let state_history_sync_timer = state.clone();
		let parts_history_sync_timer = parts_clone.clone();
		history_sync_timer.on_tick(move |_| {
			let mut s = match state_history_sync_timer.try_borrow_mut() {
				Ok(s) => s,
				Err(_) => return,
			};

			let now = std::time::Instant::now();
			let db_size = paths::db_and_wal_size();
			// Only growth counts as sync activity. The backend's post-sync WAL
			// checkpoint moves data from -wal into the main file and truncates
			// -wal, which always shrinks the total - that's housekeeping, not
			// more history arriving, and shouldn't bring "Syncing..." back.
			if db_size > s.db_size_last_seen {
				s.db_growth_last_seen = Some(now);
			}
			s.db_size_last_seen = db_size;

			if !s.main_ui_shown {
				return;
			}

			let history_sync_fresh = s
				.history_sync_last_update
				.map(|t| t.elapsed() < std::time::Duration::from_millis(3000))
				.unwrap_or(false);
			if history_sync_fresh {
				s.db_growth_status_shown = false;
				return;
			}
			let real_signal_just_expired = s.history_sync_last_update.take().is_some();

			let armed = s.history_sync_armed_until.map(|t| now < t).unwrap_or(false);
			let db_growth_fresh =
				s.db_growth_last_seen.map(|t| t.elapsed() < std::time::Duration::from_millis(5000)).unwrap_or(false);

			if armed && db_growth_fresh {
				parts_history_sync_timer.frame.set_status_text("Status: Connected (Syncing...)", 0);
				s.db_growth_status_shown = true;
			} else if real_signal_just_expired || s.db_growth_status_shown {
				if s.conn_online {
					parts_history_sync_timer.frame.set_status_text("Status: Connected", 0);
				}
				s.db_growth_status_shown = false;
			}
		});
		history_sync_timer.start(1500, false);
		Box::leak(Box::new(history_sync_timer));

		// Deliberately show nothing yet - not even the frame. The backend
		// hasn't reported login state, so anything we reveal now is a guess we
		// have to correct a few seconds later. Focusing an empty frame here
		// made NVDA announce "whaNTV panel" and then, seconds later, "search
		// conversations" once the chat list appeared. Instead we wait: the
		// "connected" handler shows the fully-populated chat list and focuses
		// the search field in one step (so NVDA says "whaNTV, search
		// conversations"), and "needs_login" shows the login panel - each
		// revealing its window only once, already correct. The frame is hidden,
		// not deleted, so the event loop stays alive to receive those events.
	});
	if let Err(e) = res {
		let _ = std::fs::write(paths::log_dir().join("main_error.log"), format!("{:?}", e));
	}

	if let Some(backend) = backend {
		// Killing the backend outright skips both the WhatsApp disconnect and
		// the database close, which strands the SQLite -wal/-shm files. Ask it
		// to exit on its own first. (Its kill-on-close job is the backstop if
		// it never comes down: the handle closes when this process exits.)
		backend.shutdown(|| request_backend_shutdown(&shutdown_pipe_id));
	}
}

fn request_backend_shutdown(pipe_id: &str) {
	let Ok(name) = pipe_id.to_ns_name::<GenericNamespaced>() else {
		return;
	};
	let Ok(mut socket) = LocalSocketStream::connect(name) else {
		return;
	};
	let Ok(drain_socket) = socket.try_clone() else {
		return;
	};

	// The backend greets every new connection with a connected event and the
	// full chat list, writing it before it reads anything. Left undrained the
	// pipe fills, the backend blocks mid-write, and it never reaches our
	// command -- so the reader has to run concurrently with the write.
	thread::spawn(move || {
		let mut reader = BufReader::new(drain_socket);
		let mut buf = String::new();
		while matches!(reader.read_line(&mut buf), Ok(n) if n > 0) {
			buf.clear();
		}
	});

	let _ = socket.write_all(b"{\"action\":\"shutdown\"}\n");
	let _ = socket.flush();
}

/// Perform a soft seek by stopping the current sink and re-opening the file
/// from `target_secs`. Returns the new Sink if successful.
fn do_seek(stream_handle: &MixerDeviceSink, path: &str, target_secs: f32, speed: f32) -> Option<Player> {
	use rodio::Source;
	let sink = Player::connect_new(&stream_handle.mixer());
	if let Ok(file) = std::fs::File::open(path) {
		if let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(file)) {
			let source = decoder.skip_duration(std::time::Duration::from_secs_f32(target_secs.max(0.0))).wsola(speed);
			sink.append(source);
			sink.play();
			return Some(sink);
		} else if let Some(opus_buf) = decode_opus_file(path) {
			let source = opus_buf.skip_duration(std::time::Duration::from_secs_f32(target_secs.max(0.0))).wsola(speed);
			sink.append(source);
			sink.play();
			return Some(sink);
		}
	}
	None
}

fn get_status_rank(s: &str) -> i32 {
	match s {
		"pending" => 0,
		"uploading" => 1,
		"sent" => 2,
		"delivered" => 3,
		"read" => 4,
		"played" => 5,
		_ => 0,
	}
}

/// One other person in a group call, as sent by the backend's
/// `call_participants` event. `state` is WhatsApp's roster state, e.g.
/// "connected", "outgoing", "ringing", "invited" or "rejected".
#[derive(Clone)]
struct CallParticipant {
	jid: String,
	name: String,
	state: String,
}

fn parse_call_participants(data: &serde_json::Value) -> Vec<CallParticipant> {
	let Some(list) = data.get("participants").and_then(|p| p.as_array()) else {
		return Vec::new();
	};
	list.iter()
		.filter_map(|p| {
			let jid = p.get("jid").and_then(|v| v.as_str())?.to_string();
			let name = p
				.get("name")
				.and_then(|v| v.as_str())
				.filter(|n| !n.is_empty())
				.map(|n| n.to_string())
				.unwrap_or_else(|| display_name_for_jid(&jid));
			let state = p.get("state").and_then(|v| v.as_str()).unwrap_or("").to_string();
			Some(CallParticipant { jid, name, state })
		})
		.collect()
}

fn join_names(names: &[&str]) -> String {
	match names {
		[] => String::new(),
		[one] => one.to_string(),
		[rest @ .., last] => format!("{} and {}", rest.join(", "), last),
	}
}

/// Status line for the call window during a group call, e.g.
/// "In group call with Asha and Ben. Ringing Chris."
fn group_call_status_text(participants: &[CallParticipant]) -> String {
	let connected: Vec<&str> =
		participants.iter().filter(|p| p.state == "connected").map(|p| p.name.as_str()).collect();
	let ringing: Vec<&str> = participants
		.iter()
		.filter(|p| p.state != "connected" && p.state != "rejected")
		.map(|p| p.name.as_str())
		.collect();
	match (connected.is_empty(), ringing.is_empty()) {
		(true, true) => "In group call. No one else is here yet.".to_string(),
		(true, false) => format!("Calling {}.", join_names(&ringing)),
		(false, true) => format!("In group call with {}.", join_names(&connected)),
		(false, false) => {
			format!("In group call with {}. Ringing {}.", join_names(&connected), join_names(&ringing))
		}
	}
}

/// What changed between two group-call rosters, as screen-reader
/// announcements ("Asha joined the call.", "Ben left the call.", ...).
fn call_participant_changes(old: &[CallParticipant], new: &[CallParticipant]) -> Vec<String> {
	let mut changes = Vec::new();
	for p in new {
		let before = old.iter().find(|o| o.jid == p.jid).map(|o| o.state.as_str());
		if p.state == "connected" && before != Some("connected") {
			changes.push(format!("{} joined the call.", p.name));
		} else if p.state != "connected" && before == Some("connected") {
			changes.push(format!("{} left the call.", p.name));
		} else if p.state == "rejected" && before != Some("rejected") {
			changes.push(format!("{} declined the call.", p.name));
		}
	}
	for o in old {
		if o.state == "connected" && !new.iter().any(|p| p.jid == o.jid) {
			changes.push(format!("{} left the call.", o.name));
		}
	}
	changes
}

/// Falls back to a formatted phone number (e.g. "+91 96671 74783") instead
/// of the raw JID when a chat has no saved contact name yet — most visibly
/// hit right after `OpenChatByPhone` jumps to a number that isn't a contact.
fn display_name_for_jid(jid: &str) -> String {
	let raw_phone = jid.split('@').next().unwrap_or(jid);
	if !raw_phone.is_empty() && raw_phone.chars().all(|c| c.is_ascii_digit()) {
		if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
			return parsed.format().mode(phonenumber::Mode::International).to_string();
		}
		return format!("+{}", raw_phone);
	}
	jid.to_string()
}

/// Formats a raw digits-only phone number (e.g. "919800000003") as
/// "+91 96671 74783", falling back to a plain "+"-prefixed string if it
/// doesn't parse as a phone number at all.
fn format_phone_digits(raw_phone: &str) -> String {
	if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
		return parsed.format().mode(phonenumber::Mode::International).to_string();
	}
	format!("+{}", raw_phone)
}

/// Builds the "Name +91 96671 74783" (or just the number, if unnamed) label
/// shown for a contact in the shared contact-picker dialog.
fn contact_picker_label(contact: &Contact) -> String {
	// A "@lid" jid's user part is WhatsApp's own large synthetic identifier,
	// not a phone number - it just happens to also be all digits, so without
	// this check an unresolved lid contact (no known phone) would get that
	// identifier formatted as if it were one, e.g. "Name +1 10000000000004".
	let is_lid = contact.jid.ends_with("@lid");
	let raw_phone = if !contact.phone.is_empty() {
		Some(contact.phone.as_str())
	} else if !is_lid {
		Some(contact.jid.split('@').next().unwrap_or(&contact.jid))
	} else {
		None
	};
	let formatted_phone =
		raw_phone.filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).map(format_phone_digits);
	match (contact.name.trim().is_empty(), formatted_phone) {
		(false, Some(phone)) => format!("{} {}", contact.name.trim(), phone),
		(false, None) => contact.name.trim().to_string(),
		(true, Some(phone)) => phone,
		(true, None) => contact.jid.clone(),
	}
}

/// Rebuilds `messages_select_list` from the current `display_items` while
/// selection mode is active - a no-op otherwise. Called everywhere
/// `messages_list` itself gets a structural rebuild (new message, delete,
/// history load) so the selection list never shows a stale row set while the
/// user is mid-selection. Previously-checked message IDs that no longer
/// exist in the rebuilt rows (e.g. deleted from another device) are dropped
/// rather than left dangling in `selected_message_ids`.
fn refresh_selection_list(s: &mut AppState, parts: &WindowParts) {
	if !s.selection_mode {
		return;
	}
	let is_group = s.selected_chat.as_deref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
	s.selection_rows = s
		.display_items
		.iter()
		.filter(|item| match item {
			whantv::DisplayItem::Single(m) => !m.is_date_divider && !m.is_unread_divider,
			_ => true,
		})
		.cloned()
		.collect();
	let valid_ids: std::collections::HashSet<String> = s.selection_rows.iter().map(|i| i.id().to_string()).collect();
	s.selected_message_ids.retain(|id| valid_ids.contains(id));

	parts.conversations_parts.messages_select_list.freeze();
	parts.conversations_parts.messages_select_list.clear();
	for (idx, item) in s.selection_rows.iter().enumerate() {
		parts.conversations_parts.messages_select_list.append(&item.format_display(is_group));
		if s.selected_message_ids.contains(item.id()) {
			parts.conversations_parts.messages_select_list.check(idx as u32, true);
		}
	}
	parts.conversations_parts.messages_select_list.thaw();
	parts.conversations_parts.selection_count_label.set_label(&format!("{} selected", s.selected_message_ids.len()));
}

/// Sets the group data dialog's `make_admin_btn` label to match whether the
/// currently selected member (by position in the filtered `members_list`) is
/// already an admin, mirroring official WhatsApp's per-member "Make admin" /
/// "Dismiss as admin" toggle instead of one static label regardless of
/// who's selected.
fn update_make_admin_button_label(state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let s = state.borrow();
	let is_admin = parts
		.group_data_dialog
		.members_list
		.get_selection()
		.and_then(|idx| s.group_data_visible_member_jids.get(idx as usize))
		.and_then(|jid| s.group_data_member_jids.iter().position(|j| j == jid))
		.and_then(|i| s.group_data_member_admin.get(i).copied())
		.unwrap_or(false);
	drop(s);
	parts.group_data_dialog.make_admin_btn.set_label(if is_admin { "Dismiss as admin" } else { "Make admin" });
}

/// Re-filters `s.all_contacts` by `query` and repaints the New Group dialog's
/// `contacts_listbox` (a wxCheckListBox) with the matches, preserving which
/// jids are already checked. Shared by opening the dialog and by a fresh
/// `contacts` response landing while it's already open - without the latter,
/// the box stays empty for the entire first `get_contacts` round trip, and
/// tabbing focus into an empty wxCheckListBox is what trips the "bad
/// wxCheckListBox index" debug assert in NVDA's accessibility queries.
fn refresh_new_group_contacts_list(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let mut filtered_labels = Vec::new();
	let mut filtered_jids = Vec::new();
	for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
		if q.is_empty() || lbl.to_lowercase().contains(&q) {
			filtered_labels.push(lbl.clone());
			filtered_jids.push(jid.clone());
		}
	}
	parts.new_group_dialog.contacts_listbox.clear();
	for lbl in &filtered_labels {
		parts.new_group_dialog.contacts_listbox.append(lbl);
	}
	let gs = &mut s.group_state;
	gs.all_contact_labels = all_labels;
	gs.all_contact_jids = all_jids;
	gs.current_jids = filtered_jids;
	for (idx, jid) in gs.current_jids.iter().enumerate() {
		if gs.checked_jids.contains(jid) {
			parts.new_group_dialog.contacts_listbox.check(idx as u32, true);
		}
	}
	// See the comment on the equivalent reset in window.rs's search handler:
	// clear()+append() leaves the native listbox caret wherever it was
	// before the rebuild, which NVDA can query past the end of a shorter
	// list and trip wx's "bad wxCheckListBox index" assert.
	// wxListBox::SetSelection asserts/crashes if given index 0 on an empty
	// list (only wxNOT_FOUND, which wxdragon's u32 API can't express, is
	// valid there) - so there's nothing to reset when the filter has no
	// matches, only when it has at least one.
	if !gs.current_jids.is_empty() {
		parts.new_group_dialog.contacts_listbox.set_selection(0, true);
	}
}

/// Recomputes `group_state.all_contact_labels`/`all_contact_jids` from
/// `s.all_contacts` without touching the New Group dialog's listbox widget.
/// Used when a `contacts` response lands while the dialog is open, so the
/// debounced rebuild it then schedules (via `new_group_contacts_timer`) has
/// fresh data to filter once it actually fires.
fn sync_new_group_contact_labels(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let gs = &mut s.group_state;
	gs.all_contact_labels = all_labels;
	gs.all_contact_jids = all_jids;
}

/// Same as `refresh_new_group_contacts_list` above, but for the Add Members
/// dialog's `contacts_listbox`, and additionally excluding anyone already in
/// the group (`group_data_member_jids`) - there's no reason to offer
/// re-adding an existing member.
fn refresh_add_members_contacts_list(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let existing_members: std::collections::HashSet<String> = s.group_data_member_jids.iter().cloned().collect();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		if existing_members.contains(&contact.jid) {
			continue;
		}
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let mut filtered_labels = Vec::new();
	let mut filtered_jids = Vec::new();
	for (lbl, jid) in all_labels.iter().zip(all_jids.iter()) {
		if q.is_empty() || lbl.to_lowercase().contains(&q) {
			filtered_labels.push(lbl.clone());
			filtered_jids.push(jid.clone());
		}
	}
	parts.add_members_dialog.contacts_listbox.clear();
	for lbl in &filtered_labels {
		parts.add_members_dialog.contacts_listbox.append(lbl);
	}
	let ams = &mut s.add_members_state;
	ams.all_contact_labels = all_labels;
	ams.all_contact_jids = all_jids;
	ams.current_jids = filtered_jids;
	for (idx, jid) in ams.current_jids.iter().enumerate() {
		if ams.checked_jids.contains(jid) {
			parts.add_members_dialog.contacts_listbox.check(idx as u32, true);
		}
	}
	// See the equivalent reset in refresh_new_group_contacts_list/window.rs:
	// clear()+append() leaves the native listbox caret wherever it was
	// before the rebuild, which NVDA can query past the end of a shorter
	// list and trip wx's "bad wxCheckListBox index" assert. SetSelection
	// itself asserts/crashes given index 0 on an empty list (only
	// wxNOT_FOUND, which wxdragon's u32 API can't express, is valid there).
	if !ams.current_jids.is_empty() {
		parts.add_members_dialog.contacts_listbox.set_selection(0, true);
	}
}

/// Recomputes `add_members_state.all_contact_labels`/`all_contact_jids` from
/// `s.all_contacts` (excluding existing group members) without touching the
/// Add Members dialog's listbox widget. Used when a `contacts` response
/// lands while the dialog is open, so the debounced rebuild it then
/// schedules (via `add_members_contacts_timer`) has fresh data to filter
/// once it actually fires.
fn sync_add_members_contact_labels(state: &Rc<RefCell<AppState>>) {
	let mut s = state.borrow_mut();
	let existing_members: std::collections::HashSet<String> = s.group_data_member_jids.iter().cloned().collect();
	let mut all_labels = Vec::new();
	let mut all_jids = Vec::new();
	for contact in &s.all_contacts {
		if existing_members.contains(&contact.jid) {
			continue;
		}
		all_labels.push(contact_picker_label(contact));
		all_jids.push(contact.jid.clone());
	}
	let ams = &mut s.add_members_state;
	ams.all_contact_labels = all_labels;
	ams.all_contact_jids = all_jids;
}

/// Re-filters `s.all_contacts` by `query` (against the same "Name +phone"
/// text shown to the user) and repaints `new_conv_dialog.results_list` with
/// the matches, recording their jids in `contact_picker_current_jids` so a
/// selected row index can be resolved back to a jid. Shared by opening the
/// dialog (empty query - the whole contact list), typing in its search
/// field, and a fresh `contacts` response landing while it's already open.
fn apply_contact_picker_filter(query: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let q = query.trim().to_lowercase();
	let mut s = state.borrow_mut();
	let mut labels = Vec::new();
	let mut jids = Vec::new();
	for contact in &s.all_contacts {
		let label = contact_picker_label(contact);
		if q.is_empty() || label.to_lowercase().contains(&q) {
			labels.push(label);
			jids.push(contact.jid.clone());
		}
	}
	parts.new_conv_dialog.results_list.clear();
	for label in &labels {
		parts.new_conv_dialog.results_list.append(label);
	}
	s.contact_picker_current_jids = jids;
}

/// Repaints the tray icon's tooltip with the current unread total, so the count
/// can be read straight off the notification area without restoring the window.
///
/// Called from every place that touches `unread_counts` rather than from one
/// funnel, because there isn't one: the backend's chats broadcast covers most of
/// it, but marking a chat read/unread, deleting a chat and leaving a group all
/// adjust the map locally with no resync behind them.
///
/// Gated on exactly the condition `ui::window` installs the icon under - a
/// refresh must never resurrect an icon switched off in Settings or pulled on
/// logout, and `set_icon` on its own would do precisely that.
/// Adds the tray icon to the notification area, then immediately writes the
/// current unread count over it.
///
/// The two calls are deliberate and not redundant: the shell takes the item's
/// accessible name from the tooltip it was *added* with and sends every later
/// change to the description, so adding the icon under [`whantv::TRAY_APP_NAME`]
/// and only then writing counts is what makes a screen reader say
/// "whaNTV, 7 unread messages" instead of pairing the live count with whatever
/// it happened to be at login.
fn install_tray_icon(s: &AppState, parts: &WindowParts) {
	if !s.settings.show_tray_icon || !s.main_ui_shown {
		return;
	}
	parts.tray_icon.set_icon(&crate::ui::theme::build_app_icon(32), whantv::TRAY_APP_NAME);
	refresh_tray_tooltip(s, parts);
}

fn refresh_tray_tooltip(s: &AppState, parts: &WindowParts) {
	if !s.settings.show_tray_icon || !s.main_ui_shown {
		return;
	}
	let unread = whantv::unread_notification_total(
		&s.unread_counts,
		&s.muted_jids,
		&s.archived_jids,
		&s.settings.archived_alert_levels,
	);
	parts.tray_icon.set_icon(&crate::ui::theme::build_app_icon(32), &whantv::tray_tooltip_text(unread));
}

/// Drops the "N unread messages" divider from `jid`'s message list right
/// away, if `jid` is the chat currently open and it has one - shared by
/// replying (obviously seen) and explicitly marking the chat read from the
/// chat-list context menu. Removing just the divider row in place is O(1)
/// against the widget rather than rebuilding+re-appending the whole
/// (unvirtualized) list, which visibly froze the UI on long chats.
fn clear_unread_divider_for_open_chat(s: &mut AppState, parts: &WindowParts, jid: &str) {
	if s.unread_divider_jid.as_deref() != Some(jid) {
		return;
	}
	s.unread_divider_jid = None;
	s.unread_divider_count = 0;
	s.background_unread_message_ids.clear();
	s.messages.retain(|m| !m.is_unread_divider);
	if let Some(di) =
		s.display_items.iter().position(|item| matches!(item, whantv::DisplayItem::Single(m) if m.is_unread_divider))
	{
		s.display_items.remove(di);
		parts.conversations_parts.messages_list.delete(di as u32);
	}
	refresh_selection_list(s, parts);
}

/// Saves whatever is currently sitting in the composer into `draft_messages`
/// under the chat that's about to be left, if any - so switching away (to
/// another chat, or closing the conversation entirely) never silently
/// discards an unsent message. A no-op if the composer is empty or no chat
/// is currently open.
fn save_composer_draft(s: &mut AppState, parts: &WindowParts) {
	let Some(prev_jid) = s.selected_chat.clone() else { return };
	let text = parts.conversations_parts.message_field.get_value();
	if text.trim().is_empty() {
		s.draft_messages.remove(&prev_jid);
	} else {
		s.draft_messages.insert(prev_jid, text);
	}
}

/// Drops a pending "reply to" quote and hides its Remove Quote button. A
/// quote belongs to the chat it was picked in: every send path reads
/// `pending_reply_id` against whatever chat is open *now*, so leaving it set
/// across a chat switch sent the next message in the other chat as a reply
/// to a message that isn't even there. Official WhatsApp cancels it too.
fn clear_pending_reply(s: &mut AppState, parts: &WindowParts) {
	s.pending_reply_id = None;
	s.pending_reply_sender = None;
	s.pending_reply_preview = None;
	parts.conversations_parts.remove_quote_btn.show(false);
}

/// Switches the conversation panel over to `jid`, requesting its message
/// history. Shared by opening a chat from the sidebar (`OpenConversation`)
/// and jumping straight to a phone number found in message text
/// (`OpenChatByPhone`) — the latter's jid may not be in `filtered_chats` yet.
fn open_chat_for_jid(jid: &str, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	let mut s = state.borrow_mut();
	save_composer_draft(&mut s, parts);
	// Leaving a chat with unsent text in the composer must not leave the
	// recipient staring at a "typing…" indicator that never clears - real
	// WhatsApp Web sends "paused" the moment you navigate away, not just
	// when the text is deleted or sent.
	if let Some(prev_jid) = s.selected_chat.clone() {
		if prev_jid != jid {
			let payload = serde_json::json!({
				"action": "send_typing",
				"data": { "jid": prev_jid, "state": "paused", "media": "" }
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
	}
	// Re-opening the same chat keeps the quote; any other chat drops it.
	if s.selected_chat.as_deref() != Some(jid) {
		clear_pending_reply(&mut s, parts);
	}
	s.selected_chat = Some(jid.to_string());
	// Mirrors official WhatsApp: opening a chat clears any of its
	// notifications still sitting in Action Center.
	if let Some(notifications) = s.active_notifications.remove(jid) {
		for notification in notifications {
			notification.close();
		}
	}
	let unread = s.unread_counts.get(jid).copied().unwrap_or(0);
	s.unread_divider_jid = if unread > 0 { Some(jid.to_string()) } else { None };
	s.unread_divider_count = unread;
	s.background_unread_message_ids.clear();
	s.initial_presence_received = false;
	s.messages.clear();
	s.loading_messages = false;
	s.no_more_messages = false;
	s.history_loading = false;
	s.prev_msg_count = 0;
	s.pending_attachments.clear();
	// Selection mode's row list is tied to the chat it was built from -
	// switching chats mid-selection would leave it pointing at messages that
	// no longer belong to the now-displayed conversation.
	if s.selection_mode {
		s.selection_mode = false;
		s.selected_message_ids.clear();
		s.selection_rows.clear();
		parts.conversations_parts.messages_select_list.show(false);
		parts.conversations_parts.selection_bar_panel.show(false);
		parts.conversations_parts.messages_list.show(true);
		parts.conversations_parts.message_field.show(true);
		parts.conversations_parts.add_attachment_btn.show(true);
		parts.conversations_parts.record_voice_btn.show(true);
	}
	parts.conversations_parts.conversation_panel.show(true);
	parts.conversations_parts.play_voice_btn.show(false);
	parts.conversations_parts.action_panel.show(false);

	let name = s.contacts.get(jid).cloned().unwrap_or_else(|| display_name_for_jid(jid));
	let label = s.presence_labels.get(jid).cloned().unwrap_or_else(|| name.clone());
	parts.conversations_parts.conv_data_btn.set_label(&label);

	parts.attachment_dialog.attachment_info_label.set_label("");
	parts.conversations_parts.messages_list.clear();
	let draft = s.draft_messages.get(jid).cloned().unwrap_or_default();
	parts.conversations_parts.message_field.set_value(&draft);
	parts.conversations_parts.send_message_btn.show(!draft.trim().is_empty());
	parts.conversations_parts.panel.layout();
	let limit = 9999999;
	let payload = serde_json::json!({
		"action": "get_messages",
		"data": { "jid": jid, "limit": limit, "offset": 0 }
	});
	let _ = s.ws_tx.send(payload.to_string());
	drop(s);
	handle_ui_command(UiCommand::SubscribePresence(jid.to_string()), state, parts);
	handle_ui_command(UiCommand::MarkRead { jid: jid.to_string(), message_ids: Vec::new() }, state, parts);
	parts.conversations_parts.messages_list.set_focus();
}

fn handle_ui_command(cmd: UiCommand, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	match cmd {
		UiCommand::PrepareNewGroup => {
			parts.new_group_dialog.ok_btn.enable(true);
			parts.new_group_dialog.name_field.set_value("");
			parts.new_group_dialog.search_field.set_value("");
			parts.new_group_dialog.extra_number_field.set_value("");
			state.borrow_mut().group_state.checked_jids.clear();
			refresh_new_group_contacts_list("", state, parts);
			let payload = serde_json::json!({ "action": "get_contacts" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			parts.new_group_dialog.dialog.show_modal();
		}
		UiCommand::PrepareAddMembers => {
			let group_jid = state.borrow().selected_chat.clone();
			if group_jid.is_none() {
				return;
			}
			parts.add_members_dialog.add_btn.enable(true);
			parts.add_members_dialog.search_field.set_value("");
			parts.add_members_dialog.selection_announcer.set_label("");
			{
				let mut s = state.borrow_mut();
				s.add_members_state.group_jid = group_jid;
				s.add_members_state.checked_jids.clear();
			}
			refresh_add_members_contacts_list("", state, parts);
			let payload = serde_json::json!({ "action": "get_contacts" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			parts.add_members_dialog.dialog.show_modal();
		}
		UiCommand::AddGroupMembers(participants) => {
			let group_jid = state.borrow().add_members_state.group_jid.clone();
			if let Some(group_jid) = group_jid {
				if !participants.is_empty() {
					let payload = serde_json::json!({
						"action": "update_group_participants",
						"data": { "jid": group_jid, "participants": participants, "action": "add" }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
			}
		}
		UiCommand::CreateGroup { name, participants } => {
			let payload = serde_json::json!({
				"action": "create_group",
				"data": {
					"name": name,
					"participants": participants
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::NewContact { name, phone } => {
			let payload = serde_json::json!({
				"action": "new_contact",
				"data": {
					"phone": phone,
					"name": name
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RequestQr => {
			state.borrow_mut().is_pairing_active = false;
			let payload = serde_json::json!({
				"action": "get_qr"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CancelQr => {
			state.borrow_mut().is_pairing_active = false;
			let payload = serde_json::json!({
				"action": "cancel_qr"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CancelPairing => {
			state.borrow_mut().is_pairing_active = false;
			let payload = serde_json::json!({
				"action": "cancel_pairing"
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RequestPairing(phone) => {
			state.borrow_mut().is_pairing_active = true;
			let payload = serde_json::json!({
				"action": "pair",
				"data": {
					"phone": phone
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::SendMessage(text) => {
			let mut s = state.borrow_mut();
			if let Some(ref jid) = s.selected_chat.clone() {
				// Sending a reply means the unread messages have obviously been
				// seen, so drop the "N unread messages" divider from the list
				// right away rather than waiting for the next time the chat is
				// opened.
				clear_unread_divider_for_open_chat(&mut s, parts, jid);

				let reply_id = s.pending_reply_id.take().unwrap_or_default();
				let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
				s.pending_reply_preview = None;

				let payload = serde_json::json!({
					"action": "send",
					"data": {
						"to": jid,
						"text": text,
						"reply_to_id": reply_id,
						"reply_to_sender": reply_sender
					}
				});
				let _ = s.ws_tx.send(payload.to_string());
				s.draft_messages.remove(jid);
				parts.conversations_parts.message_field.set_value("");
				parts.conversations_parts.send_message_btn.show(false);
				parts.conversations_parts.remove_quote_btn.show(false);
				parts.conversations_parts.conversation_panel.layout();
			}
		}
		UiCommand::ConversationSelectionChanged(_idx) => {}
		UiCommand::ProcessWebSocketMessage(raw) => {
			if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
				if let Some(t) = val.get("type").and_then(|v| v.as_str()) {
					match t {
						"connected" => {
							// A reconnect (keepalive blip, brief network drop) also
							// fires this event, not just the first connect - don't
							// clobber an in-progress "Syncing history..." status
							// with plain "Connected" just because the socket
							// happened to bounce mid-sync.
							let mid_sync = state
								.borrow()
								.history_sync_last_update
								.map(|t| t.elapsed() < std::time::Duration::from_secs(5))
								.unwrap_or(false);
							if !mid_sync {
								parts.frame.set_status_text("Status: Connected", 0);
							}

							// Emitted on every IPC connect and on every whatsmeow
							// reconnect, not just at startup. Re-running the layout
							// and focus below on a mid-session reconnect is what
							// yanked focus back to the search box.
							let first_show = !state.borrow().main_ui_shown;
							if first_show {
								parts.pairing_dialog.show(false);
								parts.qrcode_dialog.show(false);
								parts.login_panel.show(false);
								parts.nav_panel.show(true);
								parts.content_panel.show(true);
								parts.main_panel.layout();
								parts.frame.show(true);
								parts.conversations_parts.search_field.set_focus();
								state.borrow_mut().main_ui_shown = true;

								// Now that we're logged in, install the tray icon if the
								// user wants it - it's gated on login so an unpaired app
								// never parks an icon in the tray (window.rs).
								install_tray_icon(&state.borrow(), parts);
							}

							let payload = serde_json::json!({
								"action": "get_chats"
							});
							let _ = state.borrow().ws_tx.send(payload.to_string());
						}
						"needs_login" => {
							parts.frame.set_status_text("Status: Not logged in", 0);
							parts.nav_panel.show(false);
							parts.content_panel.show(false);
							parts.main_panel.layout();
							// The login panel is a Dialog owned by the frame (see
							// set_dialog_owner). Windows won't activate an owned window
							// whose owner is hidden, so with the frame no longer shown at
							// startup we reveal it here first - otherwise the login dialog
							// shows but never takes focus and NVDA lands on nothing. A
							// visible owner is what lets the owned dialog activate.
							// Keep the frame hidden so the login dialog is the only top-level
							// window while logged out. That way alt-tabbing to whaNTV activates
							// the dialog itself - a screen reader announces the window and its
							// focused field in one step - instead of landing on an empty frame
							// behind it and needing focus force-shifted into the dialog.
							parts.frame.show(false);
							parts.pairing_dialog.show(false);
							parts.qrcode_dialog.show(false);
							parts.login_panel.show(true);
							parts.login_panel.set_focus();
							state.borrow_mut().main_ui_shown = false;
							// Logged out: pull the tray icon so an unpaired app leaves
							// nothing behind in the tray (reinstalled on connect).
							parts.tray_icon.remove_icon();

							// A "reason" means WhatsApp itself ended the session (phone
							// unlinked it, session expired, etc.) rather than us just
							// having sent our own "Log Out" - worth telling the user
							// why they suddenly landed back on the login screen.
							if let Some(reason) = val.get("data").and_then(|d| d.get("reason")).and_then(|r| r.as_str())
							{
								MessageDialog::builder(
									&parts.frame,
									&format!("whaNTV was logged out of WhatsApp: {reason}"),
									"Logged Out",
								)
								.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconWarning)
								.build()
								.show_modal();
							}
						}
						"pairing_code" => {
							if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
								parts.pairing_code_display.set_value(code);
								println!("{{'data': {{'qrcode': {{'pairingCode': '{}'}}}}}}", code);
								// One setup screen at a time: the pairing-code screen
								// replaces the phone-entry and QR screens.
								parts.login_panel.show(false);
								parts.pairing_dialog.show(true);
								parts.qrcode_dialog.show(false);
								parts.pairing_code_display.select_all();
								parts.pairing_code_display.set_focus();
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_OK,
									);
								}
								live_region::announce(
									parts.pairing_announcer.clone(),
									&format!("Pairing code updated: {}", code),
								);
							}
						}
						"call_connected" => {
							if let Some(data) = val.get("data") {
								let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
								if let Some(audio_port) = data.get("audio_port").and_then(|p| p.as_u64()) {
									let mut s = state.borrow_mut();
									// Unlike call_ended/call_answer_failed, this used to act
									// on any call_connected regardless of call_id - a
									// call_connected arriving after we'd already given up on
									// (or left) that call, e.g. the answer watchdog above,
									// reopened the call UI and started a fresh audio stream
									// for a call the user was already told had ended.
									if s.active_call_id.as_deref() != Some(call_id) {
										return;
									}
									s.call_answer_deadline = None;
									s.incoming_call_caller = None;
									stop_call_ringing_sound(&mut s);
									if let Ok(streams) = call_audio::start_audio_streams(
										audio_port as u16,
										s.settings.input_device_name.clone(),
										s.settings.output_device_name.clone(),
									) {
										s.active_call_audio = Some(streams);
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											"Call connected.",
										);

										if s.call_participants.is_empty() {
											parts.call_parts.status_label.set_label("Call connected.");
										} else {
											parts
												.call_parts
												.status_label
												.set_label(&group_call_status_text(&s.call_participants));
										}
										parts.call_parts.accept_btn.show(false);
										parts.call_parts.reject_btn.show(false);
										parts.call_parts.mute_btn.show(true);
										parts.call_parts.leave_btn.show(true);
										parts.call_parts.add_participant_btn.show(true);
										parts.call_parts.frame.show(true);
									} else {
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											"Failed to start audio stream.",
										);
									}
								}
							}
						}
						"call_placed" => {
							if let Some(data) = val.get("data") {
								if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
									let mut s = state.borrow_mut();
									s.call_placing = false;
									if s.call_pending_cancel {
										// Leave was pressed before we had a call_id to leave with (see
										// call_placing doc comment). Now that the backend has one, hang
										// up immediately instead of leaving the call ringing on the
										// peer's end with nothing on screen to show for it.
										s.call_pending_cancel = false;
										let msg = serde_json::json!({
											"action": "leave_call",
											"data": { "call_id": call_id }
										});
										let _ = s.ws_tx.send(msg.to_string());
									} else {
										s.active_call_id = Some(call_id.to_string());
										s.call_participants.clear();
										start_call_ringing_sound(&mut s, include_bytes!("../sounds/telefone.ogg"));
									}
								}
							}
						}
						"call_participant_added" | "call_participant_add_failed" => {
							if let Some(data) = val.get("data") {
								let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
								if state.borrow().active_call_id.as_deref() != Some(call_id) {
									return;
								}
								let message = if t == "call_participant_added" {
									let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("Contact");
									format!("Ringing {}.", name)
								} else {
									data.get("message")
										.and_then(|m| m.as_str())
										.unwrap_or("Couldn't add that person to the call.")
										.to_string()
								};
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), &message);
							}
						}
						"call_participants" => {
							if let Some(data) = val.get("data") {
								let call_id = data.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
								let mut s = state.borrow_mut();
								if s.active_call_id.as_deref() != Some(call_id) {
									return;
								}
								let participants = parse_call_participants(data);
								// The first roster of a call is the starting line-up (who we're
								// ringing, or who's already in a call we're being invited to) -
								// the status label covers it, so only later changes are announced.
								let changes = if s.call_participants.is_empty() {
									Vec::new()
								} else {
									call_participant_changes(&s.call_participants, &participants)
								};
								s.call_participants = participants;
								// Leave the label alone while an incoming call is still ringing
								// or being answered - it's showing who's calling / "Answering...".
								if s.incoming_call_caller.is_none() && s.call_answer_deadline.is_none() {
									parts
										.call_parts
										.status_label
										.set_label(&group_call_status_text(&s.call_participants));
								}
								drop(s);
								if !changes.is_empty() {
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&changes.join(" "),
									);
								}
							}
						}
						"call_failed" => {
							let mut s = state.borrow_mut();
							if s.call_placing {
								s.call_placing = false;
								s.call_pending_cancel = false;
								parts.call_parts.frame.show(false);
								stop_call_ringing_sound(&mut s);
								play_call_fail_sound(&mut s);
								let message = val
									.get("data")
									.and_then(|d| d.get("message"))
									.and_then(|m| m.as_str())
									.unwrap_or("Failed to place call.");
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), message);
							}
						}
						"call_ended" => {
							if let Some(data) = val.get("data") {
								if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
									let mut s = state.borrow_mut();
									if s.active_call_id.as_deref() == Some(call_id) {
										s.active_call_id = None;
										s.active_call_audio = None;
										s.call_participants.clear();
										s.call_answer_deadline = None;
										parts.call_parts.frame.show(false);
										stop_call_ringing_sound(&mut s);
										play_call_fail_sound(&mut s);
										// Still set here means an incoming call rang out without
										// being answered or rejected. No toast/announcement for
										// this case - missed calls are silent, just clear state.
										if s.incoming_call_caller.take().is_none() {
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												"Call ended.",
											);
										}
									}
								}
							}
						}
						"call_answer_failed" => {
							if let Some(data) = val.get("data") {
								if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
									let mut s = state.borrow_mut();
									if s.active_call_id.as_deref() == Some(call_id) {
										s.active_call_id = None;
										s.active_call_audio = None;
										s.call_participants.clear();
										s.call_answer_deadline = None;
										s.incoming_call_caller = None;
										parts.call_parts.frame.show(false);
										stop_call_ringing_sound(&mut s);
										play_call_fail_sound(&mut s);
										let message = data
											.get("message")
											.and_then(|m| m.as_str())
											.unwrap_or("Failed to answer call.");
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											message,
										);
									}
								}
							}
						}
						"incoming_call" => {
							if let Some(data) = val.get("data") {
								let caller = data.get("caller").and_then(|c| c.as_str()).unwrap_or("Unknown");
								if let Some(call_id) = data.get("call_id").and_then(|c| c.as_str()) {
									let mut s = state.borrow_mut();
									s.active_call_id = Some(call_id.to_string());
									s.call_answer_deadline = None;
									// Prefer the backend-resolved name: the caller JID is
									// typically a @lid, which s.contacts (keyed by phone JID)
									// cannot resolve.
									let name = data
										.get("caller_name")
										.and_then(|n| n.as_str())
										.filter(|n| !n.is_empty() && *n != caller)
										.map(|n| n.to_string())
										.or_else(|| s.contacts.get(caller).cloned())
										.unwrap_or_else(|| caller.to_string());
									s.incoming_call_caller = Some(name.clone());
									s.call_participants.clear();
									let is_group = data.get("is_group").and_then(|g| g.as_bool()).unwrap_or(false);
									let group_name = data
										.get("group_name")
										.and_then(|n| n.as_str())
										.filter(|n| !n.is_empty())
										.map(|n| n.to_string())
										.or_else(|| {
											let group_jid = data.get("group_jid").and_then(|j| j.as_str())?;
											s.display_names
												.get(group_jid)
												.or_else(|| s.contacts.get(group_jid))
												.cloned()
										});
									let description = match (is_group, group_name) {
										(true, Some(group)) => format!("group voice call from {} in {}", name, group),
										(true, None) => format!("group voice call from {}", name),
										(false, _) => format!("voice call from {}", name),
									};
									parts.call_parts.status_label.set_label(&format!("Incoming {}", description));
									parts.call_parts.accept_btn.show(true);
									parts.call_parts.reject_btn.show(true);
									parts.call_parts.mute_btn.show(false);
									parts.call_parts.leave_btn.show(false);
									parts.call_parts.add_participant_btn.show(false);
									// Pop the call dialog to the front the same way a second
									// launch instance activates the main window (see
									// UiCommand::ActivateWindow) - just calling show(true) is not
									// enough to steal focus if the app is minimized or behind
									// other windows.
									parts.call_parts.frame.show(true);
									parts.call_parts.frame.iconize(false);
									parts.call_parts.frame.raise();
									start_call_ringing_sound(
										&mut s,
										include_bytes!("../sounds/ringtone_incoming_call.ogg"),
									);
									if s.settings.notifications_enabled {
										let mut toast_body = description.clone();
										toast_body[..1].make_ascii_uppercase();
										show_toast_notification("Incoming call", &toast_body);
									}
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&format!("Incoming {}", description),
									);
								}
							}
						}
						"qr" => {
							if state.borrow().is_pairing_active {
								return;
							}
							if let Some(code) = val.get("data").and_then(|d| d.get("code")).and_then(|c| c.as_str()) {
								if let Ok(qr) = qrcode::QrCode::new(code) {
									let image =
										qr.render::<image::Rgba<u8>>().quiet_zone(true).module_dimensions(5, 5).build();
									let width = image.width();
									let height = image.height();
									let data = image.into_raw();
									let mut png_bytes = Vec::new();
									let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
									use image::ImageEncoder;
									if encoder
										.write_image(&data, width, height, image::ExtendedColorType::Rgba8)
										.is_ok()
									{
										use base64::{Engine as _, engine::general_purpose};
										let b64 = general_purpose::STANDARD.encode(&png_bytes);
										println!(
											"{{'data': {{'qrcode': {{'base64': 'data:image/png;base64,{}'}}}}}}",
											b64
										);
									}
									if let Some(bitmap) = wxdragon::bitmap::Bitmap::from_rgba(&data, width, height) {
										parts.qrcode_display.set_min_size(Size::new(width as i32, height as i32));
										parts.qrcode_display.set_bitmap(&bitmap);
										parts.qrcode_dialog.fit();
										parts.qrcode_dialog.layout();
									}
								}
								// One setup screen at a time: the QR screen replaces the
								// phone-entry and pairing screens rather than stacking
								// beside them.
								parts.login_panel.show(false);
								parts.pairing_dialog.show(false);
								parts.qrcode_dialog.show(true);
								parts.qrcode_cancel_btn.set_focus();
								unsafe {
									let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
										windows::Win32::UI::WindowsAndMessaging::MB_OK,
									);
								}
								live_region::announce(parts.qrcode_announcer.clone(), "QR code updated");
							}
						}
						"error" => {
							if let Some(msg) = val.get("data").and_then(|d| d.get("message")).and_then(|m| m.as_str()) {
								// The message row itself (if any) is already updated to
								// "failed" by its own "message" event, which the backend
								// always sends before this one; this is just the popup.
								parts.conversations_parts.play_voice_btn.set_label("Play voice message");
								MessageDialog::builder(&parts.frame, msg, "Error")
									.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
									.build()
									.show_modal();
							}
						}
						"connection_state" => {
							let data = val.get("data");
							let conn_state = data.and_then(|d| d.get("state")).and_then(|v| v.as_str()).unwrap_or("");
							let message = data.and_then(|d| d.get("message")).and_then(|v| v.as_str()).unwrap_or("");
							let retry_in = data.and_then(|d| d.get("retry_in")).and_then(|v| v.as_u64());

							let mut s = state.borrow_mut();
							s.conn_online = conn_state == "online";
							// Only speak on the edges of an outage (and for a manual
							// Reconnect Now): a quick keepalive blip that reconnects on
							// the first try says nothing, and retries during a long
							// outage don't repeat the same announcement every time.
							let announcement = match conn_state {
								"online" => {
									let mid_sync = s
										.history_sync_last_update
										.map(|t| t.elapsed() < std::time::Duration::from_secs(5))
										.unwrap_or(false);
									if !mid_sync {
										parts.frame.set_status_text("Status: Connected", 0);
									}
									let speak = s.conn_outage_announced || s.conn_manual_reconnect;
									s.conn_outage_announced = false;
									s.conn_manual_reconnect = false;
									speak.then(|| "Connected to WhatsApp.".to_string())
								}
								"connecting" => {
									let label = if message.is_empty() {
										"Status: Connecting...".to_string()
									} else {
										format!("Status: {message} Reconnecting...")
									};
									parts.frame.set_status_text(&label, 0);
									None
								}
								"offline" => {
									let reason =
										if message.is_empty() { "Can't connect to WhatsApp." } else { message };
									// A temp ban can report its retry hours away, not seconds.
									let label = match retry_in {
										Some(secs @ 1..=119) => {
											format!("Status: Offline. {reason} Retrying in {secs} seconds.")
										}
										Some(secs @ 120..=7199) => {
											format!(
												"Status: Offline. {reason} Retrying in {} minutes.",
												secs.div_ceil(60)
											)
										}
										Some(secs @ 7200..) => {
											format!(
												"Status: Offline. {reason} Retrying in {} hours.",
												secs.div_ceil(3600)
											)
										}
										_ => format!("Status: Offline. {reason} Retrying automatically."),
									};
									parts.frame.set_status_text(&label, 0);
									let speak = !s.conn_outage_announced || s.conn_manual_reconnect;
									s.conn_outage_announced = true;
									s.conn_manual_reconnect = false;
									speak.then(|| format!("{reason} whaNTV will keep trying to reconnect."))
								}
								"stopped" => {
									// The backend already raised an error dialog explaining
									// this one; the status bar just keeps it visible.
									parts.frame.set_status_text(&format!("Status: {message}"), 0);
									s.conn_outage_announced = true;
									s.conn_manual_reconnect = false;
									None
								}
								_ => None,
							};
							drop(s);
							if let Some(text) = announcement {
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), &text);
							}
						}
						"history_sync" => {
							if let Some(data) = val.get("data") {
								let chats_done = data.get("chatsDone").and_then(|v| v.as_i64()).unwrap_or(0);
								let msgs_done = data.get("msgsDone").and_then(|v| v.as_i64()).unwrap_or(0);
								let label = format!(
									"Status: Connected (Syncing history... {msgs_done} messages, {chats_done} chats so far)"
								);
								parts.frame.set_status_text(&label, 0);
								{
									let mut s = state.borrow_mut();
									let now = std::time::Instant::now();
									s.history_sync_last_update = Some(now);
									s.history_sync_armed_until = Some(now + std::time::Duration::from_secs(60));
								}
								if let Some(jids) = data.get("jids").and_then(|j| j.as_array()) {
									let s = state.borrow();
									if s.history_loading {
										if let Some(selected_jid) = s.selected_chat.clone() {
											if jids.iter().any(|j| j.as_str() == Some(&selected_jid)) {
												let payload = serde_json::json!({
													"action": "get_messages",
													"data": { "jid": selected_jid, "limit": 9999999, "offset": 0 }
												});
												let _ = s.ws_tx.send(payload.to_string());
											}
										}
									}
								}
							}
						}
						"chat_lock_verify_result" => {
							if let Some(data) = val.get("data") {
								let success = data.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
								if success {
									let mut s = state.borrow_mut();
									s.chats_unlocked = true;
									s.search_query.clear(); // Clear the search bar
									parts.conversations_parts.search_field.set_value("");
									apply_chat_filter(&mut s, parts);
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										"Locked chats shown.",
									);
								}
							}
						}
						"chats" => {
							if let Some(data) = val.get("data") {
								if let Ok(chats_array) = serde_json::from_value::<Vec<serde_json::Value>>(data.clone())
								{
									let mut chat_list = Vec::new();
									let mut archived_jids = std::collections::HashSet::new();
									let mut community_jids = std::collections::HashSet::new();
									let mut pinned_jids = std::collections::HashSet::new();
									let mut muted_jids = std::collections::HashSet::new();
									let mut admin_group_jids = std::collections::HashSet::new();
									let mut locked_jids = std::collections::HashSet::new();
									let mut unread_counts = std::collections::HashMap::new();
									let mut display_names = std::collections::HashMap::new();
									let mut contacts_map = std::collections::HashMap::new();
									for chat in chats_array {
										if let Some(jid) = chat.get("JID").and_then(|j| j.as_str()) {
											let name = chat
												.get("FullName")
												.and_then(|n| n.as_str())
												.unwrap_or(jid)
												.to_string();
											chat_list.push(jid.to_string());

											let unread = chat.get("UnreadCount").and_then(|u| u.as_u64()).unwrap_or(0);
											let pinned = chat.get("Pinned").and_then(|p| p.as_bool()).unwrap_or(false);
											let muted = chat.get("Muted").and_then(|m| m.as_bool()).unwrap_or(false);
											let archived =
												chat.get("Archived").and_then(|a| a.as_bool()).unwrap_or(false);
											let is_locked =
												chat.get("IsLocked").and_then(|l| l.as_bool()).unwrap_or(false);
											let is_community =
												chat.get("IsCommunity").and_then(|c| c.as_bool()).unwrap_or(false);
											let preview =
												chat.get("PreviewSnippet").and_then(|p| p.as_str()).unwrap_or("");

											let ts = chat.get("Timestamp").and_then(|t| t.as_i64()).unwrap_or(0);
											let preview_sender =
												chat.get("PreviewSender").and_then(|p| p.as_str()).unwrap_or("");
											let preview_from_me =
												chat.get("PreviewFromMe").and_then(|p| p.as_bool()).unwrap_or(false);
											let preview_is_reaction = chat
												.get("PreviewIsReaction")
												.and_then(|p| p.as_bool())
												.unwrap_or(false);
											let preview_reaction_emoji =
												chat.get("PreviewReactionEmoji").and_then(|p| p.as_str()).unwrap_or("");

											let time_str = if ts > 0 {
												if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
													let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
													let now_dt = chrono::Local::now();
													let msg_date = local_dt.date_naive();
													let today = now_dt.date_naive();
													let yesterday = today.pred_opt().unwrap_or(today);
													let num_days_ago = (today - msg_date).num_days();
													if msg_date == today {
														local_dt.format("%I:%M %p").to_string()
													} else if msg_date == yesterday {
														"Yesterday".to_string()
													} else if num_days_ago > 1 && num_days_ago < 7 {
														local_dt.format("%A").to_string()
													} else {
														local_dt.format("%d/%m/%Y").to_string()
													}
												} else {
													String::new()
												}
											} else {
												String::new()
											};

											let mut tags = Vec::new();
											if is_locked {
												tags.push("Locked");
												locked_jids.insert(jid.to_string());
											}
											if archived {
												tags.push("Archived");
												archived_jids.insert(jid.to_string());
											}
											if pinned {
												tags.push("Pinned");
												pinned_jids.insert(jid.to_string());
											}
											if muted {
												tags.push("Muted");
												muted_jids.insert(jid.to_string());
											}
											if is_community {
												tags.push("Community");
												community_jids.insert(jid.to_string());
											}
											if chat.get("IsAdmin").and_then(|a| a.as_bool()).unwrap_or(false) {
												admin_group_jids.insert(jid.to_string());
											}
											unread_counts.insert(jid.to_string(), unread);

											let name_line = if tags.is_empty() {
												name.clone()
											} else {
												format!("{} [{}]", name, tags.join("] ["))
											};

											let unread_line = if unread == 1 {
												"1 unread message".to_string()
											} else if unread > 1 {
												format!("{} unread messages", unread)
											} else {
												String::new()
											};

											let preview_line = if preview_is_reaction
												&& !preview_reaction_emoji.is_empty()
												&& !preview.is_empty()
											{
												let who = if preview_from_me {
													"You".to_string()
												} else if !preview_sender.is_empty() {
													preview_sender.to_string()
												} else {
													"Someone".to_string()
												};
												format!(
													"{} reacted with {} to: {}",
													who,
													preview_reaction_emoji,
													whantv::friendly_preview_text(preview)
												)
											} else if !preview.is_empty() {
												let body = whantv::friendly_preview_text(preview);
												if preview_from_me {
													format!("You: {}", body)
												} else if !preview_sender.is_empty() && jid.ends_with("@g.us") {
													format!("{}: {}", preview_sender, body)
												} else {
													body
												}
											} else {
												String::new()
											};

											let mut parts_vec: Vec<&str> = Vec::new();
											parts_vec.push(&name_line);
											if !unread_line.is_empty() {
												parts_vec.push(&unread_line);
											}
											if !time_str.is_empty() {
												parts_vec.push(&time_str);
											}
											if !preview_line.is_empty() {
												parts_vec.push(&preview_line);
											}
											let display_name = parts_vec.join(" | ");
											display_names.insert(jid.to_string(), display_name);
											contacts_map.insert(jid.to_string(), name);
										}
									}
									let mut s = state.borrow_mut();
									s.chats = chat_list;
									s.contacts = contacts_map;
									s.archived_jids = archived_jids;
									s.locked_jids = locked_jids;
									s.community_jids = community_jids;
									s.pinned_jids = pinned_jids;
									s.muted_jids = muted_jids;
									s.admin_group_jids = admin_group_jids;
									s.unread_counts = unread_counts;
									s.display_names = display_names;
									refresh_tray_tooltip(&s, parts);

									// Compute new filtered list
									let lower_query = s.search_query.to_lowercase();
									let mut new_filtered = Vec::new();
									let mut new_display = Vec::new();
									if s.active_tab != 1 && s.active_tab != 2 {
										for jid in &s.chats {
											if !chat_visible_in_tab(
												s.active_tab,
												jid,
												&s.archived_jids,
												&s.community_jids,
												&s.locked_jids,
												s.chats_unlocked,
											) {
												continue;
											}
											let name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
											if lower_query.is_empty() || name.to_lowercase().contains(&lower_query) {
												let dn = s.display_names.get(jid).cloned().unwrap_or_else(|| name);
												new_filtered.push(jid.clone());
												new_display.push(dn);
											}
										}
									}

									let structure_changed = s.active_tab != 1
										&& s.active_tab != 2 && (new_filtered.len()
										!= s.filtered_chats.len()
										|| new_filtered.iter().zip(s.filtered_chats.iter()).any(|(a, b)| a != b));

									if structure_changed {
										s.filtered_chats = new_filtered;
										drop(s);
										apply_chat_filter(&mut state.borrow_mut(), parts);
									} else {
										// In-place update: just refresh display strings
										parts.conversations_parts.conversations_list.freeze();
										for (i, dn) in new_display.iter().enumerate() {
											parts.conversations_parts.conversations_list.set_string(i as u32, dn);
										}
										parts.conversations_parts.conversations_list.thaw();
										s.filtered_chats = new_filtered;
									}
								}
							}
						}
						"message" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let from = data.get("from").and_then(|f| f.as_str()).unwrap_or("");
								let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");

								let mut s = state.borrow_mut();

								if Some(jid.to_string()) == s.selected_chat {
									let is_from_me = data
										.get("from_me")
										.and_then(|f| f.as_bool())
										.unwrap_or_else(|| from == "me" || from == "You");
									let is_group = jid.ends_with("@g.us");
									let ts = data
										.get("timestamp")
										.and_then(|t| t.as_i64().or_else(|| t.as_f64().map(|f| f as i64)))
										.unwrap_or_else(|| {
											std::time::SystemTime::now()
												.duration_since(std::time::UNIX_EPOCH)
												.unwrap_or_default()
												.as_secs() as i64
										});
									let msg_id = data.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();

									// Matches either the old client-side upload placeholder (by
									// its temp_upload_ prefix) or, now that the backend sends the
									// local echo itself with a real id from the start, a message
									// we already have that's transitioning status (pending/uploading
									// -> sent/failed). Either way this is an update-in-place, not a
									// new row.
									let mut temp_msg_idx = None;
									if is_from_me {
										for (idx, msg) in s.messages.iter().enumerate() {
											if msg.id == msg_id || msg.id.starts_with("temp_upload_") {
												temp_msg_idx = Some(idx);
												break;
											}
										}
									}

									if let Some(idx) = temp_msg_idx {
										s.messages[idx].id = msg_id.clone();
										s.messages[idx].text = text.to_string();
										s.messages[idx].timestamp = ts;
										s.messages[idx].sent_at = ts;
										s.messages[idx].status = data
											.get("status")
											.and_then(|st| st.as_str())
											.filter(|st| !st.is_empty())
											.unwrap_or("sent")
											.to_string();
										s.messages[idx].media_type =
											data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("").to_string();
										s.messages[idx].media_filename = data
											.get("mediaFilename")
											.and_then(|m| m.as_str())
											.unwrap_or("")
											.to_string();
										s.messages[idx].quoted_id = data
											.get("quoted_id")
											.or_else(|| data.get("quotedId"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();
										s.messages[idx].quoted_text = data
											.get("quoted_text")
											.or_else(|| data.get("quotedText"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();
										s.messages[idx].quoted_sender = data
											.get("quoted_sender")
											.or_else(|| data.get("quotedSender"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();

										let status = s.messages[idx].status.clone();
										let media_type = s.messages[idx].media_type.clone();
										let media_filename = s.messages[idx].media_filename.clone();
										let quoted_id = s.messages[idx].quoted_id.clone();
										let quoted_text = s.messages[idx].quoted_text.clone();
										let quoted_sender = s.messages[idx].quoted_sender.clone();

										// Update display_items and UI. Matches by real id first (a
										// message transitioning pending/uploading -> sent/failed);
										// the temp_upload_ prefix check is kept only for backward
										// compatibility with any leftover client-side placeholder.
										let mut target_di = None;
										let mut updated_display = String::new();
										for (di, item) in s.display_items.iter_mut().enumerate() {
											let mut matched = false;
											match item {
												whantv::DisplayItem::Single(m) => {
													if m.id == msg_id || m.id.starts_with("temp_upload_") {
														m.id = msg_id.clone();
														m.text = text.to_string();
														m.timestamp = ts;
														m.sent_at = ts;
														m.status = status.clone();
														m.media_type = media_type.clone();
														m.media_filename = media_filename.clone();
														m.quoted_id = quoted_id.clone();
														m.quoted_text = quoted_text.clone();
														m.quoted_sender = quoted_sender.clone();
														matched = true;
													}
												}
												whantv::DisplayItem::Album { messages, .. } => {
													for m in messages.iter_mut() {
														if m.id == msg_id || m.id.starts_with("temp_upload_") {
															m.id = msg_id.clone();
															m.text = text.to_string();
															m.timestamp = ts;
															m.sent_at = ts;
															m.status = status.clone();
															m.media_type = media_type.clone();
															m.media_filename = media_filename.clone();
															m.quoted_id = quoted_id.clone();
															m.quoted_text = quoted_text.clone();
															m.quoted_sender = quoted_sender.clone();
															matched = true;
															break;
														}
													}
												}
											}
											if matched {
												target_di = Some(di);
												updated_display = item.format_display(is_group);
												break;
											}
										}

										if let Some(di) = target_di {
											parts
												.conversations_parts
												.messages_list
												.set_string(di as u32, &updated_display);
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												&updated_display,
											);
											if parts.conversations_parts.messages_list.get_selection()
												== Some(di as u32)
											{
												refresh_voice_action_button(&s, &parts.conversations_parts);
											}
										}
									} else {
										let sender_name = if is_from_me {
											"You".to_string()
										} else {
											let mut clean_jid = from.to_string();
											if let Some(idx) = from.find(':') {
												clean_jid = format!("{}@s.whatsapp.net", &from[..idx]);
											}
											data.get("senderName")
												.and_then(|n| n.as_str())
												.filter(|n| !n.is_empty())
												.map(|n| n.to_string())
												.or_else(|| s.contacts.get(&clean_jid).cloned())
												.or_else(|| s.contacts.get(from).cloned())
												.unwrap_or(clean_jid)
										};

										let mut insert_date = false;
										let mut divider_label = String::new();
										if let Some(datetime) = chrono::DateTime::from_timestamp(ts, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											let day = whantv::date_key(local_datetime);
											if let Some(last_msg) = s.messages.last() {
												if !last_msg.is_date_divider {
													if let Some(last_dt) =
														chrono::DateTime::from_timestamp(last_msg.timestamp, 0)
													{
														let last_local: chrono::DateTime<chrono::Local> =
															last_dt.into();
														if day != whantv::date_key(last_local) {
															insert_date = true;
														}
													}
												}
											} else {
												insert_date = true;
											}
											if insert_date {
												divider_label = whantv::format_date_divider_label(local_datetime);
											}
										}

										let quoted_id = data
											.get("quoted_id")
											.or_else(|| data.get("quotedId"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();
										let mut quoted_text = data
											.get("quoted_text")
											.or_else(|| data.get("quotedText"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();
										let mut quoted_sender = data
											.get("quoted_sender")
											.or_else(|| data.get("quotedSender"))
											.and_then(|q| q.as_str())
											.unwrap_or("")
											.to_string();

										// The live push event doesn't always carry the quote preview
										// (e.g. server-side lookup races). Backfill it from the
										// already-loaded chat so reply styling doesn't depend on
										// closing and reopening the chat to pick it up on next load.
										if !quoted_id.is_empty() && (quoted_text.is_empty() || quoted_sender.is_empty())
										{
											if let Some(original) = s.messages.iter().find(|m| m.id == quoted_id) {
												if quoted_text.is_empty() {
													quoted_text = if original.is_audio
														|| original.text.starts_with("[Voice Message]")
													{
														"[Voice Message]".to_string()
													} else if original.text.is_empty() {
														"[Media]".to_string()
													} else {
														original.text.clone()
													};
												}
												if quoted_sender.is_empty() {
													quoted_sender = if original.from_me {
														"me".to_string()
													} else if !original.sender_name.is_empty() {
														original.sender_name.clone()
													} else {
														original.sender_jid.clone()
													};
												}
											}
										}

										if insert_date && !divider_label.is_empty() {
											let divider = ChatMessage {
												id: String::new(),
												text: divider_label,
												timestamp: ts,
												is_date_divider: true,
												..Default::default()
											};
											s.messages.push(divider.clone());
											s.display_items.push(whantv::DisplayItem::Single(divider.clone()));
											parts
												.conversations_parts
												.messages_list
												.append(&divider.format_display(is_group));
										}

										let chat_msg = ChatMessage {
											id: msg_id.clone(),
											text: text.to_string(),
											timestamp: ts,
											from_me: is_from_me,
											sender_jid: from.to_string(),
											sender_name,
											is_audio: text.starts_with("[Voice Message]")
												|| text.starts_with("[Audio]"),
											// The backend now tells us the real status up front for our
											// own sends (pending/uploading/sent/failed); fall back to an
											// out-of-order receipt, then "sent", for anything else.
											status: data
												.get("status")
												.and_then(|st| st.as_str())
												.filter(|st| !st.is_empty())
												.map(|st| st.to_string())
												.or_else(|| s.pending_receipts.remove(&msg_id))
												.unwrap_or_else(|| "sent".to_string()),
											sent_at: if is_from_me { ts } else { 0 },
											quoted_id,
											quoted_text,
											quoted_sender,
											media_type: data
												.get("mediaType")
												.and_then(|m| m.as_str())
												.unwrap_or("")
												.to_string(),
											media_filename: data
												.get("mediaFilename")
												.and_then(|m| m.as_str())
												.unwrap_or("")
												.to_string(),
											album_id: data
												.get("albumId")
												.and_then(|m| m.as_str())
												.unwrap_or("")
												.to_string(),
											..Default::default()
										};
										s.messages.push(chat_msg.clone());

										// Incremental display_items update instead of full O(n) regrouping
										let is_deleted_msg = chat_msg.is_deleted || chat_msg.text.contains("deleted");
										let is_media_msg = !is_deleted_msg
											&& (chat_msg.media_type == "image"
												|| chat_msg.media_type == "video" || chat_msg
												.text
												.starts_with("[Photo]") || chat_msg.text.starts_with("[Image]")
												|| chat_msg.text.starts_with("[Video]"));

										let should_merge = if is_media_msg {
											if let Some(last_item) = s.display_items.last() {
												match last_item {
													whantv::DisplayItem::Single(prev_msg) => {
														let prev_is_media =
															!prev_msg.is_deleted
																&& !prev_msg.text.contains("deleted") && (prev_msg
																.media_type == "image"
																|| prev_msg.media_type == "video"
																|| prev_msg.text.starts_with("[Photo]")
																|| prev_msg.text.starts_with("[Image]")
																|| prev_msg.text.starts_with("[Video]"));
														prev_is_media && whantv::same_album(prev_msg, &chat_msg)
													}
													whantv::DisplayItem::Album { messages, .. } => messages
														.last()
														.is_some_and(|m| whantv::same_album(m, &chat_msg)),
												}
											} else {
												false
											}
										} else {
											false
										};

										if should_merge {
											// Merging into the previous item only changes that one
											// row's label (e.g. "2 photos" -> "3 photos") - update it
											// in place instead of regrouping and re-appending the
											// whole (unvirtualized) list.
											let last_idx = s.display_items.len() - 1;
											let merged = match s.display_items.pop().unwrap() {
												whantv::DisplayItem::Single(prev_msg) => whantv::DisplayItem::Album {
													id: prev_msg.id.clone(),
													sender_jid: prev_msg.sender_jid.clone(),
													sender_name: prev_msg.sender_name.clone(),
													from_me: prev_msg.from_me,
													timestamp: prev_msg.timestamp,
													messages: vec![prev_msg, chat_msg.clone()],
												},
												whantv::DisplayItem::Album {
													id,
													sender_jid,
													sender_name,
													from_me,
													timestamp,
													mut messages,
												} => {
													messages.push(chat_msg.clone());
													whantv::DisplayItem::Album {
														id,
														sender_jid,
														sender_name,
														from_me,
														timestamp,
														messages,
													}
												}
											};
											parts
												.conversations_parts
												.messages_list
												.set_string(last_idx as u32, &merged.format_display(is_group));
											s.display_items.push(merged);
										} else {
											s.display_items.push(whantv::DisplayItem::Single(chat_msg.clone()));
											parts
												.conversations_parts
												.messages_list
												.append(&chat_msg.format_display(is_group));
										}
										refresh_selection_list(&mut s, parts);

										let count = parts.conversations_parts.messages_list.get_count();
										if count > 0 && is_from_me {
											parts
												.conversations_parts
												.messages_list
												.set_selection((count - 1) as u32, true);
											refresh_voice_action_button(&s, &parts.conversations_parts);
										}
										let formatted = chat_msg.format_display(is_group);
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&formatted,
										);

										if !is_from_me {
											let sounds_enabled =
												s.settings.sounds_enabled && !s.muted_jids.contains(jid);
											if s.window_focused {
												let mark_jid = jid.to_string();
												let mark_msg_id = msg_id.clone();
												let tx = s.ws_tx.clone();
												let payload = serde_json::json!({
													"action": "mark_read",
													"data": { "jid": mark_jid, "message_ids": vec![mark_msg_id] }
												});
												let _ = tx.send(payload.to_string());
											} else {
												// The window isn't focused, so the user hasn't actually
												// seen this yet — don't send a read receipt. Queue it
												// for a batched mark_read once focus returns, and show
												// (or grow) an "N unread messages" divider right now so
												// it's already there when they look back at the chat —
												// updated/inserted in place rather than rebuilding the
												// whole list, so messages just quietly stack up in the
												// background like the official WhatsApp client.
												s.background_unread_message_ids.push(msg_id.clone());
												let is_group = jid.ends_with("@g.us");
												let existing_di = s.display_items.iter().position(
													|item| matches!(item, whantv::DisplayItem::Single(m) if m.is_unread_divider),
												);
												if let Some(di) = existing_di {
													s.unread_divider_count += 1;
													let count = s.unread_divider_count;
													let label = if count == 1 {
														"1 unread message".to_string()
													} else {
														format!("{} unread messages", count)
													};
													if let Some(m) = s.messages.iter_mut().find(|m| m.is_unread_divider)
													{
														m.text = label.clone();
													}
													if let whantv::DisplayItem::Single(m) = &mut s.display_items[di] {
														m.text = label.clone();
													}
													parts
														.conversations_parts
														.messages_list
														.set_string(di as u32, &label);
												} else {
													s.unread_divider_jid = Some(jid.to_string());
													s.unread_divider_count = 1;
													// The message that just triggered this branch was
													// appended as the last entry above, so the divider
													// belongs immediately before it.
													let divider = ChatMessage {
														text: "1 unread message".to_string(),
														is_unread_divider: true,
														..Default::default()
													};
													let insert_at = s.messages.len().saturating_sub(1);
													s.messages.insert(insert_at, divider.clone());
													let di_insert_at = s.display_items.len().saturating_sub(1);
													s.display_items.insert(
														di_insert_at,
														whantv::DisplayItem::Single(divider.clone()),
													);
													parts
														.conversations_parts
														.messages_list
														.insert(&divider.format_display(is_group), di_insert_at);
												}
											}
											play_notification_sound(NotificationSound::MessageCurrent, sounds_enabled);
										}
									}
								} else {
									let is_ignored = jid == "status@broadcast"
										|| jid.ends_with("@broadcast") || jid.ends_with("@newsletter");
									if !is_ignored {
										let is_from_me = data
											.get("from_me")
											.and_then(|f| f.as_bool())
											.unwrap_or_else(|| from == "me" || from == "You");
										if !is_from_me {
											// Archived chats default to silent and can be dialled back up per chat
											// from the chat-list context menu; muting still overrides both.
											let alert = whantv::incoming_alert(
												s.muted_jids.contains(jid),
												s.archived_jids.contains(jid),
												s.settings.archived_alert(jid),
											);
											if alert.sound {
												play_notification_sound(
													NotificationSound::MessageBackground,
													s.settings.sounds_enabled,
												);
											}
											if alert.toast && s.settings.notifications_enabled {
												// The backend's "message" event carries no sender_name
												// field - resolve the display name from the contacts
												// map ourselves, same as everywhere else in the UI, so
												// the notification doesn't show a raw JID.
												let body = whantv::friendly_preview_text(text);
												let is_group = jid.ends_with("@g.us");
												// `from` can carry a WhatsApp device suffix (e.g.
												// "1234:12@s.whatsapp.net") which will not match the
												// contacts map keyed by the plain JID - strip it, same
												// as the in-window sender resolution above.
												let clean_from = match from.find(':') {
													Some(idx) => format!("{}@s.whatsapp.net", &from[..idx]),
													None => from.to_string(),
												};
												let provided_sender = data
													.get("senderName")
													.and_then(|n| n.as_str())
													.filter(|n| !n.is_empty())
													.map(|n| n.to_string());
												let provided_chat_name = data
													.get("chatName")
													.and_then(|n| n.as_str())
													.filter(|n| !n.is_empty())
													.map(|n| n.to_string());
												if is_group {
													let mut chat_name = provided_chat_name
														.or_else(|| s.contacts.get(jid).cloned())
														.unwrap_or_else(|| jid.to_string());
													if chat_name == *jid {
														chat_name = "New Group".to_string();
													}
													let sender_name = provided_sender
														.or_else(|| s.contacts.get(&clean_from).cloned())
														.or_else(|| s.contacts.get(from).cloned())
														.unwrap_or_else(|| display_name_for_jid(&clean_from));
													let toast_body = if sender_name.is_empty() {
														body.to_string()
													} else {
														format!("{}: {}", sender_name, body)
													};
													if let Some(n) = show_toast_notification(&chat_name, &toast_body) {
														s.active_notifications
															.entry(jid.to_string())
															.or_default()
															.push(n);
													}
												} else {
													let sender_name = provided_sender
														.or_else(|| s.contacts.get(&clean_from).cloned())
														.or_else(|| s.contacts.get(from).cloned())
														.unwrap_or_else(|| display_name_for_jid(&clean_from));
													if let Some(n) = show_toast_notification(&sender_name, &body) {
														s.active_notifications
															.entry(jid.to_string())
															.or_default()
															.push(n);
													}
												}
											}
										}
									}
								}
							}
						}
						"message_deleted" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let message_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								let for_everyone = data.get("for_everyone").and_then(|f| f.as_bool()).unwrap_or(true);

								let mut s = state.borrow_mut();
								// A bulk delete (selection mode) fires one "message_deleted"
								// event per message; announcing each individually would spam
								// a screen reader with "Message deleted" N times back to
								// back, on top of the one summary announcement
								// DeleteSelectedMessages already made - suppress the
								// per-message announce while draining that batch's set.
								// Keyed by (jid, message_id) so an unrelated delete arriving
								// mid-batch (e.g. synced from another linked device) isn't
								// mistaken for part of this batch.
								let suppress_announce =
									s.bulk_delete_pending.remove(&(jid.to_string(), message_id.to_string()));
								if Some(jid.to_string()) == s.selected_chat {
									let is_group = jid.ends_with("@g.us");

									if !for_everyone {
										// "Delete for me": the row just disappears, like
										// official WhatsApp - no "This message was
										// deleted" tombstone, that's only for revokes.
										let had_message = s.messages.iter().any(|m| m.id == message_id);
										if had_message {
											s.messages.retain(|m| m.id != message_id);
											s.display_items = whantv::group_messages_into_display_items(&s.messages);
											s.prev_msg_count = s.display_items.len();

											parts.conversations_parts.messages_list.freeze();
											parts.conversations_parts.messages_list.clear();
											for item in &s.display_items {
												parts
													.conversations_parts
													.messages_list
													.append(&item.format_display(is_group));
											}
											parts.conversations_parts.messages_list.thaw();
											refresh_selection_list(&mut s, parts);
											let count = parts.conversations_parts.messages_list.get_count();
											if count > 0 {
												parts.conversations_parts.messages_list.set_selection(count - 1, true);
											}
											refresh_voice_action_button(&s, &parts.conversations_parts);
											if !suppress_announce {
												live_region::announce(
													parts.qrcode_announcer.clone(),
													"Message deleted.",
												);
											}
										}
										return;
									}

									let mut changed = false;
									for msg in s.messages.iter_mut() {
										if msg.id == message_id {
											msg.is_deleted = true;
											msg.text = "".to_string();
											changed = true;
											break;
										}
									}
									if changed {
										// Update display_items in-place instead of full O(n) regrouping
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.id == message_id => {
													m.is_deleted = true;
													m.text = String::new();
													break;
												}
												whantv::DisplayItem::Album { messages, .. } => {
													if let Some(am) = messages.iter_mut().find(|am| am.id == message_id)
													{
														am.is_deleted = true;
														am.text = String::new();
														break;
													}
												}
												_ => {}
											}
										}
										for (di, item) in s.display_items.iter().enumerate() {
											let contains = match item {
												whantv::DisplayItem::Single(m) => m.id == message_id,
												whantv::DisplayItem::Album { messages, .. } => {
													messages.iter().any(|m| m.id == message_id)
												}
											};
											if contains {
												parts
													.conversations_parts
													.messages_list
													.set_string(di as u32, &item.format_display(is_group));

												if let Some(sel_idx) =
													parts.conversations_parts.messages_list.get_selection()
												{
													if sel_idx as u32 == di as u32 {
														let mut need_layout = crate::update_action_buttons(
															&s,
															&parts.conversations_parts,
														);
														if parts.conversations_parts.play_voice_btn.is_shown() {
															parts.conversations_parts.play_voice_btn.show(false);
															need_layout = true;
														}
														if parts.conversations_parts.audio_slider.is_shown() {
															parts.conversations_parts.audio_slider.show(false);
															parts.conversations_parts.audio_progress_label.show(false);
															parts.conversations_parts.audio_speed_btn.show(false);
															need_layout = true;
														}
														if need_layout {
															parts.conversations_parts.action_panel.layout();
															parts.conversations_parts.conversation_panel.layout();
														}
													}
												}
												break;
											}
										}
										if !suppress_announce {
											live_region::announce(parts.qrcode_announcer.clone(), "Message deleted.");
										}
									}
								}
							}
						}
						"new_contact_status" => {
							if let Some(data) = val.get("data") {
								if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
									if status == "success" {
										let payload = serde_json::json!({
											"action": "get_chats"
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									} else if status == "error" {
										let err_msg =
											data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
												windows::Win32::UI::WindowsAndMessaging::MB_OK,
											);
										}
										MessageDialog::builder(&parts.new_contact_dialog.dialog, err_msg, "Error")
											.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
											.build()
											.show_modal();
									}
								}
							}
						}
						"contacts" => {
							if let Ok(rows) = serde_json::from_value::<Vec<serde_json::Value>>(
								val.get("data").cloned().unwrap_or(serde_json::Value::Null),
							) {
								let mut contacts: Vec<Contact> = rows
									.iter()
									.filter_map(|row| {
										let jid = row.get("jid").and_then(|j| j.as_str())?.to_string();
										if jid.is_empty() {
											return None;
										}
										let name = row.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
										let phone = row.get("phone").and_then(|p| p.as_str()).unwrap_or("").to_string();
										Some(Contact { jid, name, phone })
									})
									.collect();
								contacts.sort_by(|a, b| {
									let ka = if a.name.is_empty() { &a.phone } else { &a.name };
									let kb = if b.name.is_empty() { &b.phone } else { &b.name };
									ka.to_lowercase().cmp(&kb.to_lowercase())
								});
								let mut s = state.borrow_mut();
								s.all_contacts = contacts;
								// Refresh the pickers live if they're already open and the
								// backend response landed after the dialog was shown
								// with a possibly-empty or stale contact list.
								let new_conv_shown = parts.new_conv_dialog.dialog.is_shown();
								let new_group_shown = parts.new_group_dialog.dialog.is_shown();
								let add_members_shown = parts.add_members_dialog.dialog.is_shown();
								drop(s);
								if new_conv_shown {
									let query = parts.new_conv_dialog.search_field.get_value();
									apply_contact_picker_filter(&query, state, parts);
								}
								// Route a fresh `contacts` response through the same debounce
								// timer as keystrokes (rather than rebuilding the checklist
								// widget here immediately) - back-to-back rebuilds (this one
								// landing right after the dialog-open rebuild) race NVDA's
								// async accessibility queries and trip wx's "bad
								// wxCheckListBox index" assert. See the timer's own comment
								// in window.rs's bind_input_handlers.
								if new_group_shown {
									sync_new_group_contact_labels(state);
									parts.new_group_contacts_timer.stop();
									parts.new_group_contacts_timer.start(250, true);
								}
								if add_members_shown {
									sync_add_members_contact_labels(state);
									parts.add_members_contacts_timer.stop();
									parts.add_members_contacts_timer.start(250, true);
								}
							}
						}
						"create_group_status" | "new_group_status" => {
							if let Some(data) = val.get("data") {
								if let Some(status) = data.get("status").and_then(|s| s.as_str()) {
									if status == "success" {
										let payload = serde_json::json!({
											"action": "get_chats"
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									} else if status == "error" {
										parts.new_group_dialog.ok_btn.enable(true);
										let err_msg =
											data.get("error").and_then(|e| e.as_str()).unwrap_or("Unknown error");
										unsafe {
											let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
												windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE(0x00000030),
											);
										}
										MessageDialog::builder(&parts.new_group_dialog.dialog, err_msg, "Error")
											.with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError)
											.build()
											.show_modal();
									}
								}
							}
						}
						"conversation_messages" => {
							if let Some(data) = val.get("data") {
								let response_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								// The chat may have been switched again while this
								// request was in flight - applying a stale response
								// would silently populate the wrong chat's history.
								if state.borrow().selected_chat.as_deref() != Some(response_jid) {
									return;
								}
								if let Some(messages) = data.get("messages").and_then(|m| m.as_array()) {
									let mut parsed_msgs = Vec::new();
									for msg in messages {
										match serde_json::from_value::<ChatMessage>(msg.clone()) {
											Ok(chat_msg) => parsed_msgs.push(chat_msg),
											Err(e) => {
												let err_msg = ChatMessage {
													text: format!("Parse error: {} for json: {}", e, msg),
													..Default::default()
												};
												parsed_msgs.push(err_msg);
											}
										}
									}
									let mut seen_ids = std::collections::HashSet::new();
									let mut deduplicated = Vec::new();
									for msg in parsed_msgs {
										if !msg.id.is_empty() {
											if seen_ids.insert(msg.id.clone()) {
												deduplicated.push(msg);
											}
										} else {
											deduplicated.push(msg);
										}
									}

									let mut s = state.borrow_mut();
									let chat_jid = s.selected_chat.clone().unwrap_or_default();
									let chat_name =
										s.contacts.get(&chat_jid).cloned().unwrap_or_else(|| chat_jid.clone());
									let is_group_chat = chat_jid.ends_with("@g.us");

									let mut with_dates = Vec::new();
									let mut last_date: Option<chrono::NaiveDate> = None;
									for mut msg in deduplicated {
										if !is_group_chat
											&& !msg.from_me && msg.sender_name.is_empty()
											&& msg.sender_jid.is_empty()
										{
											msg.sender_name = chat_name.clone();
										}

										if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											let day = whantv::date_key(local_datetime);
											if last_date != Some(day) {
												last_date = Some(day);
												with_dates.push(ChatMessage {
													text: whantv::format_date_divider_label(local_datetime),
													timestamp: msg.timestamp,
													is_date_divider: true,
													..Default::default()
												});
											}
										}
										with_dates.push(msg);
									}

									if s.unread_divider_jid.as_deref() == Some(chat_jid.as_str()) {
										whantv::insert_unread_divider(&mut with_dates, s.unread_divider_count);
									}

									s.loading_messages = false;
									let was_history_loading = s.history_loading;
									s.history_loading = false;
									let old_count = s.prev_msg_count;
									let is_group =
										s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
									s.messages = with_dates;
									// Apply any receipts that arrived before messages finished loading.
									// Collect first to avoid simultaneous mutable borrows of s.
									let pending_ids: Vec<(String, String)> = s
										.messages
										.iter()
										.filter_map(|msg| {
											s.pending_receipts.get(&msg.id).map(|st| (msg.id.clone(), st.clone()))
										})
										.collect();
									for (id, st) in pending_ids {
										s.pending_receipts.remove(&id);
										if let Some(msg) = s.messages.iter_mut().find(|m| m.id == id) {
											if get_status_rank(&st) >= get_status_rank(&msg.status) {
												msg.status = st;
											}
										}
									}
									s.display_items = whantv::group_messages_into_display_items(&s.messages);
									let new_len = s.display_items.len();
									s.prev_msg_count = new_len;

									parts.conversations_parts.messages_list.freeze();
									parts.conversations_parts.messages_list.clear();
									for item in &s.display_items {
										parts.conversations_parts.messages_list.append(&item.format_display(is_group));
									}
									parts.conversations_parts.messages_list.thaw();
									refresh_selection_list(&mut s, parts);

									let count = parts.conversations_parts.messages_list.get_count();
									let target_pos = if let Some(target_id) = s.search_target_id.take() {
										s.display_items.iter().position(|item| match item {
											whantv::DisplayItem::Single(m) => m.id == target_id,
											whantv::DisplayItem::Album { messages, .. } => {
												messages.iter().any(|m| m.id == target_id)
											}
										})
									} else if let Some(target_text) = s.search_target_text.take() {
										let is_generic = target_text == "[Voice Message]"
											|| target_text == "[Audio]" || target_text == "Voice message"
											|| target_text == "Audio" || target_text == "[Media]";
										if is_generic {
											None
										} else {
											s.display_items.iter().position(|item| match item {
												whantv::DisplayItem::Single(m) => {
													m.text == target_text
														|| (!m.text.is_empty() && m.text.contains(&target_text))
												}
												whantv::DisplayItem::Album { messages, .. } => {
													messages.iter().any(|m| {
														m.text == target_text
															|| (!m.text.is_empty() && m.text.contains(&target_text))
													})
												}
											})
										}
									} else if old_count == 0
										&& s.unread_divider_jid.as_deref() == Some(chat_jid.as_str())
									{
										// Opening a chat with unread messages: land on the
										// "N unread messages" divider, same as pressing Enter
										// on it from the chat list, instead of the very bottom.
										s.display_items.iter().position(
											|item| matches!(item, whantv::DisplayItem::Single(m) if m.is_unread_divider),
										)
									} else {
										None
									};
									if let Some(pos) = target_pos {
										parts.conversations_parts.messages_list.set_selection(pos as u32, true);
										parts.conversations_parts.messages_list.set_focus();
									} else if count > 0 {
										if was_history_loading && new_len > old_count && old_count > 0 {
											let keep_idx = (new_len - old_count) as u32;
											if keep_idx < count {
												parts.conversations_parts.messages_list.set_selection(keep_idx, true);
											} else {
												parts.conversations_parts.messages_list.set_selection(0, true);
											}
										} else if old_count == 0 {
											parts
												.conversations_parts
												.messages_list
												.set_selection((count - 1) as u32, true);
											parts.conversations_parts.messages_list.set_focus();
										}
									}
									parts.conversations_parts.conversation_panel.freeze();
									if update_action_buttons(&s, &parts.conversations_parts) {
										parts.conversations_parts.conversation_panel.layout();
									}
									parts.conversations_parts.conversation_panel.thaw();
									refresh_voice_action_button(&s, &parts.conversations_parts);

									if was_history_loading && new_len == old_count {
										if let Some(ref jid) = s.selected_chat.clone() {
											eprintln!(
												"[LOG UI] No new history messages received for {}. Marking history complete.",
												jid
											);
											s.completed_history_chats.insert(jid.clone());
										}
									}
								}
							}
						}
						"no_more_history" => {
							let mut s = state.borrow_mut();
							s.history_loading = false;
							if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
								if !jid.is_empty() {
									eprintln!("[LOG UI] End of history reached on phone for chat: {}", jid);
									s.completed_history_chats.insert(jid.to_string());
								}
							}
						}
						"play_audio_failed" => {
							eprintln!("[LOG UI] Voice message audio failed to load or download");
							live_region::announce(
								parts.conversations_parts.messages_announcer.clone(),
								"Could not load voice message",
							);
						}
						"message_info" => {
							if let Some(data) = val.get("data") {
								let fmt_ts = |ts: i64| -> String {
									if ts == 0 {
										return String::new();
									}
									let dt = chrono::DateTime::from_timestamp(ts, 0)
										.unwrap_or_default()
										.with_timezone(&chrono::Local);
									dt.format("%l:%M %p").to_string().trim().to_lowercase()
								};
								let msg_id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
								let mut sent_s = data.get("sent_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut delivered_s = data.get("delivered_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut read_s = data.get("read_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut played_s = data.get("played_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let mut edited_s = data.get("edited_at").and_then(|v| v.as_i64()).unwrap_or(0);
								let media_type = data.get("media_type").and_then(|v| v.as_str()).unwrap_or("");
								let is_voice = media_type == "voice" || media_type == "audio";

								{
									let s = state.borrow();
									if let Some(msg) = s.messages.iter().find(|m| m.id == msg_id) {
										if sent_s == 0 {
											sent_s = if msg.sent_at > 0 { msg.sent_at } else { msg.timestamp };
										}
										if delivered_s == 0 && msg.delivered_at > 0 {
											delivered_s = msg.delivered_at;
										}
										if read_s == 0 && msg.read_at > 0 {
											read_s = msg.read_at;
										}
										if played_s == 0 && msg.played_at > 0 {
											played_s = msg.played_at;
										}
										if edited_s == 0 && msg.edited_at > 0 {
											edited_s = msg.edited_at;
										}
									}
								}

								let mut lines = Vec::new();
								let sent_fmt = fmt_ts(sent_s);
								if !sent_fmt.is_empty() {
									lines.push(format!("Sent at {}", sent_fmt));
								}
								let del_fmt = fmt_ts(delivered_s);
								if !del_fmt.is_empty() {
									lines.push(format!("Delivered at {}", del_fmt));
								}
								let read_fmt = fmt_ts(read_s);
								if !read_fmt.is_empty() {
									lines.push(format!("Read at {}", read_fmt));
								}
								if is_voice {
									let played_fmt = fmt_ts(played_s);
									if !played_fmt.is_empty() {
										lines.push(format!("Played at {}", played_fmt));
									}
								}
								let edited_fmt = fmt_ts(edited_s);
								if !edited_fmt.is_empty() {
									lines.push(format!("Edited at {}", edited_fmt));
								}

								if let Some(participants) = data.get("participants").and_then(|p| p.as_array()) {
									if !participants.is_empty() {
										lines.retain(|l| l.starts_with("Sent at") || l.starts_with("Edited at"));

										let mut played = Vec::new();
										let mut read = Vec::new();
										let mut delivered = Vec::new();
										let mut sent = Vec::new();
										for p in participants {
											let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
											let pstatus = p.get("status").and_then(|v| v.as_str()).unwrap_or("sent");
											let pts = p.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
											let p_at = |key: &str| p.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
											// Every state this member has reached, latest first, e.g.
											// "Asha — read 8:13 pm, delivered 8:07 pm".
											let mut times = Vec::new();
											if is_voice {
												let played = fmt_ts(p_at("played_at"));
												if !played.is_empty() {
													times.push(format!("played {}", played));
												}
											}
											let read_at = fmt_ts(p_at("read_at"));
											if !read_at.is_empty() {
												times.push(format!("read {}", read_at));
											}
											let delivered_at = fmt_ts(p_at("delivered_at"));
											if !delivered_at.is_empty() {
												times.push(format!("delivered {}", delivered_at));
											}
											if times.is_empty() {
												let ts_fmt = fmt_ts(pts);
												if !ts_fmt.is_empty() {
													times.push(ts_fmt);
												}
											}
											let entry = if times.is_empty() {
												name.to_string()
											} else {
												format!("{} — {}", name, times.join(", "))
											};
											match pstatus {
												"played" => played.push(entry),
												"read" => read.push(entry),
												"delivered" => delivered.push(entry),
												_ => sent.push(entry),
											}
										}

										let mut push_group = |label: &str, entries: &Vec<String>| {
											if entries.is_empty() {
												return;
											}
											lines.push(String::new());
											lines.push(label.to_string());
											for e in entries {
												lines.push(format!("  {}", e));
											}
										};
										push_group("Played by", &played);
										push_group("Read by", &read);
										push_group("Delivered to", &delivered);
										push_group("Sent to", &sent);
									}
								}

								if lines.is_empty() {
									lines.push("No receipt information available.".to_string());
								}
								let info_text = lines.join("\n");

								let dlg = Dialog::builder(&parts.frame, "Message Info")
									.with_style(DialogStyle::DefaultDialogStyle)
									.build();
								let sizer = BoxSizer::builder(Orientation::Vertical).build();
								let txt = TextCtrl::builder(&dlg)
									.with_value(&info_text)
									.with_style(
										TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap,
									)
									.build();
								sizer.add(&txt, 1, SizerFlag::Expand | SizerFlag::All, 10);
								let ok_btn = Button::builder(&dlg).with_label("OK").build();
								sizer.add(&ok_btn, 0, SizerFlag::AlignCentre | SizerFlag::Bottom | SizerFlag::All, 8);
								dlg.set_sizer_and_fit(sizer, true);
								let dlg_close = dlg.clone();
								ok_btn.on_click(move |_| {
									dlg_close.end_modal(wxdragon::id::ID_OK);
								});
								dlg.show_modal();
							}
						}
						"history_updated" => {
							eprintln!(
								"[LOG UI] History sync completed on phone! Refreshing messages for active chat..."
							);
							let s = state.borrow();
							if let Some(ref jid) = s.selected_chat.clone() {
								let limit = 9999999;
								let payload = serde_json::json!({
									"action": "get_messages",
									"data": { "jid": jid, "limit": limit, "offset": 0 }
								});
								let _ = s.ws_tx.send(payload.to_string());
							}
						}
						"export_progress" => {
							if let Some(data) = val.get("data") {
								let current = data.get("current").and_then(|c| c.as_i64()).unwrap_or(0);
								let total = data.get("total").and_then(|t| t.as_i64()).unwrap_or(0);
								parts
									.frame
									.set_status_text(&format!("Status: Exporting chat... {}/{}", current, total), 0);
							}
						}
						"chat_exported" => {
							if let Some(data) = val.get("data") {
								let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
								parts.frame.set_status_text("Status: Connected", 0);
								if data.get("text_only").and_then(|t| t.as_bool()).unwrap_or(false) {
									let count = data.get("message_count").and_then(|m| m.as_i64()).unwrap_or(0);
									let msg = format!("Chat text exported to {}. {} messages.", path, count);
									live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
									return;
								}
								let media_exported = data.get("media_exported").and_then(|m| m.as_i64()).unwrap_or(0);
								let media_skipped = data.get("media_skipped").and_then(|m| m.as_i64()).unwrap_or(0);
								let msg = if media_skipped > 0 {
									format!(
										"Chat exported to {}. {} media files included, {} could not be downloaded and were skipped.",
										path, media_exported, media_skipped
									)
								} else {
									format!("Chat exported to {}. {} media files included.", path, media_exported)
								};
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
							}
						}
						"statuses" => {
							if let Some(data) = val.get("data") {
								if let Some(groups) = data.get("groups").and_then(|g| g.as_array()) {
									let mut parsed = Vec::new();
									for g in groups {
										let poster_jid =
											g.get("posterJid").and_then(|j| j.as_str()).unwrap_or("").to_string();
										let poster_name =
											g.get("posterName").and_then(|n| n.as_str()).unwrap_or("").to_string();
										let all_viewed = g.get("allViewed").and_then(|v| v.as_bool()).unwrap_or(false);
										let mut items = Vec::new();
										if let Some(statuses) = g.get("statuses").and_then(|s| s.as_array()) {
											for it in statuses {
												items.push(StatusItem {
													id: it.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
													text: it
														.get("text")
														.and_then(|v| v.as_str())
														.unwrap_or("")
														.to_string(),
													media_type: it
														.get("mediaType")
														.and_then(|v| v.as_str())
														.unwrap_or("")
														.to_string(),
													timestamp: it
														.get("timestamp")
														.and_then(|v| v.as_i64())
														.unwrap_or(0),
													viewed: it.get("viewed").and_then(|v| v.as_bool()).unwrap_or(false),
												});
											}
										}
										parsed.push(StatusGroup { poster_jid, poster_name, all_viewed, items });
									}
									let mut s = state.borrow_mut();
									s.status_groups = parsed;
									if s.active_tab == 1 {
										apply_chat_filter(&mut s, parts);
									}
								}
							}
						}
						"status_update" => {
							if let Some(data) = val.get("data") {
								let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
								let poster_jid =
									data.get("posterJid").and_then(|v| v.as_str()).unwrap_or("").to_string();
								if id.is_empty() || poster_jid.is_empty() {
									return;
								}
								let from_me = data.get("fromMe").and_then(|v| v.as_bool()).unwrap_or(false);
								let text = data.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
								let media_type =
									data.get("mediaType").and_then(|v| v.as_str()).unwrap_or("").to_string();
								let timestamp = data.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

								let mut s = state.borrow_mut();
								let poster_name =
									s.contacts.get(&poster_jid).cloned().unwrap_or_else(|| {
										if from_me { "You".to_string() } else { poster_jid.clone() }
									});
								let item = StatusItem { id, text, media_type, timestamp, viewed: from_me };
								if let Some(group) = s.status_groups.iter_mut().find(|g| g.poster_jid == poster_jid) {
									group.items.push(item);
									group.all_viewed = group.items.iter().all(|it| it.viewed);
								} else {
									s.status_groups.push(StatusGroup {
										poster_jid,
										poster_name,
										all_viewed: from_me,
										items: vec![item],
									});
								}
								if s.active_tab == 1 {
									apply_chat_filter(&mut s, parts);
								}
							}
						}
						"starred_messages" => {
							if let Some(data) = val.get("data") {
								if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
									let mut s = state.borrow_mut();
									let mut entries = Vec::new();
									for item in results {
										let jid = item.get("jid").and_then(|j| j.as_str()).unwrap_or("").to_string();
										let message_id =
											item.get("id").and_then(|m| m.as_str()).unwrap_or("").to_string();
										if jid.is_empty() || message_id.is_empty() {
											continue;
										}
										let from_me = item.get("fromMe").and_then(|f| f.as_bool()).unwrap_or(false);
										let sender_name = item.get("senderName").and_then(|n| n.as_str()).unwrap_or("");
										let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
										let quoted_text = item.get("quotedText").and_then(|t| t.as_str()).unwrap_or("");
										let media_type = item.get("mediaType").and_then(|t| t.as_str()).unwrap_or("");
										let timestamp = item.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

										let chat_name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
										let sender = if from_me {
											"You".to_string()
										} else if !sender_name.is_empty() {
											sender_name.to_string()
										} else {
											chat_name.clone()
										};
										let body = if !text.is_empty() {
											whantv::friendly_preview_text(text)
										} else if !quoted_text.is_empty() {
											whantv::friendly_preview_text(quoted_text)
										} else if !media_type.is_empty() {
											"Media".to_string()
										} else {
											"Message".to_string()
										};
										let date_str = if timestamp > 0 {
											chrono::DateTime::from_timestamp(timestamp, 0)
												.map(|dt| {
													let local_dt: chrono::DateTime<chrono::Local> = dt.into();
													local_dt.format("%b %e, %Y").to_string()
												})
												.unwrap_or_default()
										} else {
											String::new()
										};
										let label = if date_str.is_empty() {
											format!("{} ({}): {}", chat_name, sender, body)
										} else {
											format!("{} ({}): {}, {}", chat_name, sender, body, date_str)
										};

										entries.push(StarredEntry { jid, message_id, label });
									}
									s.starred_entries = entries;
									if s.active_tab == 2 {
										apply_chat_filter(&mut s, parts);
									}
								}
							}
						}
						"search_results" => {
							if let Some(data) = val.get("data") {
								let response_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								// The chat may have been switched again while this
								// search was in flight - applying a stale response
								// would silently populate the wrong chat's messages.
								if state.borrow().selected_chat.as_deref() != Some(response_jid) {
									return;
								}
								if let Some(results) = data.get("results").and_then(|r| r.as_array()) {
									let mut parsed_msgs: Vec<ChatMessage> = Vec::new();
									for msg_val in results {
										if let Ok(msg) = serde_json::from_value::<ChatMessage>(msg_val.clone()) {
											parsed_msgs.push(msg);
										}
									}

									let mut with_dates = Vec::new();
									let mut last_date: Option<chrono::NaiveDate> = None;
									for msg in parsed_msgs {
										if let Some(datetime) = chrono::DateTime::from_timestamp(msg.timestamp, 0) {
											let local_datetime: chrono::DateTime<chrono::Local> = datetime.into();
											let day = whantv::date_key(local_datetime);
											if last_date != Some(day) {
												last_date = Some(day);
												with_dates.push(ChatMessage {
													text: whantv::format_date_divider_label(local_datetime),
													timestamp: msg.timestamp,
													is_date_divider: true,
													..Default::default()
												});
											}
										}
										with_dates.push(msg);
									}

									let mut s = state.borrow_mut();
									let is_group =
										s.selected_chat.as_ref().map(|j| j.ends_with("@g.us")).unwrap_or(false);
									s.messages = with_dates;
									s.display_items = whantv::group_messages_into_display_items(&s.messages);

									parts.conversations_parts.messages_list.freeze();
									parts.conversations_parts.messages_list.clear();
									for item in &s.display_items {
										parts.conversations_parts.messages_list.append(&item.format_display(is_group));
									}
									parts.conversations_parts.messages_list.thaw();
									refresh_selection_list(&mut s, parts);
									let count = parts.conversations_parts.messages_list.get_count();
									if count > 0 {
										parts.conversations_parts.messages_list.set_selection(0, true);
									}
									refresh_voice_action_button(&s, &parts.conversations_parts);
								}
							}
						}
						"typing" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data
									.get("chat_jid")
									.or_else(|| data.get("jid"))
									.and_then(|j| j.as_str())
									.unwrap_or("");
								let mut sender_jid = data
									.get("sender_jid")
									.or_else(|| data.get("jid"))
									.and_then(|j| j.as_str())
									.unwrap_or("");
								if sender_jid.is_empty() {
									sender_jid = chat_jid;
								}
								if chat_jid.is_empty() || sender_jid.is_empty() {
									return;
								}
								let is_typing = data.get("typing").and_then(|t| t.as_bool()).unwrap_or(false);
								let is_recording = data.get("recording").and_then(|r| r.as_bool()).unwrap_or(false);

								let state_changed = {
									let mut s = state.borrow_mut();
									if is_typing || is_recording {
										s.typing_timestamps.insert(chat_jid.to_string(), std::time::Instant::now());
									} else {
										s.typing_timestamps.remove(chat_jid);
									}
									let new_kind: u8 = if is_recording {
										2
									} else if is_typing {
										1
									} else {
										0
									};
									let prev_kind = s.typing_announce_state.get(chat_jid).copied().unwrap_or(0);
									if new_kind == 0 {
										s.typing_announce_state.remove(chat_jid);
									} else {
										s.typing_announce_state.insert(chat_jid.to_string(), new_kind);
									}
									new_kind != prev_kind
								};

								let s = state.borrow();
								if Some(chat_jid.to_string()) == s.selected_chat {
									let sender_name = s
										.contacts
										.get(sender_jid)
										.cloned()
										.unwrap_or_else(|| display_name_for_jid(sender_jid));
									if !sender_name.is_empty() {
										let announcement = if is_recording && s.settings.announce_recording {
											Some(format!("{} is recording audio...", sender_name))
										} else if is_typing && s.settings.announce_typing {
											Some(format!("{} is typing...", sender_name))
										} else {
											None
										};
										if let Some(ann) = announcement {
											if state_changed {
												live_region::announce(
													parts.conversations_parts.messages_announcer.clone(),
													&ann,
												);
											}
										}
										let norm_chat_jid = if let Some((user, server)) = chat_jid.split_once('@') {
											let base = user
												.split('.')
												.next()
												.unwrap_or(user)
												.split(':')
												.next()
												.unwrap_or(user);
											format!("{}@{}", base, server)
										} else {
											chat_jid.to_string()
										};
										let base_label = s
											.presence_labels
											.get(chat_jid)
											.or_else(|| s.presence_labels.get(&norm_chat_jid))
											.cloned()
											.unwrap_or_else(|| {
												s.contacts
													.get(chat_jid)
													.cloned()
													.unwrap_or_else(|| "Conversation data".to_string())
											});

										let btn_label = if is_recording || is_typing {
											let status = if is_recording {
												format!("{} is recording audio...", sender_name)
											} else {
												format!("{} is typing...", sender_name)
											};
											format!("{}, {}", base_label, status)
										} else {
											base_label
										};
										parts.conversations_parts.conv_data_btn.set_label(&btn_label);
									}
								}
							}
						}
						"presence" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								if jid.is_empty() || jid.ends_with("@g.us") {
									return;
								}
								let clean_jid = if let Some((user, server)) = jid.split_once('@') {
									let base_user =
										user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
									format!("{}@{}", base_user, server)
								} else {
									jid.to_string()
								};
								let unavailable = data.get("unavailable").and_then(|u| u.as_bool()).unwrap_or(false);
								let last_seen = data.get("last_seen").and_then(|l| l.as_i64()).unwrap_or(0);
								let mut s = state.borrow_mut();

								let name = s
									.contacts
									.get(&clean_jid)
									.or_else(|| s.contacts.get(jid))
									.cloned()
									.unwrap_or_else(|| display_name_for_jid(&clean_jid));

								let btn_label = if unavailable {
									if last_seen > 0 {
										if let Some(datetime) = chrono::DateTime::from_timestamp(last_seen, 0) {
											let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
											let now = chrono::Local::now();
											let time_str = local_dt.format("%l:%M %p").to_string().trim().to_string();
											let date_str = if local_dt.date_naive() == now.date_naive() {
												format!("today at {}", time_str)
											} else {
												local_dt.format("%b %e at %l:%M %p").to_string().replace("  ", " ")
											};
											format!("{} (last seen {})", name, date_str)
										} else {
											name.clone()
										}
									} else {
										name.clone()
									}
								} else {
									format!("{} (online)", name)
								};

								s.presence_labels.insert(clean_jid.clone(), btn_label.clone());
								s.presence_labels.insert(jid.to_string(), btn_label.clone());

								if Some(clean_jid.clone()) == s.selected_chat
									|| Some(jid.to_string()) == s.selected_chat
								{
									let mut should_announce = true;
									if !s.initial_presence_received {
										s.initial_presence_received = true;
										should_announce = false;
									}

									let announcement = if unavailable {
										if last_seen > 0 {
											if let Some(datetime) = chrono::DateTime::from_timestamp(last_seen, 0) {
												let local_dt: chrono::DateTime<chrono::Local> = datetime.into();
												let now = chrono::Local::now();
												let time_str =
													local_dt.format("%l:%M %p").to_string().trim().to_string();
												let date_str = if local_dt.date_naive() == now.date_naive() {
													format!("today at {}", time_str)
												} else {
													local_dt.format("%b %e at %l:%M %p").to_string().replace("  ", " ")
												};
												format!("{}, last seen {}", name, date_str)
											} else {
												format!("{}, offline", name)
											}
										} else {
											format!("{}, offline", name)
										}
									} else {
										format!("{} is now online", name)
									};
									if should_announce {
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&announcement,
										);
									}
									parts.conversations_parts.conv_data_btn.set_label(&btn_label);
								}
							}
						}
						"group_info" => {
							if let Some(payload) = val.get("data") {
								let info = payload.get("Info").unwrap_or(payload);
								let viewer_is_admin =
									payload.get("ViewerIsAdmin").and_then(|v| v.as_bool()).unwrap_or(false);

								if let Some(group_jid) =
									info.get("JID").and_then(|j| j.get("User")).and_then(|u| u.as_str())
								{
									let full_jid = format!("{}@g.us", group_jid);
									let mut s = state.borrow_mut();
									if viewer_is_admin {
										s.admin_group_jids.insert(full_jid);
									} else {
										s.admin_group_jids.remove(&full_jid);
									}
								}

								// Set visibility of admin buttons
								parts.group_data_dialog.add_member_btn.show(viewer_is_admin);
								parts.group_data_dialog.make_admin_btn.show(viewer_is_admin);
								parts.group_data_dialog.remove_member_btn.show(viewer_is_admin);
								parts.group_data_dialog.save_settings_btn.show(viewer_is_admin);
								parts.group_data_dialog.announce_chk.enable(viewer_is_admin);
								parts.group_data_dialog.locked_chk.enable(viewer_is_admin);

								// Set basic settings
								if let Some(topic) = info.get("Topic").and_then(|t| t.as_str()) {
									parts.group_data_dialog.group_desc_field.set_value(topic);
								}
								if let Some(is_announce) = info.get("IsAnnounce").and_then(|t| t.as_bool()) {
									parts.group_data_dialog.announce_chk.set_value(is_announce);
								}
								if let Some(is_locked) = info.get("IsLocked").and_then(|t| t.as_bool()) {
									parts.group_data_dialog.locked_chk.set_value(is_locked);
								}
								let is_community = info.get("IsParent").and_then(|t| t.as_bool()).unwrap_or(false);
								parts.group_data_dialog.groups_list.clear();
								if is_community {
									let s = state.borrow();
									if let Some(ref jid) = s.selected_chat {
										let payload = serde_json::json!({
											"action": "get_community_subgroups",
											"data": { "jid": jid }
										});
										let _ = s.ws_tx.send(payload.to_string());
									}
								}

								// Parse members
								let s = state.borrow();
								let mut member_strings = Vec::new();
								let mut member_jids = Vec::new();
								let mut member_admin = Vec::new();
								if let Some(participants) = payload.get("Participants").and_then(|p| p.as_array()) {
									for p in participants {
										if let Some(jid) = p.get("JID").and_then(|j| j.as_str()) {
											// PhoneJID is the participant's actual phone-number
											// JID (backend-resolved from a @lid address when the
											// group hides phone numbers) - display/contact lookups
											// must use it, never the raw action JID, or a privacy
											// group shows unreadable LIDs instead of names/numbers.
											let phone_jid = p.get("PhoneJID").and_then(|j| j.as_str()).unwrap_or(jid);
											let is_admin = p.get("IsAdmin").and_then(|a| a.as_bool()).unwrap_or(false);
											let is_super =
												p.get("IsSuperAdmin").and_then(|a| a.as_bool()).unwrap_or(false);

											let backend_name = p.get("Name").and_then(|n| n.as_str()).unwrap_or("");

											let mut local_name = if !backend_name.is_empty() {
												backend_name.to_string()
											} else {
												s.contacts
													.get(phone_jid)
													.cloned()
													.unwrap_or_else(|| phone_jid.to_string())
											};

											let raw_phone = phone_jid.split('@').next().unwrap_or(phone_jid);
											let mut phone = raw_phone.to_string();
											if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
												phone =
													parsed.format().mode(phonenumber::Mode::International).to_string();
											}

											if local_name == phone_jid {
												local_name = phone.clone();
											}

											let mut role = "";
											if is_super {
												role = " (Super Admin)";
											} else if is_admin {
												role = " (Admin)";
											}

											let display = if local_name == phone {
												format!("{}{}", local_name, role)
											} else {
												format!("{}{} - {}", local_name, role, phone)
											};
											member_strings.push(display);
											member_jids.push(jid.to_string());
											member_admin.push(is_admin || is_super);
										}
									}
								}
								drop(s);
								let mut s_mut = state.borrow_mut();
								s_mut.group_data_members = member_strings;
								s_mut.group_data_member_jids = member_jids;
								s_mut.group_data_member_admin = member_admin;

								// Populate Media/Documents/Links
								s_mut.group_data_media = build_media_messages(&s_mut.messages);
								s_mut.group_data_documents = build_document_messages(&s_mut.messages);
								s_mut.group_data_links = build_link_messages(&s_mut.messages);
								populate_message_list(
									&parts.group_data_dialog.media_list,
									&s_mut.group_data_media,
									|m| m.display_filename(),
								);
								populate_message_list(
									&parts.group_data_dialog.documents_list,
									&s_mut.group_data_documents,
									|m| m.display_filename(),
								);
								populate_message_list(
									&parts.group_data_dialog.links_list,
									&s_mut.group_data_links,
									|m| m.first_link().unwrap_or_default(),
								);

								let query_members = parts.group_data_dialog.members_search.get_value().to_lowercase();
								parts.group_data_dialog.members_list.clear();
								let mut visible_jids = Vec::new();
								for (i, member) in s_mut.group_data_members.iter().enumerate() {
									if query_members.is_empty() || member.to_lowercase().contains(&query_members) {
										parts.group_data_dialog.members_list.append(member);
										visible_jids.push(s_mut.group_data_member_jids[i].clone());
									}
								}
								s_mut.group_data_visible_member_jids = visible_jids;
								drop(s_mut);
								// The list was just rebuilt from scratch, so there's no
								// selection yet - reset the label to the no-selection default.
								parts.group_data_dialog.make_admin_btn.set_label("Make admin");
							}
						}
						"community_subgroups" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let s = state.borrow();
								let is_relevant = s.selected_chat.as_deref() == Some(jid)
									&& parts.group_data_dialog.dialog.is_shown();
								drop(s);
								if is_relevant {
									parts.group_data_dialog.groups_list.clear();
									let mut subgroup_jids = Vec::new();
									if let Some(groups) = data.get("groups").and_then(|g| g.as_array()) {
										for g in groups {
											let name = g.get("Name").and_then(|n| n.as_str()).unwrap_or("");
											let is_default =
												g.get("IsDefaultSubGroup").and_then(|d| d.as_bool()).unwrap_or(false);
											let label = if is_default {
												format!("{} (Announcements)", name)
											} else {
												name.to_string()
											};
											parts.group_data_dialog.groups_list.append(&label);
											subgroup_jids
												.push(g.get("JID").and_then(|j| j.as_str()).unwrap_or("").to_string());
										}
									}
									state.borrow_mut().group_data_subgroup_jids = subgroup_jids;
								}
							}
						}
						"group_updated" => {
							if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
								let s = state.borrow();
								let is_relevant = s.selected_chat.as_deref() == Some(jid)
									&& parts.group_data_dialog.dialog.is_shown();
								if is_relevant {
									let payload = serde_json::json!({
										"action": "get_group_info",
										"data": { "jid": jid }
									});
									let _ = s.ws_tx.send(payload.to_string());
								}
							}
						}
						"community_subgroup_joined" => {
							if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
								// The chat list itself already self-corrects via the
								// broadcastChats() the backend fires right before this -
								// but silence otherwise leaves a screen-reader user unable
								// to tell "joined" apart from "nothing happened yet" (the
								// failure paths for this same action all announce via the
								// "error" event; this was the one success path that didn't).
								let name = state.borrow().contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&format!("Joined {}.", name),
								);
							}
						}
						"group_invite_info" => {
							if let Some(data) = val.get("data") {
								let message_id =
									data.get("messageId").and_then(|v| v.as_str()).unwrap_or("").to_string();
								let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
								let topic = data.get("topic").and_then(|v| v.as_str()).unwrap_or("");
								let size = data.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
								let display_name = if name.is_empty() { "This group".to_string() } else { name };
								let mut body = format!("{display_name}\n{size} member(s)");
								if !topic.is_empty() {
									body.push_str(&format!("\n\n{topic}"));
								}
								let dlg = MessageDialog::builder(&parts.frame, &body, "Group Invite")
									.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
									.build();
								if dlg.show_modal() == wxdragon::id::ID_YES {
									let _ = state.borrow().ui_tx.send(UiCommand::JoinGroupInvite(message_id));
								}
							}
						}
						"group_invite_joined" => {
							let name = val
								.get("data")
								.and_then(|d| d.get("jid"))
								.and_then(|j| j.as_str())
								.and_then(|jid| state.borrow().contacts.get(jid).cloned())
								.unwrap_or_else(|| "the group".to_string());
							live_region::announce(
								parts.conversations_parts.messages_announcer.clone(),
								&format!("Joined {}.", name),
							);
						}
						"about_updated" => {
							if let Some(jid) = val.get("data").and_then(|d| d.get("jid")).and_then(|j| j.as_str()) {
								let s = state.borrow();
								let is_relevant = s.selected_chat.as_deref() == Some(jid)
									&& parts.conversation_data_dialog.dialog.is_shown();
								if is_relevant {
									let payload = serde_json::json!({
										"action": "get_user_info",
										"data": { "jid": jid }
									});
									let _ = s.ws_tx.send(payload.to_string());
								}
							}
						}
						"user_info" => {
							if let Some(data) = val.get("data") {
								let jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let status = data.get("status").and_then(|s| s.as_str()).unwrap_or("");
								let push_name = data.get("pushName").and_then(|p| p.as_str()).unwrap_or("");
								let full_name = data.get("fullName").and_then(|f| f.as_str()).unwrap_or("");

								let s = state.borrow();
								if Some(jid.to_string()) == s.selected_chat {
									let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.to_string());
									let raw_phone = jid.split('@').next().unwrap_or(jid);
									let mut phone = raw_phone.to_string();
									if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
										phone = parsed.format().mode(phonenumber::Mode::International).to_string();
									}

									let mut text = format!("Name: {}\nPhone: {}\n", local_name, phone);
									if !push_name.is_empty() {
										text.push_str(&format!("WhatsApp Name: {}\n", push_name));
									} else if !full_name.is_empty() {
										text.push_str(&format!("WhatsApp Name: {}\n", full_name));
									} else {
										text.push_str("WhatsApp Name: Not available\n");
									}

									if !status.is_empty() {
										text.push_str(&format!("About/Status: {}\n", status));
									} else {
										text.push_str("About/Status: Not set\n");
									}

									parts.conversation_data_dialog.info_field.set_value(&text);
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&format!("Conversation data loaded for {}", local_name),
									);
								}
							}
						}
						"own_profile" => {
							if let Some(data) = val.get("data") {
								let mut s = state.borrow_mut();
								if let Some(push_name) = data.get("pushName").and_then(|p| p.as_str()) {
									s.own_push_name = push_name.to_string();
								}
								if let Some(about) = data.get("about").and_then(|a| a.as_str()) {
									s.own_about = about.to_string();
								}
								let push_name = s.own_push_name.clone();
								let about = s.own_about.clone();
								drop(s);
								if parts.settings_dialog.dialog.is_shown() {
									parts.settings_dialog.name_field.set_value(&push_name);
									parts.settings_dialog.about_field.set_value(&about);
								}
							}
						}
						"privacy_settings" => {
							if let Some(data) = val.get("data") {
								let mut s = state.borrow_mut();
								for (key, ptr) in [
									("groupadd", "groupadd"),
									("last", "last"),
									("status", "status"),
									("profile", "profile"),
									("readreceipts", "readreceipts"),
									("online", "online"),
									("calladd", "calladd"),
								] {
									if let Some(v) = data.get(key).and_then(|v| v.as_str()) {
										s.privacy_settings.insert(ptr.to_string(), v.to_string());
									}
								}
								s.privacy_settings_loaded = true;
								let privacy_settings = s.privacy_settings.clone();
								drop(s);
								if parts.settings_dialog.dialog.is_shown() {
									use crate::ui::settings_dialog::*;
									parts.settings_dialog.ok_btn.enable(true);
									let get = |k: &str| privacy_settings.get(k).map(|s| s.as_str()).unwrap_or("all");
									set_combo_from_value(
										&parts.settings_dialog.last_seen_combo,
										VISIBILITY_CHOICES,
										get("last"),
									);
									set_combo_from_value(
										&parts.settings_dialog.profile_photo_combo,
										VISIBILITY_CHOICES,
										get("profile"),
									);
									set_combo_from_value(
										&parts.settings_dialog.about_privacy_combo,
										VISIBILITY_CHOICES,
										get("status"),
									);
									set_combo_from_value(
										&parts.settings_dialog.read_receipts_combo,
										READ_RECEIPTS_CHOICES,
										get("readreceipts"),
									);
									set_combo_from_value(
										&parts.settings_dialog.groups_combo,
										VISIBILITY_CHOICES,
										get("groupadd"),
									);
									set_combo_from_value(
										&parts.settings_dialog.calls_combo,
										CALL_CHOICES,
										get("calladd"),
									);
									set_combo_from_value(
										&parts.settings_dialog.online_combo,
										ONLINE_CHOICES,
										get("online"),
									);
								}
							}
						}
						"message_status" | "receipt" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data
									.get("jid")
									.or_else(|| data.get("chat_jid"))
									.and_then(|j| j.as_str())
									.unwrap_or("");
								let raw_status = data
									.get("status")
									.or_else(|| data.get("type"))
									.and_then(|s| s.as_str())
									.unwrap_or("");

								let mut msg_ids = Vec::new();
								if let Some(ids) = data.get("message_ids").and_then(|i| i.as_array()) {
									for id_val in ids {
										if let Some(id_str) = id_val.as_str() {
											msg_ids.push(id_str.to_string());
										}
									}
								} else if let Some(id_str) = data.get("message_id").and_then(|i| i.as_str()) {
									msg_ids.push(id_str.to_string());
								}

								// Normalize JID: strip :device suffix so "1234:0@s.whatsapp.net"
								// compares equal to "1234@s.whatsapp.net".
								let norm_jid_str = |jid: &str| -> String {
									let no_device = jid.split(':').next().unwrap_or(jid);
									if no_device.contains('@') {
										no_device.to_string()
									} else {
										jid.split('@').next().unwrap_or(jid).to_string()
									}
								};
								let norm_incoming = norm_jid_str(chat_jid);

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_incoming == norm_jid_str(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									let norm_status = match raw_status {
										"read-self" => "read",
										"sender" => "sent",
										"inactive" => "delivered",
										"" => "delivered",
										other => other,
									};
									let receipt_ts = data.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0);

									let mut any_updated = false;
									for target_id in &msg_ids {
										let mut found = false;
										for msg in s.messages.iter_mut() {
											if msg.id == *target_id {
												if get_status_rank(norm_status) >= get_status_rank(&msg.status) {
													msg.status = norm_status.to_string();
													if receipt_ts > 0 {
														match norm_status {
															"delivered" => {
																if msg.delivered_at == 0 {
																	msg.delivered_at = receipt_ts;
																}
															}
															"read" => {
																if msg.read_at == 0 {
																	msg.read_at = receipt_ts;
																}
															}
															"played" => {
																if msg.played_at == 0 {
																	msg.played_at = receipt_ts;
																}
															}
															_ => {}
														}
													}
												}
												found = true;
												any_updated = true;
												break;
											}
										}
										if !found {
											s.pending_receipts.insert(target_id.clone(), norm_status.to_string());
										}
									}
									if any_updated {
										// Update display items in-place instead of full O(n) regrouping
										// Collect status updates first to avoid simultaneous borrows
										let status_updates: Vec<(String, String, i64, i64, i64)> = s
											.messages
											.iter()
											.filter(|msg| msg_ids.contains(&msg.id))
											.map(|msg| {
												(
													msg.id.clone(),
													msg.status.clone(),
													msg.delivered_at,
													msg.read_at,
													msg.played_at,
												)
											})
											.collect();
										for (upd_id, upd_status, upd_del, upd_read, upd_played) in &status_updates {
											for item in s.display_items.iter_mut() {
												match item {
													whantv::DisplayItem::Single(m) if m.id == *upd_id => {
														m.status = upd_status.clone();
														m.delivered_at = *upd_del;
														m.read_at = *upd_read;
														m.played_at = *upd_played;
														break;
													}
													whantv::DisplayItem::Album { messages, .. } => {
														if let Some(am) =
															messages.iter_mut().find(|am| am.id == *upd_id)
														{
															am.status = upd_status.clone();
															am.delivered_at = *upd_del;
															am.read_at = *upd_read;
															am.played_at = *upd_played;
															break;
														}
													}
													_ => {}
												}
											}
										}
										for (di, item) in s.display_items.iter().enumerate() {
											let contains = match item {
												whantv::DisplayItem::Single(m) => msg_ids.contains(&m.id),
												whantv::DisplayItem::Album { messages, .. } => {
													messages.iter().any(|m| msg_ids.contains(&m.id))
												}
											};
											if contains {
												parts
													.conversations_parts
													.messages_list
													.set_string(di as u32, &item.format_display(is_group));
											}
										}
									}
									if any_updated && (norm_status == "sent" || norm_status == "delivered") {
										// A reconnect or history sync can flush receipts for a
										// whole backlog of older outgoing messages at once; only
										// ding for messages we actually just sent, not every
										// backlogged ID that happens to be getting acked now.
										let now_ts = std::time::SystemTime::now()
											.duration_since(std::time::UNIX_EPOCH)
											.unwrap_or_default()
											.as_secs() as i64;
										let mut should_play = false;
										for target_id in &msg_ids {
											if s.played_sent_sounds.insert(target_id.clone()) {
												let is_recent = s
													.messages
													.iter()
													.any(|m| m.id == *target_id && now_ts - m.sent_at < 20);
												if is_recent {
													should_play = true;
												}
											}
										}
										if should_play {
											play_notification_sound(
												NotificationSound::MessageSent,
												s.settings.sounds_enabled,
											);
										}
									}
								}
							}
						}
						"message_edited" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								let new_text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
								let edited_at = data.get("edited_at").and_then(|e| e.as_i64()).unwrap_or(0);

								let norm_jid_edit = |jid: &str| -> String {
									if let Some((user, server)) = jid.split_once('@') {
										let base_user =
											user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
										format!("{}@{}", base_user, server)
									} else {
										jid.to_string()
									}
								};

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_jid_edit(chat_jid) == norm_jid_edit(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									let mut changed = false;
									for msg in s.messages.iter_mut() {
										if msg.id == target_id {
											msg.text = new_text.to_string();
											msg.is_edited = true;
											msg.edited_at = edited_at;
											changed = true;
											break;
										}
									}
									if changed {
										for msg in s.messages.iter_mut() {
											if msg.quoted_id == target_id {
												msg.quoted_text = new_text.to_string();
											}
										}
										// Update display_items in-place instead of full O(n) regrouping
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.id == target_id => {
													m.text = new_text.to_string();
													m.is_edited = true;
													m.edited_at = edited_at;
													break;
												}
												whantv::DisplayItem::Album { messages, .. } => {
													if let Some(am) = messages.iter_mut().find(|am| am.id == target_id)
													{
														am.text = new_text.to_string();
														am.is_edited = true;
														am.edited_at = edited_at;
														break;
													}
												}
												_ => {}
											}
										}
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.quoted_id == target_id => {
													m.quoted_text = new_text.to_string();
												}
												whantv::DisplayItem::Album { messages, .. } => {
													for am in messages.iter_mut() {
														if am.quoted_id == target_id {
															am.quoted_text = new_text.to_string();
														}
													}
												}
												_ => {}
											}
										}
										for (di, item) in s.display_items.iter().enumerate() {
											let contains = match item {
												whantv::DisplayItem::Single(m) => {
													m.id == target_id || m.quoted_id == target_id
												}
												whantv::DisplayItem::Album { messages, .. } => messages
													.iter()
													.any(|m| m.id == target_id || m.quoted_id == target_id),
											};
											if contains {
												let updated_display = item.format_display(is_group);
												parts
													.conversations_parts
													.messages_list
													.set_string(di as u32, &updated_display);
												live_region::announce(
													parts.conversations_parts.messages_announcer.clone(),
													&format!("Message edited: {}", updated_display),
												);
												break;
											}
										}
									}
								}
							}
						}
						"message_starred" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								let starred = data.get("starred").and_then(|s| s.as_bool()).unwrap_or(false);

								let norm_jid_star = |jid: &str| -> String {
									if let Some((user, server)) = jid.split_once('@') {
										let base_user =
											user.split('.').next().unwrap_or(user).split(':').next().unwrap_or(user);
										format!("{}@{}", base_user, server)
									} else {
										jid.to_string()
									}
								};

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_jid_star(chat_jid) == norm_jid_star(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");
									for msg in s.messages.iter_mut() {
										if msg.id == target_id {
											msg.starred = starred;
											break;
										}
									}
									for item in s.display_items.iter_mut() {
										if let whantv::DisplayItem::Single(m) = item {
											if m.id == target_id {
												m.starred = starred;
												break;
											}
										}
									}
									for (di, item) in s.display_items.iter().enumerate() {
										let matches =
											matches!(item, whantv::DisplayItem::Single(m) if m.id == target_id);
										if matches {
											let updated_display = item.format_display(is_group);
											parts
												.conversations_parts
												.messages_list
												.set_string(di as u32, &updated_display);
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												if starred { "Message starred." } else { "Message unstarred." },
											);
											break;
										}
									}
								}
								if s.active_tab == 2 {
									let payload = serde_json::json!({ "action": "get_starred_messages" });
									let _ = s.ws_tx.send(payload.to_string());
								}
							}
						}
						"message_reaction" => {
							if let Some(data) = val.get("data") {
								let chat_jid = data.get("jid").and_then(|j| j.as_str()).unwrap_or("");
								let target_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("");
								let reactor = data.get("reactor").and_then(|r| r.as_str()).unwrap_or("");
								let emoji = data.get("emoji").and_then(|e| e.as_str()).unwrap_or("");
								let reactor_name_val = data.get("reactor_name").and_then(|n| n.as_str()).unwrap_or("");

								let norm_jid_react = |jid: &str| -> String {
									let no_device = jid.split(':').next().unwrap_or(jid);
									if no_device.contains('@') {
										no_device.to_string()
									} else {
										jid.split('@').next().unwrap_or(jid).to_string()
									}
								};

								let mut s = state.borrow_mut();
								let is_match = if let Some(ref sel) = s.selected_chat {
									norm_jid_react(chat_jid) == norm_jid_react(sel)
								} else {
									false
								};

								if is_match {
									let is_group = chat_jid.ends_with("@g.us");

									let is_own_reaction =
										reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";

									let resolved_name = if is_own_reaction {
										"You".to_string()
									} else if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
										reactor_name_val.to_string()
									} else {
										s.contacts
											.get(norm_jid_react(reactor).as_str())
											.or_else(|| s.contacts.get(reactor))
											.cloned()
											.unwrap_or_else(|| "Contact".to_string())
									};

									for msg in s.messages.iter_mut() {
										if msg.id == target_id {
											if emoji.is_empty() {
												msg.reactions.retain(|r| {
													if is_own_reaction {
														r.reactor_name != "You"
															&& r.reactor_jid != "me" && r.reactor_name != "me"
													} else {
														r.reactor_jid != reactor && r.reactor_name != resolved_name
													}
												});
											} else {
												let mut found = false;
												for r in msg.reactions.iter_mut() {
													let r_is_own = r.reactor_name == "You" || r.reactor_jid == "me";
													if (is_own_reaction && r_is_own)
														|| (!is_own_reaction
															&& (r.reactor_jid == reactor
																|| r.reactor_name == resolved_name))
													{
														r.emoji = emoji.to_string();
														r.reactor_name = resolved_name.clone();
														found = true;
														break;
													}
												}
												if !found {
													msg.reactions.push(whantv::ChatReaction {
														reactor_jid: if is_own_reaction {
															"me".to_string()
														} else {
															reactor.to_string()
														},
														reactor_name: resolved_name.clone(),
														emoji: emoji.to_string(),
														timestamp: std::time::SystemTime::now()
															.duration_since(std::time::UNIX_EPOCH)
															.unwrap_or_default()
															.as_secs() as i64,
													});
												}
											}
											break;
										}
									}

									// Sync reactions into display_items in-place instead of full O(n) regrouping
									let updated_reactions =
										s.messages.iter().find(|m| m.id == target_id).map(|m| m.reactions.clone());
									if let Some(new_reactions) = updated_reactions {
										for item in s.display_items.iter_mut() {
											match item {
												whantv::DisplayItem::Single(m) if m.id == target_id => {
													m.reactions = new_reactions.clone();
													break;
												}
												whantv::DisplayItem::Album { messages, .. } => {
													if let Some(am) = messages.iter_mut().find(|am| am.id == target_id)
													{
														am.reactions = new_reactions.clone();
														break;
													}
												}
												_ => {}
											}
										}
									}
									for (di, item) in s.display_items.iter().enumerate() {
										let contains = match item {
											whantv::DisplayItem::Single(m) => m.id == target_id,
											whantv::DisplayItem::Album { messages, .. } => {
												messages.iter().any(|m| m.id == target_id)
											}
										};
										if contains {
											parts
												.conversations_parts
												.messages_list
												.set_string(di as u32, &item.format_display(is_group));
											let announce_msg = if emoji.is_empty() {
												format!("Reaction removed by {}", resolved_name)
											} else {
												format!("{} reacted with {}", resolved_name, emoji)
											};
											live_region::announce(
												parts.conversations_parts.messages_announcer.clone(),
												&announce_msg,
											);
											break;
										}
									}
								} else if !emoji.is_empty() {
									// Reaction on a chat that isn't currently open - there's no
									// loaded ChatMessage to update, just surface it the way
									// WhatsApp itself does: a notification naming the reactor and
									// what they reacted to.
									let is_own_reaction =
										reactor == "me" || reactor_name_val == "You" || reactor_name_val == "me";
									if !is_own_reaction {
										let text = data.get("text").and_then(|t| t.as_str()).unwrap_or("");
										if !text.is_empty() {
											let resolved_name =
												if !reactor_name_val.is_empty() && !reactor_name_val.contains('@') {
													reactor_name_val.to_string()
												} else {
													s.contacts
														.get(norm_jid_react(reactor).as_str())
														.or_else(|| s.contacts.get(reactor))
														.cloned()
														.unwrap_or_else(|| "Contact".to_string())
												};
											let alert = whantv::incoming_alert(
												s.muted_jids.contains(chat_jid),
												s.archived_jids.contains(chat_jid),
												s.settings.archived_alert(chat_jid),
											);
											if alert.sound {
												play_notification_sound(
													NotificationSound::MessageBackground,
													s.settings.sounds_enabled,
												);
											}
											if alert.toast && s.settings.notifications_enabled {
												if let Some(n) = show_toast_notification(
													&resolved_name,
													&format!(
														"Reacted {} to \"{}\"",
														emoji,
														whantv::friendly_preview_text(text)
													),
												) {
													s.active_notifications
														.entry(chat_jid.to_string())
														.or_default()
														.push(n);
												}
											}
										}
									}
								}
							}
						}
						"copied_album_to_clipboard" => {
							let ann_text = if let Some(data) = val.get("data") {
								let msg_ids: Vec<String> = data
									.get("message_ids")
									.and_then(|ids| ids.as_array())
									.map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
									.unwrap_or_default();

								let mut s = state.borrow_mut();
								let mut is_voice = false;
								for id in &msg_ids {
									s.pending_copy_announcements.remove(id);
									if let Some(m) = s.messages.iter().find(|msg| msg.id == *id) {
										if m.is_audio
											|| m.media_type == "voice" || m.media_type == "audio"
											|| m.text.starts_with("[Voice Message]")
											|| m.text.starts_with("[Audio]")
										{
											is_voice = true;
										}
									}
								}
								if is_voice {
									"Copied voice message to clipboard.".to_string()
								} else if msg_ids.len() <= 1 {
									"Copied file to clipboard.".to_string()
								} else {
									format!("Copied {} files to clipboard.", msg_ids.len())
								}
							} else {
								"Copied file to clipboard.".to_string()
							};

							live_region::announce(parts.conversations_parts.messages_announcer.clone(), &ann_text);
						}
						"media_downloaded" => {
							if let Some(data) = val.get("data") {
								let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								let mut path = data.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
								let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("").to_string();
								let clean_filename = ChatMessage::sanitize_filename(&filename);

								let clean_path = ChatMessage::sanitize_filename(&path);
								if clean_path != path && std::path::Path::new(&path).exists() {
									if std::fs::rename(&path, &clean_path).is_ok() {
										path = clean_path;
									}
								}

								let mut s = state.borrow_mut();
								s.downloads_in_progress.remove(&msg_id);
								if let Some(msg) = s.messages.iter_mut().find(|m| m.id == msg_id) {
									msg.media_path = path.clone();
									if !clean_filename.is_empty() {
										msg.media_filename = clean_filename.clone();
									}
								}
								// Update display_items in-place instead of full O(n) regrouping
								for item in s.display_items.iter_mut() {
									match item {
										whantv::DisplayItem::Single(m) if m.id == msg_id => {
											m.media_path = path.clone();
											if !clean_filename.is_empty() {
												m.media_filename = clean_filename.clone();
											}
											break;
										}
										whantv::DisplayItem::Album { messages, .. } => {
											if let Some(am) = messages.iter_mut().find(|am| am.id == msg_id) {
												am.media_path = path.clone();
												if !clean_filename.is_empty() {
													am.media_filename = clean_filename.clone();
												}
												break;
											}
										}
										_ => {}
									}
								}

								let download_had_focus = parts.conversations_parts.action_download_btn.has_focus();
								parts.conversations_parts.conversation_panel.freeze();
								if update_action_buttons(&s, &parts.conversations_parts) {
									parts.conversations_parts.action_panel.layout();
								}
								parts.conversations_parts.conversation_panel.thaw();

								if download_had_focus && !parts.conversations_parts.action_download_btn.is_shown() {
									if parts.conversations_parts.action_open_btn.is_shown() {
										parts.conversations_parts.action_open_btn.set_focus();
									} else if parts.conversations_parts.action_save_as_btn.is_shown() {
										parts.conversations_parts.action_save_as_btn.set_focus();
									}
								}

								let is_copy_pending = s.pending_copy_announcements.contains(&msg_id);
								let media_type = data.get("mediaType").and_then(|m| m.as_str()).unwrap_or("");
								if media_type != "voice" && !is_copy_pending {
									let announce_name = if filename.is_empty() { "File".to_string() } else { filename };
									live_region::announce(
										parts.conversations_parts.messages_announcer.clone(),
										&format!("{} downloaded", announce_name),
									);
								}
							}
						}
						"media_download_progress" => {
							if let Some(data) = val.get("data") {
								let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								let percent = data.get("percent").and_then(|p| p.as_i64()).unwrap_or(0);
								let mut s = state.borrow_mut();
								let prev = s.downloads_in_progress.insert(msg_id.clone(), percent);
								parts.conversations_parts.conversation_panel.freeze();
								if update_action_buttons(&s, &parts.conversations_parts) {
									parts.conversations_parts.action_panel.layout();
								}
								parts.conversations_parts.conversation_panel.thaw();
								for milestone in [25, 50, 75] {
									if prev.unwrap_or(0) < milestone && percent >= milestone {
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											&format!("Download {}%", percent),
										);
										break;
									}
								}
							}
						}
						"media_download_cancelled" | "media_download_failed" => {
							if let Some(data) = val.get("data") {
								let msg_id = data.get("message_id").and_then(|m| m.as_str()).unwrap_or("").to_string();
								let mut s = state.borrow_mut();
								s.downloads_in_progress.remove(&msg_id);
								parts.conversations_parts.conversation_panel.freeze();
								if update_action_buttons(&s, &parts.conversations_parts) {
									parts.conversations_parts.action_panel.layout();
								}
								parts.conversations_parts.conversation_panel.thaw();
								let announcement = if t == "media_download_cancelled" {
									"Download cancelled".to_string()
								} else {
									let err = data.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
									format!("Download failed: {}", err)
								};
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&announcement,
								);
								parts.conversations_parts.play_voice_btn.set_label("Play voice message");
							}
						}
						"media_saved" => {
							if let Some(data) = val.get("data") {
								let path = data.get("path").and_then(|p| p.as_str()).unwrap_or("");
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&format!("File saved to {}", path),
								);
							}
						}
						"play_audio" => {
							if let Some(data) = val.get("data") {
								if let Some(path) = data.get("path").and_then(|p| p.as_str()) {
									// Backend sends "message_id", not "id"
									let msg_id = data
										.get("message_id")
										.or_else(|| data.get("id"))
										.and_then(|i| i.as_str())
										.map(|s| s.to_string());
									eprintln!(
										"[LOG UI] Playing voice note: {} from path: {}",
										msg_id.as_deref().unwrap_or(""),
										path
									);
									let mut s = state.borrow_mut();
									// Keep playing_msg_id from the UI-side set (which used the correct msg.id),
									// only update it if the server echoes a non-empty id
									if let Some(ref id) = msg_id {
										if !id.is_empty() {
											s.playing_msg_id = Some(id.clone());
										}
									}
									// The user may have hit "pause" while this was still loading
									// (playing_msg_id is set immediately on click, before this
									// response arrives) - honor that instead of always starting
									// playback out from under them.
									let start_paused = s.is_audio_paused;
									s.audio_elapsed_secs = 0.0;
									s.audio_start_instant = None;
									s.audio_path = Some(path.to_string());
									if let Some(ref sink) = s.audio_sink {
										sink.stop();
									}
									s.audio_sink = None;

									// Duration strategy:
									// 1. Count samples via rodio (works for WAV/FLAC/MP3 but not opus)
									// 2. Parse "M:SS" from the message text e.g. "[Voice Message] (0:23)"
									// 3. Count samples by consuming the decoder (always works, small files)
									s.audio_total_secs = 0.0;

									// Try rodio total_duration first (fast, works for some formats)
									if let Ok(probe_file) = File::open(path) {
										if let Ok(probe_dec) = Decoder::new(BufReader::new(probe_file)) {
											use rodio::Source;
											if let Some(dur) = probe_dec.total_duration() {
												s.audio_total_secs = dur.as_secs_f32();
											}
										}
									}

									// Parse duration from message text: "[Voice Message] (M:SS)" or "[Audio] (M:SS)"
									if s.audio_total_secs == 0.0 {
										let playing_id = s.playing_msg_id.clone();
										if let Some(ref pid) = playing_id {
											if let Some(msg) = s.messages.iter().find(|m| &m.id == pid) {
												let text = &msg.text;
												// Find "(M:SS)" pattern inside the tag
												if let Some(open) = text.find('(') {
													if let Some(close) = text[open..].find(')') {
														let inner = &text[open + 1..open + close];
														// parse "M:SS"
														let parts_dur: Vec<&str> = inner.split(':').collect();
														if parts_dur.len() == 2 {
															if let (Ok(m), Ok(ss)) = (
																parts_dur[0].parse::<f32>(),
																parts_dur[1].parse::<f32>(),
															) {
																s.audio_total_secs = m * 60.0 + ss;
															}
														}
													}
												}
											}
										}
									}

									eprintln!("[LOG UI] Audio total duration: {}s", s.audio_total_secs);

									ensure_audio_stream(&mut s);
									let mut played = false;
									if let Some(ref stream_handle) = s.audio_stream {
										let sink = Player::connect_new(&stream_handle.mixer());
										if let Ok(file) = File::open(path) {
											if let Ok(decoder) = Decoder::new(BufReader::new(file)) {
												sink.append(decoder.wsola(s.audio_speed));
												if start_paused {
													sink.pause();
												} else {
													sink.play();
												}
												s.audio_sink = Some(sink);
												played = true;
												s.audio_start_instant =
													if start_paused { None } else { Some(std::time::Instant::now()) };
												parts.conversations_parts.play_voice_btn.set_label(if start_paused {
													"Play voice message"
												} else {
													"Pause voice message"
												});
											} else if let Some(opus_buf) = decode_opus_file(path) {
												sink.append(opus_buf.wsola(s.audio_speed));
												if start_paused {
													sink.pause();
												} else {
													sink.play();
												}
												s.audio_sink = Some(sink);
												played = true;
												s.audio_start_instant =
													if start_paused { None } else { Some(std::time::Instant::now()) };
												parts.conversations_parts.play_voice_btn.set_label(if start_paused {
													"Play voice message"
												} else {
													"Pause voice message"
												});
											}
										}
									}
									if !played {
										use std::io::Write;
										let path_escaped = path.replace("'", "''");
										if s.audio_stdin.is_none() {
											let ps_script = r#"
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.Playback.MediaPlayer, Windows.Media, ContentType=WindowsRuntime]
$p = New-Object Windows.Media.Playback.MediaPlayer
while ($true) {
    $line = [System.Console]::ReadLine()
    if ($null -eq $line -or $line -eq "EXIT") { break }
    if ($line.StartsWith("PLAY ")) {
        $path = $line.Substring(5)
        $p.Source = [Windows.Media.Core.MediaSource]::CreateFromUri([Uri]$path)
        $p.Play()
    } elseif ($line -eq "PAUSE") {
        $p.Pause()
    } elseif ($line -eq "RESUME") {
        $p.Play()
    } elseif ($line.StartsWith("SEEK ")) {
        $sec = [double]($line.Substring(5))
        $p.PlaybackSession.Position = [TimeSpan]::FromSeconds($sec)
    }
}
"#;
											if let Ok(mut child) = std::process::Command::new("powershell")
												.args(&["-NoProfile", "-WindowStyle", "Hidden", "-Command", ps_script])
												.stdin(std::process::Stdio::piped())
												.creation_flags(0x08000000) // CREATE_NO_WINDOW
												.spawn()
											{
												s.audio_stdin = child.stdin.take();
												s.audio_process = Some(child);
											}
										}
										if let Some(ref mut stdin) = s.audio_stdin {
											let _ = writeln!(stdin, "PLAY {}", path_escaped);
											if start_paused {
												let _ = writeln!(stdin, "PAUSE");
											}
											let _ = stdin.flush();
											s.audio_start_instant =
												if start_paused { None } else { Some(std::time::Instant::now()) };
											parts.conversations_parts.play_voice_btn.set_label(if start_paused {
												"Play voice message"
											} else {
												"Pause voice message"
											});
										}
									}
									s.is_audio_paused = start_paused;
									// Show slider UI and speed button
									parts.conversations_parts.audio_slider.show(true);
									parts.conversations_parts.audio_speed_btn.show(true);
									parts.conversations_parts.audio_progress_label.show(false);
									let total = s.audio_total_secs;
									let max_sec = (total as i32).max(1);
									parts.conversations_parts.audio_slider.set_range(0, max_sec);
									parts.conversations_parts.audio_slider.set_value(0);
									let fmt_dur = |v: f32| {
										if v > 0.0 {
											format!("{}:{:02}", v as u32 / 60, v as u32 % 60)
										} else {
											"?:??".to_string()
										}
									};
									parts
										.conversations_parts
										.audio_progress_label
										.set_label(&format!("0:00 / {}", fmt_dur(total)));
									parts.conversations_parts.conversation_panel.layout();
								}
							}
						}
						"attachment_uploading" => {
							// Percentage-level upload progress. The message row itself
							// (text/status) is kept in sync by the "message" event, which
							// the backend sends immediately with a real id and again on
							// completion/failure; this only drives the live announcement
							// and the (usually already-closed) attachment dialog's label.
							if let Some(data) = val.get("data") {
								let filename = data.get("filename").and_then(|f| f.as_str()).unwrap_or("file");
								let percent = data.get("percent").and_then(|p| p.as_i64());
								let msg = if let Some(p) = percent {
									format!("Uploading {} ({}%)...", filename, p)
								} else {
									format!("Uploading {}...", filename)
								};
								parts.attachment_dialog.attachment_info_label.set_label(&msg);
								live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
							}
						}
						"attachment_sent" => {
							// The message row itself is already updated to "sent" by its
							// own "message" event, which the backend always sends first.
							parts.attachment_dialog.attachment_info_label.set_label("");
						}
						_ => {}
					}
				}
			}
		}
		UiCommand::ExitApp => {
			// See the matching comment on the tray menu's Exit handler in
			// ui/window.rs: closing the main frame alone doesn't end the app
			// because other top-level windows (the call window, dialogs) are
			// built once at startup and never destroyed.
			parts.frame.close(true);
			if let Some(app) = wxdragon::get_app_instance() {
				app.exit_main_loop();
			}
		}
		UiCommand::NavSelected(idx) => {
			match idx {
				0 => {
					// Conversations
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 0;
					parts.conversations_parts.new_conv_btn.set_label("New Conversation");
					parts.conversations_parts.post_media_status_btn.show(false);
					parts.conversations_parts.conversations_label.set_label("Conversations");
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				1 => {
					// Status
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 1;
					parts.conversations_parts.new_conv_btn.set_label("Post Status Update...");
					parts.conversations_parts.post_media_status_btn.show(true);
					parts.conversations_parts.conversations_label.set_label("Status Updates");
					let payload = serde_json::json!({ "action": "get_statuses" });
					let _ = state.borrow().ws_tx.send(payload.to_string());
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				2 => {
					// Starred Messages
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 2;
					parts.conversations_parts.new_conv_btn.set_label("New Conversation");
					parts.conversations_parts.post_media_status_btn.show(false);
					parts.conversations_parts.conversations_label.set_label("Starred Messages");
					let payload = serde_json::json!({ "action": "get_starred_messages" });
					let _ = state.borrow().ws_tx.send(payload.to_string());
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				3 => {
					// Archived Chats
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 3;
					parts.conversations_parts.new_conv_btn.set_label("New Conversation");
					parts.conversations_parts.post_media_status_btn.show(false);
					parts.conversations_parts.conversations_label.set_label("Archived Chats");
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				4 => {
					// Communities
					parts.conversations_parts.panel.show(true);
					state.borrow_mut().active_tab = 4;
					parts.conversations_parts.new_conv_btn.set_label("New Conversation");
					parts.conversations_parts.post_media_status_btn.show(false);
					parts.conversations_parts.conversations_label.set_label("Communities");
					apply_chat_filter(&mut state.borrow_mut(), parts);
				}
				_ => {}
			}
			parts.content_panel.layout();
		}
		UiCommand::FilterConversations(query) => {
			let mut s = state.borrow_mut();
			s.search_query = query;
			apply_chat_filter(&mut s, parts);
		}
		UiCommand::OpenConversation(idx) => {
			if state.borrow().active_tab == 1 {
				// Status: selecting a row views that status instead of opening a chat.
				let (poster_jid, message_id) = {
					let s = state.borrow();
					(s.filtered_chats.get(idx).cloned(), s.filtered_status_message_ids.get(idx).cloned())
				};
				if let (Some(poster_jid), Some(message_id)) = (poster_jid, message_id) {
					let mut s = state.borrow_mut();
					let mut media_type = String::new();
					let mut text = String::new();
					let mut poster_name = poster_jid.clone();
					for group in s.status_groups.iter_mut() {
						if group.poster_jid == poster_jid {
							poster_name = group.poster_name.clone();
							if let Some(item) = group.items.iter_mut().find(|it| it.id == message_id) {
								item.viewed = true;
								media_type = item.media_type.clone();
								text = item.text.clone();
							}
							group.all_viewed = group.items.iter().all(|it| it.viewed);
							break;
						}
					}
					apply_chat_filter(&mut s, parts);
					let ws_tx = s.ws_tx.clone();
					drop(s);

					let mark_payload = serde_json::json!({
						"action": "mark_status_viewed",
						"data": { "id": message_id, "poster_jid": poster_jid }
					});
					let _ = ws_tx.send(mark_payload.to_string());

					if !media_type.is_empty() {
						let open_payload = serde_json::json!({
							"action": "open_media",
							"data": { "jid": "status@broadcast", "message_id": message_id }
						});
						let _ = ws_tx.send(open_payload.to_string());
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("Opening {}'s status media...", poster_name),
						);
					} else {
						let msg = if text.is_empty() {
							format!("{}'s status has no content.", poster_name)
						} else {
							format!("{}: {}", poster_name, text)
						};
						live_region::announce(parts.conversations_parts.messages_announcer.clone(), &msg);
					}
				}
				return;
			}
			let jid = state.borrow().filtered_chats.get(idx).cloned();
			if let Some(ref jid) = jid {
				if state.borrow().active_tab == 2 {
					let msg_id = state.borrow().filtered_starred_message_ids.get(idx).cloned();
					if let Some(msg_id) = msg_id {
						state.borrow_mut().search_target_id = Some(msg_id);
					}
				}
				open_chat_for_jid(jid, state, parts);
			}
		}
		UiCommand::OpenChatByPhone(digits) => {
			let jid = format!("{}@s.whatsapp.net", digits);
			open_chat_for_jid(&jid, state, parts);
		}
		UiCommand::OpenChatByJid(jid) => {
			open_chat_for_jid(&jid, state, parts);
		}
		UiCommand::ToggleSelectionMode => {
			let mut s = state.borrow_mut();
			s.selection_mode = !s.selection_mode;
			if s.selection_mode {
				s.selected_message_ids.clear();
				refresh_selection_list(&mut s, parts);
				parts.conversations_parts.messages_list.show(false);
				parts.conversations_parts.messages_select_list.show(true);
				parts.conversations_parts.selection_bar_panel.show(true);
				parts.conversations_parts.message_field.show(false);
				parts.conversations_parts.add_attachment_btn.show(false);
				parts.conversations_parts.send_message_btn.show(false);
				parts.conversations_parts.record_voice_btn.show(false);
			} else {
				s.selected_message_ids.clear();
				s.selection_rows.clear();
				parts.conversations_parts.messages_select_list.show(false);
				parts.conversations_parts.selection_bar_panel.show(false);
				parts.conversations_parts.messages_list.show(true);
				parts.conversations_parts.message_field.show(true);
				parts.conversations_parts.add_attachment_btn.show(true);
				parts
					.conversations_parts
					.send_message_btn
					.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
				parts.conversations_parts.record_voice_btn.show(true);
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::DeleteSelectedMessages { for_everyone } => {
			let (jid_opt, ws_tx, to_delete) = {
				let s = state.borrow();
				let ids = &s.selected_message_ids;
				let msgs: Vec<whantv::ChatMessage> = s
					.selection_rows
					.iter()
					.filter(|item| ids.contains(item.id()))
					.flat_map(|item| item.messages())
					.collect();
				(s.selected_chat.clone(), s.ws_tx.clone(), msgs)
			};
			if let Some(jid) = jid_opt {
				{
					let mut s = state.borrow_mut();
					for msg in &to_delete {
						s.bulk_delete_pending.insert((jid.clone(), msg.id.clone()));
					}
				}
				for msg in &to_delete {
					let sender_jid = if msg.from_me { "me".to_string() } else { msg.sender_jid.clone() };
					let payload = serde_json::json!({
						"action": "delete_message",
						"data": {
							"jid": jid,
							"message_id": msg.id,
							"sender_jid": sender_jid,
							"for_everyone": for_everyone
						}
					});
					let _ = ws_tx.send(payload.to_string());
				}
			}

			let mut s = state.borrow_mut();
			s.selection_mode = false;
			s.selected_message_ids.clear();
			s.selection_rows.clear();
			drop(s);
			parts.conversations_parts.messages_select_list.show(false);
			parts.conversations_parts.selection_bar_panel.show(false);
			parts.conversations_parts.messages_list.show(true);
			parts.conversations_parts.message_field.show(true);
			parts.conversations_parts.add_attachment_btn.show(true);
			parts.conversations_parts.record_voice_btn.show(true);
			parts
				.conversations_parts
				.send_message_btn
				.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
			parts.conversations_parts.conversation_panel.layout();
			if !to_delete.is_empty() {
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Selected messages deleted.",
				);
			}
		}
		UiCommand::LoadMoreMessages => {
			let mut s = state.borrow_mut();
			if s.history_loading {
				eprintln!("[LOG UI] Request already in flight for chat, skipping duplicate trigger");
				return;
			}
			if let Some(ref jid) = s.selected_chat.clone() {
				if s.completed_history_chats.contains(jid) {
					eprintln!("[LOG UI] All available history already loaded for chat {}, skipping request", jid);
					return;
				}
				s.history_loading = true;
				eprintln!("[LOG UI] Reached top of chat! Requesting older history from phone for: {}", jid);
				let oldest_msg_id = s.messages.first().map(|m| m.id.clone()).unwrap_or_default();
				let payload = serde_json::json!({
					"action": "request_history_sync",
					"data": { "jid": jid, "message_id": oldest_msg_id, "count": 50 }
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}

		UiCommand::SearchInConversation(query) => {
			if let Some(ref jid) = state.borrow().selected_chat {
				let payload = serde_json::json!({
					"action": "search_messages",
					"data": { "jid": jid, "query": query }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::ToggleSearchPanel => {
			if state.borrow().selected_chat.is_none() {
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), "No chat open.");
				return;
			}
			let visible = parts.conversations_parts.search_panel.is_shown();
			parts.conversations_parts.search_panel.show(!visible);
			if !visible {
				parts.conversations_parts.search_conv_field.set_value("");
				parts.conversations_parts.search_conv_field.set_focus();
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::PromptLockedChats => {
			let dlg = TextEntryDialog::builder(&parts.frame, "Enter Secret Code:", "Locked Chats").build();
			if dlg.show_modal() == wxdragon::id::ID_OK {
				if let Some(val) = dlg.get_value() {
					let text = val.trim().to_string();
					if !text.is_empty() {
						let payload = serde_json::json!({
							"action": "verify_chat_lock_code",
							"data": { "code": text }
						});
						let _ = state.borrow().ws_tx.send(payload.to_string());
					}
				}
			}
		}
		UiCommand::CloseLockedChats => {
			let mut s = state.borrow_mut();
			if s.chats_unlocked {
				s.chats_unlocked = false;
				apply_chat_filter(&mut s, parts);
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Locked chats hidden.");
			}
		}
		UiCommand::CloseSearch => {
			// Hide search panel and reload the real chat messages
			parts.conversations_parts.search_panel.show(false);
			parts.conversations_parts.search_conv_field.set_value("");
			if let Some(jid) = state.borrow().selected_chat.clone() {
				let limit = 9999999;
				let payload = serde_json::json!({
					"action": "get_messages",
					"data": { "jid": jid, "limit": limit, "offset": 0 }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::JumpToSearchResult { message_id, message_text } => {
			// Hide search, set the target so get_messages scrolls to it, reload chat
			parts.conversations_parts.search_panel.show(false);
			parts.conversations_parts.search_conv_field.set_value("");
			{
				let mut s = state.borrow_mut();
				if !message_id.is_empty() {
					s.search_target_id = Some(message_id);
				} else if !message_text.is_empty() {
					s.search_target_text = Some(message_text);
				}
				if let Some(jid) = s.selected_chat.clone() {
					let limit = 9999999;
					let payload = serde_json::json!({
						"action": "get_messages",
						"data": { "jid": jid, "limit": limit, "offset": 0 }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::StartRecording => {
			let mut s = state.borrow_mut();
			if s.selected_chat.is_some() && !s.is_voice_recording && !s.voice_recording_opening {
				let temp_dir = std::env::temp_dir();
				let ts =
					std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
				let wav_path = temp_dir.join(format!("voice_rec_{}_0.wav", ts));
				let wav_path_str = wav_path.to_string_lossy().to_string();

				s.voice_recording_opening = true;
				s.voice_recording_generation += 1;
				let generation = s.voice_recording_generation;
				let pending = s.voice_recording_pending.clone();
				let ui_tx = s.ui_tx.clone();
				let wav_path_for_thread = wav_path_str.clone();

				parts.conversations_parts.record_voice_btn.enable(false);

				voice_recorder::RecordingSession::start_async(
					&wav_path_str,
					s.settings.input_device_name.clone(),
					s.settings.voice_message_noise_suppression,
					move |result| {
						*pending.lock().unwrap() = Some(PendingVoiceRecording {
							generation,
							wav_path: wav_path_for_thread,
							is_resume: false,
							result,
						});
						let _ = ui_tx.send(UiCommand::VoiceRecordingReady);
					},
				);
			}
		}
		UiCommand::StopRecording => {
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				"Voice recording stopped. Ready to send.",
			);
		}
		UiCommand::TogglePauseRecording => {
			let mut s = state.borrow_mut();
			if s.is_voice_recording && !s.voice_recording_opening {
				if s.is_voice_paused {
					// Resume by starting a brand new segment rather than a
					// pause/resume on the same capture session - segments are
					// spliced together on send.
					let temp_dir = std::env::temp_dir();
					let ts = std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.unwrap_or_default()
						.as_secs();
					let seg_idx = s.voice_recording_segments.len();
					let wav_path = temp_dir.join(format!("voice_rec_{}_{}.wav", ts, seg_idx));
					let wav_path_str = wav_path.to_string_lossy().to_string();

					s.voice_recording_opening = true;
					s.voice_recording_generation += 1;
					let generation = s.voice_recording_generation;
					let pending = s.voice_recording_pending.clone();
					let ui_tx = s.ui_tx.clone();
					let wav_path_for_thread = wav_path_str.clone();

					parts.conversations_parts.pause_resume_btn.enable(false);

					voice_recorder::RecordingSession::start_async(
						&wav_path_str,
						s.settings.input_device_name.clone(),
						s.settings.voice_message_noise_suppression,
						move |result| {
							*pending.lock().unwrap() = Some(PendingVoiceRecording {
								generation,
								wav_path: wav_path_for_thread,
								is_resume: true,
								result,
							});
							let _ = ui_tx.send(UiCommand::VoiceRecordingReady);
						},
					);
				} else {
					// Pause by closing off the current segment as a
					// complete, valid WAV file.
					if let Some(path) = s.voice_recording_path.take() {
						if let Some(session) = s.voice_recording_session.take() {
							session.finish();
						}
						s.voice_recording_segments.push(path);
					}
					s.is_voice_paused = true;
					s.voice_pause_instant = Some(std::time::Instant::now());
					// Unlike Discard/Send, pausing never told the backend the
					// composing state had stopped - the recipient kept seeing
					// "recording audio..." for the whole pause, not just until
					// WhatsApp's own protocol-level timeout cleared it.
					if let Some(ref jid) = s.selected_chat {
						let payload = serde_json::json!({
							"action": "send_typing",
							"data": { "jid": jid, "state": "paused", "media": "" }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
					parts.conversations_parts.pause_resume_btn.set_label("Resume recording");
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Voice recording paused.",
					);
				}
			}
		}
		UiCommand::VoiceRecordingReady => {
			let pending = {
				let s = state.borrow();
				s.voice_recording_pending.lock().unwrap().take()
			};
			let Some(pending) = pending else {
				return;
			};

			let mut s = state.borrow_mut();
			if pending.generation != s.voice_recording_generation {
				// Stale: the recording was discarded/sent (or another open
				// was kicked off) before this device-open finished. Clean up
				// quietly instead of resurrecting recording state.
				drop(s);
				if let Ok(session) = pending.result {
					session.finish();
				}
				let _ = std::fs::remove_file(&pending.wav_path);
				return;
			}
			s.voice_recording_opening = false;

			if pending.is_resume {
				parts.conversations_parts.pause_resume_btn.enable(true);
				match pending.result {
					Ok(session) => {
						s.voice_recording_path = Some(pending.wav_path);
						s.voice_recording_session = Some(session);
						s.is_voice_paused = false;
						if let Some(inst) = s.voice_pause_instant.take() {
							s.voice_paused_duration += inst.elapsed();
						}
						if let Some(ref jid) = s.selected_chat {
							let payload = serde_json::json!({
								"action": "send_typing",
								"data": { "jid": jid, "state": "composing", "media": "audio" }
							});
							let _ = s.ws_tx.send(payload.to_string());
						}
						parts.conversations_parts.pause_resume_btn.set_label("Pause recording");
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Voice recording resumed.",
						);
					}
					Err(e) => {
						eprintln!("[LOG UI] Failed to resume voice recording: {:?}", e);
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Failed to resume recording.",
						);
					}
				}
			} else {
				parts.conversations_parts.record_voice_btn.enable(true);
				match pending.result {
					Ok(session) => {
						s.is_voice_recording = true;
						s.is_voice_paused = false;
						s.voice_recording_path = Some(pending.wav_path);
						s.voice_recording_session = Some(session);
						s.voice_recording_segments.clear();
						s.voice_recording_start = Some(std::time::Instant::now());
						s.voice_paused_duration = std::time::Duration::ZERO;
						s.voice_pause_instant = None;

						if let Some(ref jid) = s.selected_chat {
							let payload = serde_json::json!({
								"action": "send_typing",
								"data": { "jid": jid, "state": "composing", "media": "audio" }
							});
							let _ = s.ws_tx.send(payload.to_string());
						}

						parts.conversations_parts.message_field.show(false);
						parts.conversations_parts.add_attachment_btn.show(false);
						parts.conversations_parts.send_message_btn.show(false);
						parts.conversations_parts.record_voice_btn.show(false);
						parts.conversations_parts.discard_voice_btn.show(true);
						parts.conversations_parts.pause_resume_btn.show(true);
						parts.conversations_parts.send_voice_btn.show(true);
						parts.conversations_parts.pause_resume_btn.set_label("Pause recording");
						parts.conversations_parts.conversation_panel.layout();
						parts.conversations_parts.send_voice_btn.set_focus();
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Recording voice message...",
						);
					}
					Err(e) => {
						eprintln!("[LOG UI] Failed to start voice recording: {:?}", e);
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Failed to start microphone recording.",
						);
					}
				}
			}
		}
		UiCommand::ForceSync => {
			let _ = state.borrow().ws_tx.send(serde_json::json!({ "action": "force_sync" }).to_string());
		}
		UiCommand::SendVoiceMessage => {
			let (
				segments,
				rec_start,
				paused_dur,
				pause_inst,
				is_paused,
				jid_opt,
				sounds_enabled,
				ws_tx,
				reply_id,
				reply_sender,
			) = {
				let mut s = state.borrow_mut();
				s.is_voice_recording = false;
				// Invalidate a resume's device-open still in flight, if any -
				// see the matching comment in DiscardRecording.
				s.voice_recording_generation += 1;
				s.voice_recording_opening = false;
				// If still actively recording (not paused), close off the
				// final segment as a complete, valid WAV file.
				if let Some(path) = s.voice_recording_path.take() {
					if let Some(session) = s.voice_recording_session.take() {
						session.finish();
					}
					s.voice_recording_segments.push(path);
				}
				let reply_id = s.pending_reply_id.take().unwrap_or_default();
				let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
				s.pending_reply_preview = None;
				(
					std::mem::take(&mut s.voice_recording_segments),
					s.voice_recording_start.take(),
					s.voice_paused_duration,
					s.voice_pause_instant.take(),
					s.is_voice_paused,
					s.selected_chat.clone(),
					s.settings.sounds_enabled,
					s.ws_tx.clone(),
					reply_id,
					reply_sender,
				)
			};
			parts.conversations_parts.remove_quote_btn.show(false);

			if let Some(ref jid) = jid_opt {
				let payload = serde_json::json!({
					"action": "send_typing",
					"data": { "jid": jid, "state": "paused", "media": "" }
				});
				let _ = ws_tx.send(payload.to_string());
			}

			if !segments.is_empty() {
				if let (Some(start_inst), Some(jid)) = (rec_start, jid_opt) {
					let total_elapsed = start_inst.elapsed();
					let total_paused = paused_dur
						+ if is_paused {
							pause_inst.map(|i| i.elapsed()).unwrap_or_default()
						} else {
							std::time::Duration::ZERO
						};
					let active_secs = total_elapsed.saturating_sub(total_paused).as_secs().max(1);

					thread::spawn(move || {
						// Splice pause/resume segments into one continuous
						// WAV before encoding (a single-segment recording,
						// the common case, needs no splicing).
						let is_multi_segment = segments.len() > 1;
						let wav_path = if !is_multi_segment {
							segments[0].clone()
						} else {
							let merged_path = segments[0].replace(".wav", "_merged.wav");
							let result = concat_wav_segments(&segments, &merged_path);
							// The originals are fully absorbed into
							// `merged_path` now (or, on failure, no longer
							// useful) - clean them up either way.
							for seg in &segments {
								let _ = std::fs::remove_file(seg);
							}
							match result {
								Ok(()) => merged_path,
								Err(e) => {
									eprintln!("[LOG UI] Failed to splice voice recording segments: {}", e);
									String::new()
								}
							}
						};
						if wav_path.is_empty() {
							eprintln!("[LOG UI] No usable audio to send for voice message");
							return;
						}

						let ogg_path = wav_path.replace(".wav", ".ogg");
						let (final_path, mimetype) = match convert_wav_to_opus_ogg(&wav_path, &ogg_path) {
							Ok(_) => {
								// wav_path is only an intermediate once
								// converted; if it's the sole segment itself
								// leave it (matches the single-segment path,
								// which never deletes the source recording).
								if is_multi_segment {
									let _ = std::fs::remove_file(&wav_path);
								}
								(ogg_path, "audio/ogg; codecs=opus")
							}
							Err(e) => {
								eprintln!("[LOG UI] Error converting wav to opus natively: {}", e);
								(wav_path.clone(), "audio/wav")
							}
						};

						let payload = serde_json::json!({
							"action": "send_voice_message",
							"data": {
								"jid": jid,
								"path": final_path,
								"mimetype": mimetype,
								"duration": active_secs,
								"reply_to_id": reply_id,
								"reply_to_sender": reply_sender
							}
						});
						let _ = ws_tx.send(payload.to_string());
					});

					play_notification_sound(NotificationSound::VoiceMessageSent, sounds_enabled);
					live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice message sent.");
				}
			}

			parts.conversations_parts.message_field.show(true);
			parts.conversations_parts.add_attachment_btn.show(true);
			parts.conversations_parts.record_voice_btn.show(true);
			parts.conversations_parts.record_voice_btn.enable(true);
			parts.conversations_parts.pause_resume_btn.enable(true);
			parts
				.conversations_parts
				.send_message_btn
				.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
			parts.conversations_parts.message_field.set_focus();

			parts.conversations_parts.discard_voice_btn.show(false);
			parts.conversations_parts.pause_resume_btn.show(false);
			parts.conversations_parts.send_voice_btn.show(false);

			parts.conversations_parts.conversation_panel.layout();
		}
		UiCommand::DiscardRecording | UiCommand::DiscardVoiceMessage => {
			{
				let mut s = state.borrow_mut();
				s.is_voice_recording = false;
				s.is_voice_paused = false;
				// Invalidate any device-open still in flight (a resume, or the
				// initial start) - its result will arrive later via
				// VoiceRecordingReady and be cleaned up there instead of
				// resurrecting the recording state we're tearing down here.
				s.voice_recording_generation += 1;
				s.voice_recording_opening = false;
				// The in-progress segment's capture session (if any) is still
				// writing to disk - stop it, then clean up its file along with
				// any earlier segments from prior pause cycles.
				if let Some(session) = s.voice_recording_session.take() {
					session.finish();
				}
				if let Some(path) = s.voice_recording_path.take() {
					let _ = std::fs::remove_file(path);
				}
				for seg in s.voice_recording_segments.drain(..) {
					let _ = std::fs::remove_file(seg);
				}
				if let Some(ref jid) = s.selected_chat {
					let payload = serde_json::json!({
						"action": "send_typing",
						"data": { "jid": jid, "state": "paused", "media": "" }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}

			parts.conversations_parts.message_field.show(true);
			parts.conversations_parts.add_attachment_btn.show(true);
			parts.conversations_parts.record_voice_btn.show(true);
			parts.conversations_parts.record_voice_btn.enable(true);
			parts.conversations_parts.pause_resume_btn.enable(true);
			parts
				.conversations_parts
				.send_message_btn
				.show(!parts.conversations_parts.message_field.get_value().trim().is_empty());
			parts.conversations_parts.message_field.set_focus();

			parts.conversations_parts.discard_voice_btn.show(false);
			parts.conversations_parts.pause_resume_btn.show(false);
			parts.conversations_parts.send_voice_btn.show(false);

			parts.conversations_parts.conversation_panel.layout();
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Voice recording discarded.");
		}
		UiCommand::AddAttachment => {
			let file_dialog = FileDialog::builder(&parts.frame)
				.with_message("Choose file(s) to attach")
				.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist | FileDialogStyle::Multiple)
				.build();
			if file_dialog.show_modal() == wxdragon::id::ID_OK {
				let paths = file_dialog.get_paths();
				if !paths.is_empty() {
					let mut s = state.borrow_mut();
					for path in paths {
						if !path.is_empty() {
							s.pending_attachments.push(path.clone());
						}
					}
					let file_names: Vec<String> = s
						.pending_attachments
						.iter()
						.map(|p| std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string())
						.collect();
					let label_text = format!("Files: {}", file_names.join(", "));
					drop(s);
					parts.attachment_dialog.attachment_info_label.set_label(&label_text);
					parts.attachment_dialog.caption_field.set_value("");
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Files attached: {}", file_names.len()),
					);

					if !parts.attachment_dialog.dialog.is_shown() {
						parts.attachment_dialog.caption_field.set_focus();
						parts.attachment_dialog.dialog.show_modal();
					} else {
						parts.attachment_dialog.caption_field.set_focus();
					}
				}
			}
		}
		UiCommand::PasteAttachment => {
			if let Some(_locker) = wxdragon::prelude::Clipboard::get().locker() {
				let mut fdo = wxdragon::prelude::FileDataObject::new();
				if wxdragon::prelude::Clipboard::get().get_data(&mut fdo) {
					let paths = fdo.get_files();
					if !paths.is_empty() {
						let mut s = state.borrow_mut();
						for path in paths {
							if !path.is_empty() {
								s.pending_attachments.push(path.clone());
							}
						}
						let file_names: Vec<String> = s
							.pending_attachments
							.iter()
							.map(|p| {
								std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string()
							})
							.collect();
						let label_text = format!("Files: {}", file_names.join(", "));
						drop(s);
						parts.attachment_dialog.attachment_info_label.set_label(&label_text);
						parts.attachment_dialog.caption_field.set_value("");
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							&format!("Pasted files: {}", file_names.len()),
						);

						if !parts.attachment_dialog.dialog.is_shown() {
							parts.attachment_dialog.caption_field.set_focus();
							parts.attachment_dialog.dialog.show_modal();
						} else {
							parts.attachment_dialog.caption_field.set_focus();
						}
					}
				}
			}
		}
		UiCommand::DiscardAttachment => {
			{
				let mut s = state.borrow_mut();
				s.pending_attachments.clear();
			}
			parts.attachment_dialog.attachment_info_label.set_label("");
			parts.attachment_dialog.caption_field.set_value("");
			parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_CANCEL);
		}
		UiCommand::SendAttachment { caption } => {
			let jid = state.borrow().selected_chat.clone();
			let (paths, reply_id, reply_sender) = {
				let mut s = state.borrow_mut();
				let paths = std::mem::take(&mut s.pending_attachments);
				let reply_id = s.pending_reply_id.take().unwrap_or_default();
				let reply_sender = s.pending_reply_sender.take().unwrap_or_default();
				s.pending_reply_preview = None;
				(paths, reply_id, reply_sender)
			};
			parts.conversations_parts.remove_quote_btn.show(false);
			if let Some(ref jid) = jid {
				// The backend persists a "uploading" row and broadcasts it
				// immediately, before the upload even starts, so the local
				// echo comes back through the normal "message" event handler
				// with a real id from the start instead of a client-side
				// placeholder here.
				for (idx, path) in paths.iter().enumerate() {
					let ext =
						std::path::Path::new(path).extension().unwrap_or_default().to_string_lossy().to_lowercase();
					let file_type = match ext.as_str() {
						"jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => "image",
						"mp4" | "avi" | "mov" | "mkv" | "webm" | "3gp" => "video",
						_ => "document",
					};
					let filename =
						std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().to_string();

					// Only the first attachment in a multi-file batch carries the
					// reply-to context, so a reply doesn't get quoted once per file.
					let payload = serde_json::json!({
						"action": "send_attachment",
						"data": {
							"jid": jid,
							"type": file_type,
							"path": path,
							"filename": filename,
							"caption": caption,
							"reply_to_id": if idx == 0 { reply_id.as_str() } else { "" },
							"reply_to_sender": if idx == 0 { reply_sender.as_str() } else { "" }
						}
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
				parts.attachment_dialog.attachment_info_label.set_label("");
				parts.attachment_dialog.caption_field.set_value("");
				parts.attachment_dialog.dialog.end_modal(wxdragon::id::ID_OK);
			}
		}
		UiCommand::DownloadMedia => {
			let (to_start, to_cancel, jid_opt) = {
				let s = state.borrow();
				let sel = parts.conversations_parts.messages_list.get_selection();
				if let Some(idx) = sel {
					if let Some(item) = s.display_items.get(idx as usize) {
						let pending: Vec<(String, String)> = item
							.messages()
							.into_iter()
							.filter(|m| !m.is_media_downloaded())
							.map(|m| (m.id.clone(), m.display_filename()))
							.collect();
						let (in_progress, not_started): (Vec<_>, Vec<_>) =
							pending.into_iter().partition(|(id, _)| s.downloads_in_progress.contains_key(id));
						(not_started, in_progress, s.selected_chat.clone())
					} else {
						(Vec::new(), Vec::new(), None)
					}
				} else {
					(Vec::new(), Vec::new(), None)
				}
			};

			// Already downloading: clicking again means "cancel", not "start over".
			if let (Some(jid), false) = (jid_opt.clone(), to_cancel.is_empty()) {
				for (msg_id, _) in &to_cancel {
					let payload = serde_json::json!({
						"action": "cancel_download",
						"data": { "jid": jid, "message_id": msg_id }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
			}

			if let (Some(jid), false) = (jid_opt, to_start.is_empty()) {
				let count = to_start.len();
				{
					let mut s = state.borrow_mut();
					for (msg_id, _) in &to_start {
						s.downloads_in_progress.insert(msg_id.clone(), 0);
					}
				}
				parts.conversations_parts.conversation_panel.freeze();
				if update_action_buttons(&state.borrow(), &parts.conversations_parts) {
					parts.conversations_parts.conversation_panel.layout();
				}
				parts.conversations_parts.conversation_panel.thaw();

				for (msg_id, _) in &to_start {
					let payload = serde_json::json!({
						"action": "download_media",
						"data": { "jid": jid, "message_id": msg_id }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
				}
				if count == 1 {
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Downloading {}", to_start[0].1),
					);
				} else {
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						&format!("Downloading album of {} files...", count),
					);
				}
			}
		}
		UiCommand::OpenMedia => {
			let s = state.borrow();
			if let (Some(jid), Some(idx)) =
				(s.selected_chat.clone(), parts.conversations_parts.messages_list.get_selection())
			{
				if let Some(item) = s.display_items.get(idx as usize) {
					if let Some(msg) = item.messages().iter().find(|m| m.is_media_downloaded()) {
						let payload = serde_json::json!({
							"action": "open_media",
							"data": { "jid": jid, "message_id": msg.id }
						});
						let _ = s.ws_tx.send(payload.to_string());
					}
				}
			}
		}
		UiCommand::SaveMediaAs => {
			let item_opt = {
				let s = state.borrow();
				let sel = parts.conversations_parts.messages_list.get_selection();
				sel.and_then(|idx| s.display_items.get(idx as usize).cloned())
			};

			if let Some(item) = item_opt {
				match item {
					whantv::DisplayItem::Single(msg) => {
						// Grab an owned copy and drop the borrow before opening the
						// modal dialog below — show_modal() pumps the message loop,
						// and any handler that needs state.borrow_mut() while we're
						// still holding this Ref would panic with a BorrowMutError.
						let jid_opt = state.borrow().selected_chat.clone();
						if let Some(jid) = jid_opt {
							let filename = msg.display_filename();
							let dialog = FileDialog::builder(&parts.frame)
								.with_message("Save file as")
								.with_default_file(&filename)
								.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
								.build();
							if dialog.show_modal() == wxdragon::id::ID_OK {
								if let Some(path) = dialog.get_path() {
									if msg.is_media_downloaded() {
										let _ = std::fs::copy(&msg.media_path, &path);
										live_region::announce(
											parts.conversations_parts.messages_announcer.clone(),
											"File saved successfully.",
										);
									} else {
										let payload = serde_json::json!({
											"action": "save_media_as",
											"data": { "jid": jid, "message_id": msg.id, "dest_path": path }
										});
										let _ = state.borrow().ws_tx.send(payload.to_string());
									}
								}
							}
						}
					}
					whantv::DisplayItem::Album { messages, .. } => {
						let photo_count = messages
							.iter()
							.filter(|m| {
								m.media_type == "image"
									|| m.text.starts_with("[Photo]")
									|| m.text.starts_with("[Image]")
							})
							.count();
						let video_count = messages
							.iter()
							.filter(|m| m.media_type == "video" || m.text.starts_with("[Video]"))
							.count();
						let (dlg_title, noun) = if photo_count > 0 && video_count == 0 {
							("Select folder to save all photos into", "photos")
						} else if video_count > 0 && photo_count == 0 {
							("Select folder to save all videos into", "videos")
						} else {
							("Select folder to save all media into", "media files")
						};
						let dir_dialog = wxdragon::DirDialog::builder(&parts.frame, dlg_title, "").build();
						if dir_dialog.show_modal() == wxdragon::id::ID_OK {
							if let Some(picked_path) = dir_dialog.get_path() {
								let dest_dir = std::path::PathBuf::from(&picked_path);
								let mut saved_count = 0;
								for (idx, msg) in messages.iter().enumerate() {
									if msg.is_media_downloaded() {
										let src = std::path::Path::new(&msg.media_path);
										let ext = src
											.extension()
											.and_then(|e| e.to_str())
											.unwrap_or(if msg.media_type == "video" { "mp4" } else { "jpg" });
										let fn_str = if !msg.media_filename.is_empty() {
											msg.media_filename.clone()
										} else {
											let prefix = if msg.media_type == "video" { "video" } else { "photo" };
											format!("{}_{:02}.{}", prefix, idx + 1, ext)
										};
										let dest_file = dest_dir.join(&fn_str);
										if std::fs::copy(src, &dest_file).is_ok() {
											saved_count += 1;
										}
									}
								}
								live_region::announce(
									parts.conversations_parts.messages_announcer.clone(),
									&format!("Saved {} {} to folder.", saved_count, noun),
								);
							}
						}
					}
				}
			}
		}
		UiCommand::CancelEdit => {
			parts.conversations_parts.message_field.set_value("");
			parts.conversations_parts.cancel_edit_btn.show(false);
			parts.conversations_parts.conversation_panel.layout();
			parts.conversations_parts.message_field.set_focus();
		}
		UiCommand::ShowConversationData => {
			let s = state.borrow();
			if let Some(ref jid) = s.selected_chat {
				if jid.ends_with("@g.us") {
					let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
					parts.group_data_dialog.group_name_field.set_value(&local_name);
					parts.group_data_dialog.members_list.clear();
					parts.group_data_dialog.media_list.clear();
					parts.group_data_dialog.documents_list.clear();
					parts.group_data_dialog.links_list.clear();
					parts.group_data_dialog.groups_list.clear();

					let payload = serde_json::json!({
						"action": "get_group_info",
						"data": { "jid": jid }
					});
					let _ = s.ws_tx.send(payload.to_string());
					drop(s);
					state.borrow_mut().group_data_subgroup_jids.clear();

					parts.group_data_dialog.dialog.show_modal();
				} else {
					let local_name = s.contacts.get(jid).cloned().unwrap_or_else(|| jid.clone());
					let raw_phone = jid.split('@').next().unwrap_or(jid);
					let mut phone = raw_phone.to_string();
					if let Ok(parsed) = phonenumber::parse(None, format!("+{}", raw_phone)) {
						phone = parsed.format().mode(phonenumber::Mode::International).to_string();
					}

					let initial_text = format!(
						"Name: {}\nPhone: {}\nWhatsApp Name: Loading...\nAbout/Status: Loading...",
						local_name, phone
					);
					parts.conversation_data_dialog.info_field.set_value(&initial_text);

					let payload = serde_json::json!({
						"action": "get_user_info",
						"data": { "jid": jid }
					});
					let _ = s.ws_tx.send(payload.to_string());

					let media = build_media_messages(&s.messages);
					let documents = build_document_messages(&s.messages);
					let links = build_link_messages(&s.messages);
					drop(s);

					populate_message_list(&parts.conversation_data_dialog.media_list, &media, |m| m.display_filename());
					populate_message_list(&parts.conversation_data_dialog.documents_list, &documents, |m| {
						m.display_filename()
					});
					populate_message_list(&parts.conversation_data_dialog.links_list, &links, |m| {
						m.first_link().unwrap_or_default()
					});

					let mut s_mut = state.borrow_mut();
					s_mut.conversation_data_media = media;
					s_mut.conversation_data_documents = documents;
					s_mut.conversation_data_links = links;
					drop(s_mut);

					parts.conversation_data_dialog.info_field.set_focus();
					parts.conversation_data_dialog.dialog.show_modal();
				}
			}
		}
		UiCommand::RemoveQuote => {
			clear_pending_reply(&mut state.borrow_mut(), parts);
			parts.conversations_parts.conversation_panel.layout();
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Quoted message removed.");
			parts.conversations_parts.message_field.set_focus();
		}
		UiCommand::SendTyping { jid, state: typing_state, media } => {
			let target_jid = if jid.is_empty() { state.borrow().selected_chat.clone() } else { Some(jid) };
			if let Some(target) = target_jid {
				let payload = serde_json::json!({
					"action": "send_typing",
					"data": {
						"jid": target,
						"state": typing_state,
						"media": media
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::ForwardMessage { message_id, to_jids } => {
			let payload = serde_json::json!({
				"action": "forward_message",
				"data": {
					"message_id": message_id,
					"to_jids": to_jids
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Message forwarded.");
		}
		UiCommand::PlaceCall => {
			let jid = {
				let s = state.borrow();
				if let Some(j) = s.selected_chat.as_ref() {
					j.clone()
				} else {
					return;
				}
			};
			let msg = serde_json::json!({
				"action": "place_call",
				"data": { "jid": jid }
			});
			let mut s = state.borrow_mut();
			let _ = s.ws_tx.send(msg.to_string());
			s.call_placing = true;
			s.call_pending_cancel = false;

			let is_group = jid.ends_with("@g.us");
			let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
			if is_group {
				parts.call_parts.status_label.set_label(&format!("Starting group call in {}", name));
			} else {
				parts.call_parts.status_label.set_label(&format!("In call with {}", name));
			}
			parts.call_parts.accept_btn.show(false);
			parts.call_parts.reject_btn.show(false);
			parts.call_parts.mute_btn.show(true);
			// Shown by call_connected - WhatsApp only accepts an invite into an
			// established call, so offering it while ringing just fails.
			parts.call_parts.add_participant_btn.show(false);
			parts.call_parts.leave_btn.show(true);
			parts.call_parts.frame.show(true);
			drop(s);

			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				if is_group { "Placing group call..." } else { "Placing call..." },
			);
		}
		UiCommand::AnswerCall => {
			let mut s = state.borrow_mut();
			if let Some(call_id) = &s.active_call_id {
				let msg = serde_json::json!({
					"action": "answer_call",
					"data": { "call_id": call_id }
				});
				let _ = s.ws_tx.send(msg.to_string());
				stop_call_ringing_sound(&mut s);
				s.call_answer_deadline = Some(std::time::Instant::now());
				parts.call_parts.status_label.set_label("Answering...");
			}
		}
		UiCommand::RejectCall => {
			let mut s = state.borrow_mut();
			if let Some(call_id) = &s.active_call_id {
				let msg = serde_json::json!({
					"action": "reject_call",
					"data": { "call_id": call_id }
				});
				let _ = s.ws_tx.send(msg.to_string());
				parts.call_parts.frame.show(false);
				s.active_call_id = None;
				s.call_participants.clear();
				s.call_answer_deadline = None;
				s.incoming_call_caller = None;
				stop_call_ringing_sound(&mut s);
				play_call_fail_sound(&mut s);
			}
		}
		UiCommand::LeaveCall => {
			let mut s = state.borrow_mut();
			if let Some(call_id) = &s.active_call_id {
				let msg = serde_json::json!({
					"action": "leave_call",
					"data": { "call_id": call_id }
				});
				let _ = s.ws_tx.send(msg.to_string());
				parts.call_parts.frame.show(false);
				s.active_call_id = None;
				s.active_call_audio = None;
				s.call_participants.clear();
				s.call_answer_deadline = None;
				stop_call_ringing_sound(&mut s);
				play_call_fail_sound(&mut s);
			} else if s.call_placing {
				// We're in the window between sending place_call and hearing back
				// call_placed/call_failed with a real call_id - there's nothing to
				// send leave_call for yet. Hide the UI now for immediate feedback,
				// and mark the cancel as pending so the call_placed handler hangs
				// up as soon as the call_id actually arrives instead of leaving it
				// ringing on the peer's end.
				s.call_pending_cancel = true;
				parts.call_parts.frame.show(false);
				stop_call_ringing_sound(&mut s);
			}
		}
		UiCommand::FilterGroupMembers(query) => {
			let mut s = state.borrow_mut();
			let lower_query = query.to_lowercase();
			parts.group_data_dialog.members_list.clear();
			let mut visible_jids = Vec::new();
			for (i, member) in s.group_data_members.iter().enumerate() {
				if lower_query.is_empty() || member.to_lowercase().contains(&lower_query) {
					parts.group_data_dialog.members_list.append(member);
					visible_jids.push(s.group_data_member_jids[i].clone());
				}
			}
			s.group_data_visible_member_jids = visible_jids;
			drop(s);
			// Filtering rebuilds the list from scratch, so any prior selection
			// is gone - reset the label to the no-selection default.
			parts.group_data_dialog.make_admin_btn.set_label("Make admin");
		}
		UiCommand::GroupMemberSelectionChanged => {
			update_make_admin_button_label(state, parts);
		}
		UiCommand::DownloadMediaMessage(message_id) => {
			let (jid_opt, needs_download, filename) = {
				let s = state.borrow();
				let msg = s.messages.iter().find(|m| m.id == message_id);
				(
					s.selected_chat.clone(),
					msg.map(|m| !m.is_media_downloaded()).unwrap_or(false),
					msg.map(|m| m.display_filename()).unwrap_or_default(),
				)
			};
			if let (Some(jid), true) = (jid_opt, needs_download) {
				state.borrow_mut().downloads_in_progress.insert(message_id.clone(), 0);
				let payload = serde_json::json!({
					"action": "download_media",
					"data": { "jid": jid, "message_id": message_id }
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					&format!("Downloading {}", filename),
				);
			}
		}
		UiCommand::OpenMediaMessage(message_id) => {
			let s = state.borrow();
			if let Some(jid) = s.selected_chat.clone() {
				if s.messages.iter().any(|m| m.id == message_id && m.is_media_downloaded()) {
					let payload = serde_json::json!({
						"action": "open_media",
						"data": { "jid": jid, "message_id": message_id }
					});
					let _ = s.ws_tx.send(payload.to_string());
				}
			}
		}
		UiCommand::SaveMediaMessageAs(message_id) => {
			// Grab an owned copy and drop the borrow before opening the modal
			// save dialog below - show_modal() pumps the message loop, and any
			// handler that needs state.borrow_mut() while we're still holding
			// this Ref would panic with a BorrowMutError.
			let (jid_opt, msg_opt) = {
				let s = state.borrow();
				(s.selected_chat.clone(), s.messages.iter().find(|m| m.id == message_id).cloned())
			};
			if let (Some(jid), Some(msg)) = (jid_opt, msg_opt) {
				let filename = msg.display_filename();
				let dialog = FileDialog::builder(&parts.frame)
					.with_message("Save file as")
					.with_default_file(&filename)
					.with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
					.build();
				if dialog.show_modal() == wxdragon::id::ID_OK {
					if let Some(path) = dialog.get_path() {
						if msg.is_media_downloaded() {
							let _ = std::fs::copy(&msg.media_path, &path);
							live_region::announce(
								parts.conversations_parts.messages_announcer.clone(),
								"File saved successfully.",
							);
						} else {
							let payload = serde_json::json!({
								"action": "save_media_as",
								"data": { "jid": jid, "message_id": msg.id, "dest_path": path }
							});
							let _ = state.borrow().ws_tx.send(payload.to_string());
						}
					}
				}
			}
		}
		UiCommand::MuteCall => {
			let s = state.borrow();
			if let Some(audio) = &s.active_call_audio {
				let current = audio.is_muted.load(std::sync::atomic::Ordering::Relaxed);
				audio.is_muted.store(!current, std::sync::atomic::Ordering::Relaxed);
				let status = if !current { "Microphone muted." } else { "Microphone unmuted." };
				parts.call_parts.mute_btn.set_label(if !current { "Unmute microphone" } else { "Mute microphone" });
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), status);
			}
		}
		UiCommand::PrepareContactPicker(mode) => {
			if mode == ContactPickerMode::AddCallParticipant && state.borrow().active_call_id.is_none() {
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), "No active call.");
				return;
			}
			state.borrow_mut().contact_picker_mode = mode;
			parts.new_conv_dialog.search_field.set_value("");
			let heading = match mode {
				ContactPickerMode::OpenChat => "Search name or number",
				ContactPickerMode::AddCallParticipant => "Search a contact to add to the call, then press Enter",
			};
			parts.new_conv_dialog.search_label.set_label(heading);
			// "New Group" and "New Contact" only make sense when this dialog
			// is opened to start a chat - shown regardless of mode, they
			// caused exactly the confusion reported when adding a call
			// participant: press Enter on a search match to add someone
			// instead, which is the only action that applies here.
			let is_open_chat = mode == ContactPickerMode::OpenChat;
			parts.new_conv_dialog.new_group_btn.show(is_open_chat);
			parts.new_conv_dialog.new_contact_btn.show(is_open_chat);
			parts.new_conv_dialog.dialog.layout();
			apply_contact_picker_filter("", state, parts);
			let payload = serde_json::json!({ "action": "get_contacts" });
			let _ = state.borrow().ws_tx.send(payload.to_string());
			parts.new_conv_dialog.dialog.show_modal();
		}
		UiCommand::FilterContactPicker(query) => {
			apply_contact_picker_filter(&query, state, parts);
		}
		UiCommand::SelectContactPickerResult(idx) => {
			let (jid_opt, mode) = {
				let s = state.borrow();
				(s.contact_picker_current_jids.get(idx).cloned(), s.contact_picker_mode)
			};
			let Some(jid) = jid_opt else {
				return;
			};
			parts.new_conv_dialog.dialog.end_modal(wxdragon::id::ID_OK);
			match mode {
				ContactPickerMode::OpenChat => {
					open_chat_for_jid(&jid, state, parts);
				}
				ContactPickerMode::AddCallParticipant => {
					let s = state.borrow();
					if let Some(call_id) = &s.active_call_id {
						let msg = serde_json::json!({
							"action": "add_participant",
							"data": { "call_id": call_id, "jid": jid }
						});
						let _ = s.ws_tx.send(msg.to_string());
						drop(s);
						// The outcome arrives as call_participant_added /
						// call_participant_add_failed once WhatsApp has answered.
						live_region::announce(
							parts.conversations_parts.messages_announcer.clone(),
							"Adding to call...",
						);
					}
				}
			}
		}
		UiCommand::SubscribePresence(jid) => {
			let payload = serde_json::json!({
				"action": "subscribe_presence",
				"data": {
					"jid": jid
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::ActivateWindow => {
			// A newly launched second instance is asking us to come to the
			// foreground instead of starting its own copy. We have to be the
			// one to call raise()/SetForegroundWindow on our own window —
			// the launcher process doing it directly would just flash the
			// taskbar icon, since Windows only lets a process steal focus
			// for itself (see single_instance::request_activate).
			parts.frame.show(true);
			parts.frame.iconize(false);
			parts.frame.raise();
			handle_ui_command(UiCommand::SetPresence(true), state, parts);
		}
		UiCommand::SetPresence(available) => {
			let mut s = state.borrow_mut();
			let regained_focus = available && !s.window_focused;
			s.window_focused = available;

			// The "N unread messages" divider is already built and kept
			// up to date live as messages arrive in the background (see the
			// "message" event handler); on refocus there's nothing left to
			// do but send the batched read receipt for what queued up.
			if regained_focus && !s.background_unread_message_ids.is_empty() {
				if let Some(jid) = s.selected_chat.clone() {
					let ids = std::mem::take(&mut s.background_unread_message_ids);
					let payload = serde_json::json!({
						"action": "mark_read",
						"data": { "jid": jid, "message_ids": ids }
					});
					let _ = s.ws_tx.send(payload.to_string());
					// handle_mark_read on the backend only broadcasts a
					// "chat_updated" event, which nothing here listens for -
					// left alone, unread_counts (and thus the chat list's
					// "N unread messages" label, and the divider recreated
					// from it on the next OpenConversation) would stay stale
					// until some unrelated event happened to trigger a full
					// chats resync. Clear it locally now and ask for a fresh
					// snapshot to repaint the list right away.
					s.unread_counts.insert(jid, 0);
					refresh_tray_tooltip(&s, parts);
					let refresh_payload = serde_json::json!({ "action": "get_chats" });
					let _ = s.ws_tx.send(refresh_payload.to_string());
				} else {
					s.background_unread_message_ids.clear();
				}
			}

			// Going unavailable (blur, minimize, hide-to-tray) must pause any
			// in-flight composing state too, for the same reason a chat switch
			// does - otherwise backgrounding the app mid-compose leaves the
			// other side seeing "typing…" indefinitely.
			if !available {
				if let Some(jid) = s.selected_chat.clone() {
					let typing_payload = serde_json::json!({
						"action": "send_typing",
						"data": { "jid": jid, "state": "paused", "media": "" }
					});
					let _ = s.ws_tx.send(typing_payload.to_string());
				}
			}

			let payload = serde_json::json!({
				"action": "set_presence",
				"data": {
					"available": available
				}
			});
			let _ = s.ws_tx.send(payload.to_string());
		}
		UiCommand::MarkRead { jid, message_ids } => {
			let payload = serde_json::json!({
				"action": "mark_read",
				"data": {
					"jid": jid,
					"message_ids": message_ids
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::PlayVoiceMessage(msg_id) => {
			let payload = serde_json::json!({
				"action": "play_voice_message",
				"data": { "id": msg_id }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::CloseConversation => {
			let mut s = state.borrow_mut();
			save_composer_draft(&mut s, parts);
			s.selected_chat = None;
			clear_pending_reply(&mut s, parts);
			parts.conversations_parts.conversation_panel.show(false);
			parts.conversations_parts.panel.layout();
			parts.conversations_parts.conversations_list.set_focus();
		}
		UiCommand::EditMessage { message_id, new_text } => {
			let jid = state.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let payload = serde_json::json!({
					"action": "edit_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"new_text": new_text
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::DeleteMessage { message_id, sender_jid, for_everyone } => {
			let jid = state.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let payload = serde_json::json!({
					"action": "delete_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"sender_jid": sender_jid,
						"for_everyone": for_everyone
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::StarMessage { message_id, sender_jid, from_me, starred } => {
			let jid = state.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let payload = serde_json::json!({
					"action": "star_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"sender_jid": sender_jid,
						"from_me": from_me,
						"starred": starred
					}
				});
				let _ = state.borrow().ws_tx.send(payload.to_string());
			}
		}
		UiCommand::ExportChat { jid, dest_path } => {
			let payload = serde_json::json!({
				"action": "export_chat",
				"data": { "jid": jid, "dest_path": dest_path }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				"Exporting chat, this may take a while for chats with a lot of media...",
			);
		}
		UiCommand::ExportChatText { jid, dest_path } => {
			let payload = serde_json::json!({
				"action": "export_chat_text",
				"data": { "jid": jid, "dest_path": dest_path }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Exporting chat text...");
		}
		UiCommand::PostStatus { text } => {
			let payload = serde_json::json!({
				"action": "post_status",
				"data": { "text": text }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Posting status update...");
		}
		UiCommand::PostMediaStatus => {
			let file_dialog = FileDialog::builder(&parts.frame)
				.with_message("Choose a photo or video to post as a status")
				.with_wildcard(
					"Photos and videos|*.jpg;*.jpeg;*.png;*.gif;*.webp;*.bmp;*.mp4;*.avi;*.mov;*.mkv;*.webm;*.3gp",
				)
				.with_style(FileDialogStyle::Open | FileDialogStyle::FileMustExist)
				.build();
			if file_dialog.show_modal() == wxdragon::id::ID_OK {
				if let Some(path) = file_dialog.get_path() {
					let ext =
						std::path::Path::new(&path).extension().unwrap_or_default().to_string_lossy().to_lowercase();
					let media_type = match ext.as_str() {
						"mp4" | "avi" | "mov" | "mkv" | "webm" | "3gp" => "video",
						_ => "image",
					};
					let filename =
						std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();

					let dlg = wxdragon::TextEntryDialog::builder(
						&parts.frame,
						"Add a caption (optional):",
						"Post Photo/Video Status",
					)
					.build();
					let caption = if dlg.show_modal() == wxdragon::id::ID_OK {
						dlg.get_value().unwrap_or_default()
					} else {
						String::new()
					};

					let payload = serde_json::json!({
						"action": "post_media_status",
						"data": { "type": media_type, "path": path, "filename": filename, "caption": caption }
					});
					let _ = state.borrow().ws_tx.send(payload.to_string());
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Posting status update...",
					);
				}
			}
		}
		UiCommand::GetMessageInfo { message_id } => {
			let payload = serde_json::json!({
				"action": "get_message_info",
				"data": { "id": message_id }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RetryMessage { message_id } => {
			let payload = serde_json::json!({
				"action": "retry_message",
				"data": { "message_id": message_id }
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::RetrySelectedMessage => {
			// The toolbar's Retry send button: unlike the context menu's single
			// RetryMessage, this retries every failed item of an album.
			let s = state.borrow();
			let failed: Vec<String> = parts
				.conversations_parts
				.messages_list
				.get_selection()
				.and_then(|idx| s.display_items.get(idx as usize))
				.map(|item| match item {
					whantv::DisplayItem::Single(m) => std::slice::from_ref(m),
					whantv::DisplayItem::Album { messages, .. } => messages.as_slice(),
				})
				.unwrap_or_default()
				.iter()
				.filter(|m| m.from_me && !m.is_date_divider && m.status == "failed")
				.map(|m| m.id.clone())
				.collect();
			for message_id in &failed {
				let payload = serde_json::json!({
					"action": "retry_message",
					"data": { "message_id": message_id }
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
			if !failed.is_empty() {
				live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Retrying send...");
			}
		}
		UiCommand::RequestMessageResend { message_id } => {
			let s = state.borrow();
			if let Some(jid) = s.selected_chat.clone() {
				let payload = serde_json::json!({
					"action": "request_message_resend",
					"data": { "jid": jid, "message_id": message_id }
				});
				let _ = s.ws_tx.send(payload.to_string());
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Requested this message from your phone",
				);
			}
		}
		UiCommand::CopyAlbumToClipboard { message_ids } => {
			let (paths, is_single_voice): (Vec<String>, bool) = {
				let s = state.borrow();
				let mut p = Vec::new();
				let mut voice_count = 0;
				let temp_dir = std::env::temp_dir().join("whantv_clipboard");
				let _ = std::fs::create_dir_all(&temp_dir);
				let mut used_names: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

				for id in &message_ids {
					if let Some(msg) = s.messages.iter().find(|m| m.id == *id) {
						if msg.is_media_downloaded()
							&& !msg.media_path.is_empty()
							&& std::path::Path::new(&msg.media_path).exists()
						{
							let display_name = unique_clipboard_filename(&msg.display_filename(), &mut used_names);
							let temp_path = temp_dir.join(&display_name);
							let copy_path = if std::fs::copy(&msg.media_path, &temp_path).is_ok() {
								temp_path.to_string_lossy().to_string()
							} else {
								msg.media_path.clone()
							};
							p.push(copy_path);
							if msg.is_audio
								|| msg.media_type == "voice"
								|| msg.media_type == "audio"
								|| msg.text.starts_with("[Voice Message]")
								|| msg.text.starts_with("[Audio]")
							{
								voice_count += 1;
							}
						}
					}
				}
				(p, message_ids.len() == 1 && voice_count > 0)
			};

			if !paths.is_empty() {
				if copy_files_to_clipboard(&paths) {
					let ann_text = if is_single_voice {
						"Copied voice message to clipboard.".to_string()
					} else if paths.len() == 1 {
						"Copied file to clipboard.".to_string()
					} else {
						format!("Copied {} files to clipboard.", paths.len())
					};
					live_region::announce(parts.conversations_parts.messages_announcer.clone(), &ann_text);
				} else {
					live_region::announce(
						parts.conversations_parts.messages_announcer.clone(),
						"Failed to copy file to clipboard.",
					);
				}
			} else {
				let payload = serde_json::json!({
					"action": "copy_album_to_clipboard",
					"data": { "message_ids": message_ids }
				});
				{
					let mut s = state.borrow_mut();
					for id in &message_ids {
						s.pending_copy_announcements.insert(id.clone());
					}
					let _ = s.ws_tx.send(payload.to_string());
				}
				live_region::announce(
					parts.conversations_parts.messages_announcer.clone(),
					"Downloading file before copying to clipboard...",
				);
			}
		}
		UiCommand::SeekAudio(delta) => {
			let mut s = state.borrow_mut();
			if s.playing_msg_id.is_some() {
				let elapsed = if s.is_audio_paused {
					s.audio_elapsed_secs
				} else {
					s.audio_elapsed_secs + s.audio_start_instant.map(|i| i.elapsed().as_secs_f32()).unwrap_or(0.0)
				};
				let total = s.audio_total_secs;
				let target =
					if total > 0.0 { (elapsed + delta).max(0.0).min(total) } else { (elapsed + delta).max(0.0) };
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				let path = s.audio_path.clone().unwrap_or_default();
				if let Some(ref stream_handle) = s.audio_stream {
					let new_sink = do_seek(stream_handle, &path, target, s.audio_speed);
					s.audio_sink = new_sink;
				}
				if let Some(ref mut stdin) = s.audio_stdin {
					use std::io::Write;
					let _ = writeln!(stdin, "SEEK {}", target);
					let _ = stdin.flush();
				}
				s.audio_elapsed_secs = target;
				s.audio_start_instant = Some(std::time::Instant::now());
				s.is_audio_paused = false;
				parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
				let max_sec = (total as i32).max(1);
				let pos = (target as i32).max(0).min(max_sec);
				parts.conversations_parts.audio_slider.set_range(0, max_sec);
				parts.conversations_parts.audio_slider.set_value(pos);
			}
		}
		UiCommand::RefreshMessages => {}
		UiCommand::DownloadAttachment(_) => {}
		UiCommand::ChangeAudioSpeed => {}
		UiCommand::UpdateGroupSettings { name, topic, announce, locked } => {
			let jid = state.borrow().selected_chat.clone();
			if let Some(jid) = jid {
				let s = state.borrow();

				let msg_name = serde_json::json!({
					"action": "set_group_name",
					"data": { "jid": jid, "name": name }
				});
				let _ = s.ws_tx.send(msg_name.to_string());

				let msg_topic = serde_json::json!({
					"action": "set_group_topic",
					"data": { "jid": jid, "topic": topic }
				});
				let _ = s.ws_tx.send(msg_topic.to_string());

				let msg_ann = serde_json::json!({
					"action": "set_group_announce",
					"data": { "jid": jid, "value": announce }
				});
				let _ = s.ws_tx.send(msg_ann.to_string());

				let msg_lock = serde_json::json!({
					"action": "set_group_locked",
					"data": { "jid": jid, "value": locked }
				});
				let _ = s.ws_tx.send(msg_lock.to_string());

				live_region::announce(parts.conversations_parts.messages_announcer.clone(), "Group settings updated.");
			}
		}
		UiCommand::ToggleGroupMemberAdmin(idx) => {
			let s = state.borrow();
			if let Some(jid) = s.selected_chat.clone() {
				if let Some(member_jid) = s.group_data_visible_member_jids.get(idx as usize).cloned() {
					let is_admin = s
						.group_data_member_jids
						.iter()
						.position(|j| j == &member_jid)
						.and_then(|i| s.group_data_member_admin.get(i).copied())
						.unwrap_or(false);
					let action = if is_admin { "demote" } else { "promote" };
					let msg = serde_json::json!({
						"action": "update_group_participants",
						"data": { "jid": jid, "participants": [member_jid], "action": action }
					});
					let _ = s.ws_tx.send(msg.to_string());
				}
			}
		}
		UiCommand::RemoveGroupMember(idx) => {
			let s = state.borrow();
			if let Some(jid) = s.selected_chat.clone() {
				if let Some(member_jid) = s.group_data_visible_member_jids.get(idx as usize).cloned() {
					let label = s.group_data_members.get(idx as usize).cloned().unwrap_or_default();
					drop(s);
					let confirm = MessageDialog::builder(
						&parts.frame,
						&format!("Remove {} from the group?", label),
						"Remove Member",
					)
					.with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
					.build();
					if confirm.show_modal() == wxdragon::id::ID_YES {
						let msg = serde_json::json!({
							"action": "update_group_participants",
							"data": { "jid": jid, "participants": [member_jid], "action": "remove" }
						});
						let _ = state.borrow().ws_tx.send(msg.to_string());
					}
				}
			}
		}
		UiCommand::JoinCommunitySubgroup(idx) => {
			let s = state.borrow();
			if let Some(subgroup_jid) = s.group_data_subgroup_jids.get(idx as usize).cloned() {
				let msg = serde_json::json!({
					"action": "join_community_subgroup",
					"data": { "jid": subgroup_jid }
				});
				let _ = s.ws_tx.send(msg.to_string());
			}
		}
		UiCommand::OpenGroupInvite(message_id) => {
			let msg = serde_json::json!({
				"action": "get_group_invite_info",
				"data": { "message_id": message_id }
			});
			let _ = state.borrow().ws_tx.send(msg.to_string());
		}
		UiCommand::JoinGroupInvite(message_id) => {
			let msg = serde_json::json!({
				"action": "join_group_invite",
				"data": { "message_id": message_id }
			});
			let _ = state.borrow().ws_tx.send(msg.to_string());
		}
		UiCommand::SeekAudioAbsolute(slider_val) => {
			let mut s = state.borrow_mut();
			if s.playing_msg_id.is_some() {
				let total = s.audio_total_secs;
				let target = (slider_val as f32).max(0.0).min(total);
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				let path = s.audio_path.clone().unwrap_or_default();
				if let Some(ref stream_handle) = s.audio_stream {
					let new_sink = do_seek(stream_handle, &path, target, s.audio_speed);
					s.audio_sink = new_sink;
				}
				if let Some(ref mut stdin) = s.audio_stdin {
					use std::io::Write;
					let _ = writeln!(stdin, "SEEK {}", target);
					let _ = stdin.flush();
				}
				s.audio_elapsed_secs = target;
				s.audio_start_instant = Some(std::time::Instant::now());
				s.is_audio_paused = false;
				parts.conversations_parts.play_voice_btn.set_label("Pause voice message");
				let max_sec = (total as i32).max(1);
				let pos = (target as i32).max(0).min(max_sec);
				parts.conversations_parts.audio_slider.set_range(0, max_sec);
				parts.conversations_parts.audio_slider.set_value(pos);
			}
		}
		UiCommand::ReactToMessage { message_id, sender_jid, emoji } => {
			let s = state.borrow();
			if let Some(ref jid) = s.selected_chat {
				let payload = serde_json::json!({
					"action": "react_message",
					"data": {
						"jid": jid,
						"message_id": message_id,
						"sender_jid": sender_jid,
						"emoji": emoji
					}
				});
				let _ = s.ws_tx.send(payload.to_string());
			}
		}
		UiCommand::ToggleAudioSpeed => {
			let mut s = state.borrow_mut();
			let old_speed = s.audio_speed;
			let new_speed = if s.audio_speed >= 2.0 {
				1.0
			} else if s.audio_speed >= 1.5 {
				2.0
			} else {
				1.5
			};
			s.audio_speed = new_speed;
			parts.conversations_parts.audio_speed_btn.set_label(&format!("{:.1}x", new_speed));

			// Trigger soft-seek to apply new wsola speed
			if s.playing_msg_id.is_some() && s.audio_sink.is_some() {
				let was_paused = s.is_audio_paused;
				// Use the *old* speed: the wall-clock time elapsed since the
				// last seek/start was played back at the previous speed, not
				// the one we're switching to. Using the new speed here made
				// every speed change jump the position, i.e. "plays from a
				// bit before" where it actually was.
				let elapsed = s.audio_elapsed_secs
					+ s.audio_start_instant.map(|i| i.elapsed().as_secs_f32() * old_speed).unwrap_or(0.0);
				let target = elapsed.max(0.0).min(s.audio_total_secs);
				if let Some(ref sink) = s.audio_sink {
					sink.stop();
				}
				s.audio_sink = None;
				let path = s.audio_path.clone().unwrap_or_default();
				if let Some(ref stream_handle) = s.audio_stream {
					let new_sink = do_seek(stream_handle, &path, target, new_speed);
					if was_paused {
						if let Some(ref sink) = new_sink {
							sink.pause();
						}
					}
					s.audio_sink = new_sink;
				}
				s.audio_elapsed_secs = target;
				s.audio_start_instant = if was_paused { None } else { Some(std::time::Instant::now()) };
				s.is_audio_paused = was_paused;
				parts.conversations_parts.play_voice_btn.set_label(if was_paused {
					"Play voice message"
				} else {
					"Pause voice message"
				});
			}
		}
		UiCommand::ToggleLockChat { jid, lock } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if lock {
					s.locked_jids.insert(jid.clone());
				} else {
					s.locked_jids.remove(&jid);
				}
				apply_chat_filter(&mut s, parts);
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_locked",
				"data": {
					"jid": jid,
					"value": lock
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			let action_str = if lock { "Locked" } else { "Unlocked" };
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("{} chat {}.", action_str, chat_name),
			);
		}
		UiCommand::TogglePinChat { jid, pin } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if pin {
					s.pinned_jids.insert(jid.clone());
				} else {
					s.pinned_jids.remove(&jid);
				}
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_pinned",
				"data": {
					"jid": jid,
					"value": pin
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} {}.", chat_name, if pin { "pinned" } else { "unpinned" }),
			);
		}
		UiCommand::ToggleArchiveChat { jid, archive } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if archive {
					s.archived_jids.insert(jid.clone());
				} else {
					s.archived_jids.remove(&jid);
				}
				let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				apply_chat_filter(&mut s, parts);
				name
			};
			let payload = serde_json::json!({
				"action": "set_chat_archived",
				"data": {
					"jid": jid,
					"value": archive
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} {}.", chat_name, if archive { "archived" } else { "unarchived" }),
			);
		}
		UiCommand::SetArchivedAlert { jid, level } => {
			let (chat_name, spoken) = {
				let mut s = state.borrow_mut();
				s.settings.set_archived_alert(&jid, level);
				s.settings.save();
				// The tooltip total only counts chats that would actually alert,
				// so changing a level changes it.
				refresh_tray_tooltip(&s, parts);
				let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				let spoken = match level {
					whantv::ArchivedAlert::Silent => "silent",
					whantv::ArchivedAlert::SoundOnly => "sound only",
					whantv::ArchivedAlert::Full => "notify normally",
				};
				(name, spoken)
			};
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Archived alerts for {} set to {}.", chat_name, spoken),
			);
		}
		UiCommand::ToggleMuteChat { jid, mute } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if mute {
					s.muted_jids.insert(jid.clone());
				} else {
					s.muted_jids.remove(&jid);
				}
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_muted",
				"data": {
					"jid": jid,
					"muted": mute,
					"duration_seconds": 0
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Notifications {} for {}.", if mute { "muted" } else { "unmuted" }, chat_name),
			);
		}
		UiCommand::ToggleReadChat { jid, read } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				if read {
					s.unread_counts.insert(jid.clone(), 0);
					clear_unread_divider_for_open_chat(&mut s, parts, &jid);
				} else {
					s.unread_counts.insert(jid.clone(), 1);
				}
				refresh_tray_tooltip(&s, parts);
				s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone())
			};
			let payload = serde_json::json!({
				"action": "set_chat_read",
				"data": {
					"jid": jid,
					"value": read
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} marked as {}.", chat_name, if read { "read" } else { "unread" }),
			);
		}
		UiCommand::DeleteChat { jid } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				s.chats.retain(|j| j != &jid);
				s.filtered_chats.retain(|j| j != &jid);
				s.archived_jids.remove(&jid);
				s.pinned_jids.remove(&jid);
				s.muted_jids.remove(&jid);
				s.unread_counts.remove(&jid);
				s.draft_messages.remove(&jid);
				let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				if s.selected_chat.as_ref() == Some(&jid) {
					s.selected_chat = None;
					clear_pending_reply(&mut s, parts);
					parts.conversations_parts.conversation_panel.show(false);
				}
				apply_chat_filter(&mut s, parts);
				refresh_tray_tooltip(&s, parts);
				name
			};
			let payload = serde_json::json!({
				"action": "delete_chat",
				"data": {
					"jid": jid
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Chat {} deleted.", chat_name),
			);
		}
		UiCommand::LeaveGroup { jid } => {
			let chat_name = {
				let mut s = state.borrow_mut();
				s.chats.retain(|j| j != &jid);
				s.filtered_chats.retain(|j| j != &jid);
				s.archived_jids.remove(&jid);
				s.pinned_jids.remove(&jid);
				s.muted_jids.remove(&jid);
				s.unread_counts.remove(&jid);
				s.draft_messages.remove(&jid);
				let name = s.contacts.get(&jid).cloned().unwrap_or_else(|| jid.clone());
				if s.selected_chat.as_ref() == Some(&jid) {
					s.selected_chat = None;
					clear_pending_reply(&mut s, parts);
					parts.conversations_parts.conversation_panel.show(false);
				}
				apply_chat_filter(&mut s, parts);
				refresh_tray_tooltip(&s, parts);
				name
			};
			let payload = serde_json::json!({
				"action": "leave_group",
				"data": {
					"jid": jid
				}
			});
			let _ = state.borrow().ws_tx.send(payload.to_string());
			live_region::announce(
				parts.conversations_parts.messages_announcer.clone(),
				&format!("Left group {}.", chat_name),
			);
		}
	}
}

/// Decodes an Ogg Opus file into interleaved f32 PCM at 48kHz.
///
/// This walks the raw Ogg page structure by hand instead of using the `ogg`
/// crate's `PacketReader`: that reader enforces strict page-continuity
/// bookkeeping and hard-errors (aborting the rest of the file) on real-world
/// voice notes - including ones recorded with pause/resume by WhatsApp's
/// own official app - that don't perfectly match its expectations, even
/// though the same files play back with the exact intended duration in VLC,
/// ffmpeg, and the official WhatsApp client. `PacketReader`'s only lenient
/// recovery path is gated behind an internal flag that's not reachable
/// through its public API without a real prior seek, and its packet data is
/// otherwise private, so there's no way to opt into leniency from outside
/// the crate. Since the Ogg page format itself is simple, this reimplements
/// just enough of it to reconstruct packets tolerantly: on any structural
/// oddity (a page continuation that doesn't line up, a lost sync pattern)
/// it drops only the one ambiguous packet at that seam and keeps going,
/// instead of losing everything after it.
fn decode_opus_file(path: &str) -> Option<rodio::buffer::SamplesBuffer> {
	use opus::{Channels, Decoder};

	fn feed_packet(
		packet: &[u8],
		decoder: &mut Option<Decoder>,
		channels: &mut usize,
		pre_skip: &mut usize,
		skipped: &mut usize,
		samples: &mut Vec<f32>,
	) {
		if packet.len() >= 8 && &packet[0..8] == b"OpusHead" {
			// Header packet, not audio - (re)establish the decoder from it.
			// Real chaining (a new pause/resume segment spliced in as its
			// own logical stream) shows up as more of these later in the
			// file; picking up its channel count/pre-skip here handles that
			// case too, not just the file's very first stream.
			*channels = if packet.len() >= 10 { (packet[9] as usize).max(1) } else { 1 };
			*pre_skip = if packet.len() >= 12 { u16::from_le_bytes([packet[10], packet[11]]) as usize } else { 0 };
			let opus_channels = if *channels >= 2 { Channels::Stereo } else { Channels::Mono };
			*decoder = Decoder::new(48000, opus_channels).ok();
			*skipped = 0;
			return;
		}
		if packet.len() >= 8 && &packet[0..8] == b"OpusTags" {
			return;
		}
		let Some(dec) = decoder.as_mut() else { return };
		let mut pcm = vec![0i16; 5760 * (*channels).max(1)]; // Max frame size for opus is 120ms at 48kHz
		if let Ok(n) = dec.decode(packet, &mut pcm, false) {
			pcm.truncate(n * (*channels).max(1));
			let mut frame: Vec<f32> = pcm.into_iter().map(|s| (s as f32) / 32768.0).collect();
			if *skipped < *pre_skip {
				let frames_here = frame.len() / (*channels).max(1);
				let to_skip = (*pre_skip - *skipped).min(frames_here);
				frame.drain(0..to_skip * (*channels).max(1));
				*skipped += to_skip;
			}
			samples.extend(frame);
		}
	}

	let data = std::fs::read(path).ok()?;
	let len = data.len();
	let mut pos: usize = 0;

	let mut decoder: Option<Decoder> = None;
	let mut channels: usize = 1;
	let mut pre_skip: usize = 0;
	let mut skipped: usize = 0;
	let mut samples: Vec<f32> = Vec::new();
	let mut pending: Vec<u8> = Vec::new(); // partial packet carried across a page boundary

	while pos + 27 <= len {
		if &data[pos..pos + 4] != b"OggS" {
			// Lost sync (or not actually at a page boundary yet) - scan
			// forward for the next capture pattern rather than giving up.
			match data[pos + 1..].windows(4).position(|w| w == b"OggS") {
				Some(off) => {
					pos += 1 + off;
					continue;
				}
				None => break,
			}
		}
		let header = &data[pos..pos + 27];
		let flags = header[5];
		let segs = header[26] as usize;
		if pos + 27 + segs > len {
			break;
		}
		let lacing = &data[pos + 27..pos + 27 + segs];
		let body_len: usize = lacing.iter().map(|&b| b as usize).sum();
		let body_start = pos + 27 + segs;
		if body_start + body_len > len {
			break;
		}
		let body = &data[body_start..body_start + body_len];

		let starts_with_continued = flags & 0x01 != 0;
		if !starts_with_continued {
			pending.clear();
		}

		let mut offset = 0usize;
		let mut seg_run_start = 0usize;
		let mut first_packet_of_page = true;
		for (i, &seg_len) in lacing.iter().enumerate() {
			offset += seg_len as usize;
			let is_packet_end = seg_len < 255;
			let is_last_segment = i + 1 == lacing.len();
			if is_packet_end {
				let packet_bytes = &body[seg_run_start..offset];
				seg_run_start = offset;
				let is_continuation = first_packet_of_page && starts_with_continued;
				first_packet_of_page = false;
				if is_continuation {
					if pending.is_empty() {
						// No matching partial packet to complete this with
						// (e.g. we just resynced) - can't recover it, drop
						// it and move on to the next one.
						continue;
					}
					let mut full = std::mem::take(&mut pending);
					full.extend_from_slice(packet_bytes);
					feed_packet(&full, &mut decoder, &mut channels, &mut pre_skip, &mut skipped, &mut samples);
				} else {
					feed_packet(packet_bytes, &mut decoder, &mut channels, &mut pre_skip, &mut skipped, &mut samples);
				}
			} else if is_last_segment {
				// Page ends mid-packet - stash it for the next page to
				// (hopefully) complete.
				pending.extend_from_slice(&body[seg_run_start..offset]);
			}
		}

		pos = body_start + body_len;
	}

	if samples.is_empty() {
		return None;
	}

	Some(rodio::buffer::SamplesBuffer::new(
		std::num::NonZeroU16::new(channels as u16).unwrap_or(std::num::NonZeroU16::new(1).unwrap()),
		std::num::NonZeroU32::new(48000).unwrap(),
		samples,
	))
}

fn convert_wav_to_opus_ogg(wav_path: &str, ogg_path: &str) -> anyhow::Result<()> {
	use std::{fs::File, io::BufWriter};

	use hound::WavReader;
	use ogg::PacketWriter;
	use opus::{Application, Channels, Encoder};

	let mut reader = WavReader::open(wav_path)?;
	let spec = reader.spec();

	if spec.sample_rate != 48000 {
		return Err(anyhow::anyhow!("Sample rate must be 48000"));
	}
	let is_stereo = spec.channels == 2;
	let opus_channels = if is_stereo { Channels::Stereo } else { Channels::Mono };

	let mut encoder = Encoder::new(48000, opus_channels, Application::Voip)?;
	let out_file = File::create(ogg_path)?;
	let mut writer = PacketWriter::new(BufWriter::new(out_file));
	let serial = 12345;

	// OpusHead
	let mut head = Vec::new();
	head.extend_from_slice(b"OpusHead");
	head.push(1); // Version
	head.push(spec.channels as u8); // Channels
	head.extend_from_slice(&384_u16.to_le_bytes()); // Preskip
	head.extend_from_slice(&48000_u32.to_le_bytes()); // Input sample rate
	head.extend_from_slice(&0_u16.to_le_bytes()); // Gain
	head.push(0); // Channel mapping family
	writer.write_packet(head, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

	// OpusTags
	let mut tags = Vec::new();
	tags.extend_from_slice(b"OpusTags");
	tags.extend_from_slice(&8_u32.to_le_bytes()); // Vendor length
	tags.extend_from_slice(b"whaNTV  ");
	tags.extend_from_slice(&0_u32.to_le_bytes()); // User comments list length
	writer.write_packet(tags, serial, ogg::PacketWriteEndInfo::EndPage, 0)?;

	let mut granule = 0;
	let mut samples = Vec::new();
	let samples_per_frame = if is_stereo { 960 * 2 } else { 960 };

	for sample in reader.samples::<i16>() {
		samples.push(sample?);
		if samples.len() == samples_per_frame {
			let mut out = vec![0u8; 1024];
			let len = encoder.encode(&samples, &mut out)?;
			out.truncate(len);
			granule += 960;
			writer.write_packet(out, serial, ogg::PacketWriteEndInfo::NormalPacket, granule)?;
			samples.clear();
		}
	}

	if !samples.is_empty() {
		let remaining_frames = samples.len() / (if is_stereo { 2 } else { 1 });
		samples.resize(samples_per_frame, 0);
		let mut out = vec![0u8; 1024];
		let len = encoder.encode(&samples, &mut out)?;
		out.truncate(len);
		granule += remaining_frames as u64;
		writer.write_packet(out, serial, ogg::PacketWriteEndInfo::EndStream, granule)?;
	}

	Ok(())
}

#[cfg(test)]
mod call_participant_tests {
	use super::*;

	fn p(jid: &str, name: &str, state: &str) -> CallParticipant {
		CallParticipant { jid: jid.into(), name: name.into(), state: state.into() }
	}

	#[test]
	fn status_text_lists_connected_and_ringing() {
		let roster = [p("1@lid", "Asha", "connected"), p("2@lid", "Ben", "connected"), p("3@lid", "Chris", "ringing")];
		assert_eq!(group_call_status_text(&roster), "In group call with Asha and Ben. Ringing Chris.");
		let roster = [p("1@lid", "Asha", "outgoing"), p("2@lid", "Ben", "outgoing"), p("3@lid", "Chris", "outgoing")];
		assert_eq!(group_call_status_text(&roster), "Calling Asha, Ben and Chris.");
	}

	#[test]
	fn changes_announce_join_leave_and_decline() {
		let old = [
			p("1@lid", "Asha", "outgoing"),
			p("2@lid", "Ben", "ringing"),
			p("3@lid", "Chris", "connected"),
			p("4@lid", "Dev", "connected"),
		];
		let new = [p("1@lid", "Asha", "connected"), p("2@lid", "Ben", "rejected"), p("4@lid", "Dev", "rejected")];
		assert_eq!(
			call_participant_changes(&old, &new),
			vec!["Asha joined the call.", "Ben declined the call.", "Dev left the call.", "Chris left the call."]
		);
		assert!(call_participant_changes(&new, &new).is_empty());
	}
}
