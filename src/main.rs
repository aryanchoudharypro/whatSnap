#![windows_subsystem = "windows"]
mod types;
pub use types::*;

mod announce;
mod audio;
use audio::*;
mod audio_device_watch;
mod audio_devices;
mod autostart;
mod backend_process;
mod call_audio;
mod call_helpers;
mod call_video_send;
mod chat_ui;
mod media_foundation;
mod video_devices;
use chat_ui::request_backend_shutdown;
mod clipboard;
pub use clipboard::copy_files_to_clipboard;
mod contact_picker;
mod countries;
mod do_not_disturb;
mod keymap;
mod message_display;
use message_display::update_action_buttons;
#[cfg(target_os = "windows")]
mod link_handler;
mod paths;
mod ringback;
mod settings;
mod single_instance;
mod tray;
mod ui;
mod ui_commands;
mod ui_wake;
mod voice_dsp;
mod voice_recorder;
mod websocket_events;
use websocket_events::process_websocket_message;
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
use rodio::{MixerDeviceSink, Player};
use wxdragon::prelude::*;

use crate::{
	ui::window::{WindowParts, bind_input_handlers, build_main_window},
	ui_wake::{ID_UI_WAKE, UiCommandSender, UiWaker},
};

/// How recently a message must have been sent for its "delivered" receipt to
/// ding. Also bounds `AppState::played_sent_sounds`: an ack older than this
/// can never ring, so it is never recorded and aged-out entries are dropped.
const SENT_SOUND_WINDOW_SECS: i64 = 20;

/// How many messages a `get_messages` request loads: -1 is every one. Opening
/// a chat loads all of its stored history at once, so the only thing left for
/// Home at the top to do is ask the phone (see `load_more_messages`). Paging
/// a chat in 1000-message pages made reaching its start a long series of
/// Home presses, each of which could strand the chat half-loaded.
pub const MESSAGE_PAGE_SIZE: i32 = -1;

/// How long `load_more_messages` waits for a `history_sync` broadcast
/// covering an on-demand phone history request before giving up on it (see
/// `AppState::phone_history_pending_jid`). WhatsApp simply stays quiet when
/// there's nothing older to send back - there's no explicit "no more
/// history" reply - so a timeout is the only way to notice that and stop
/// asking, which matters because repeatedly re-requesting the same
/// already-exhausted history is exactly the kind of on-demand traffic that
/// risks tripping WhatsApp's abuse detection.
const PHONE_HISTORY_TIMEOUT_SECS: u64 = 20;

// Presence and chatstate timing, taken verbatim from WhatsApp Web's own
// bundle (2.3000.1047483476) so this client's traffic looks like it:
// - WAWebConversation.react: window blur -> Stream.markUnavailable(15 * 1e3);
//   focus -> after delayMs(500), if still focused, markAvailable().
// - WAWebMain.react: an idle loop that waits 60s (w = 6e4), then arms
//   markUnavailable(3e4) until mousemove/scroll/keydown/focus/resize.
// - WAWebPresenceChatAction: p = 2500 (paused after the last keystroke),
//   _ = 1e4 (composing/recording resent while still going, only while the
//   contact is online).
// - WAWebStreamModel: stays available through a connected call, and on
//   Windows goes unavailable when minimised to tray unless in a call.
const PRESENCE_BLUR_GRACE: Duration = Duration::from_millis(15_000);
const PRESENCE_FOCUS_SETTLE: Duration = Duration::from_millis(500);
const PRESENCE_IDLE_CHECK_EVERY: Duration = Duration::from_millis(60_000);
const PRESENCE_IDLE_GRACE: Duration = Duration::from_millis(30_000);
const CHATSTATE_PAUSE_AFTER: Duration = Duration::from_millis(2_500);
const CHATSTATE_RESEND_EVERY: Duration = Duration::from_millis(10_000);

/// An outgoing chatstate in progress: typing (`media` empty) or recording a
/// voice note (`media` "audio").
pub(crate) struct ComposingState {
	jid: String,
	media: String,
	last_sent: std::time::Instant,
	last_key: std::time::Instant,
}

/// WhatsApp Web's availability model (WAWebStreamModel), see the constants above.
pub(crate) struct PresenceModel {
	/// Stream.available: the user is considered present.
	available: bool,
	/// Pending markUnavailable() shift timer.
	unavailable_at: Option<std::time::Instant>,
	/// Focus regained, waiting out PRESENCE_FOCUS_SETTLE.
	focus_settle_since: Option<std::time::Instant>,
	/// Hidden to the tray (Windows' isMinimizedToTray).
	hidden: bool,
	/// Idle loop: when the next idle check arms.
	next_idle_check: std::time::Instant,
	/// Idle loop: armed, with the input tick it was armed at.
	idle_armed_at_input: Option<u32>,
}

impl Default for PresenceModel {
	fn default() -> Self {
		Self {
			available: true,
			unavailable_at: None,
			focus_settle_since: None,
			hidden: false,
			next_idle_check: std::time::Instant::now() + PRESENCE_IDLE_CHECK_EVERY,
			idle_armed_at_input: None,
		}
	}
}

struct AppState {
	settings: crate::settings::Settings,
	/// `settings.shortcuts` as a keymap, built when they load or change
	/// rather than on every key press.
	keymap: Rc<crate::keymap::Keymap>,
	ws_tx: mpsc::Sender<String>,
	ui_tx: UiCommandSender,
	audio_stream: Option<MixerDeviceSink>,
	/// Friendly name of the device `audio_stream` was actually opened
	/// against (as resolved at open time, even for "System Default") - lets
	/// `ensure_audio_stream` notice the resolved device has since changed
	/// (headphones unplugged/replugged) and reopen instead of silently
	/// writing to a now-stale stream until the app is restarted.
	audio_stream_device_name: Option<String>,
	/// When `audio_stream` was last (re)opened. A cached WASAPI stream can go
	/// silently dead across a sleep/hibernate cycle even when the resolved
	/// device name ends up unchanged (e.g. a transient enumeration race right
	/// at wake, or the OS invalidating the session across suspend regardless
	/// of name) - unlike notification sounds, which always open a stream
	/// fresh and keep working. `ensure_audio_stream` treats a stream older
	/// than `AUDIO_STREAM_MAX_AGE` as untrustworthy and reopens it rather
	/// than reusing it forever based on name-match alone.
	audio_stream_opened_at: Option<std::time::Instant>,
	/// Flipped by `audio_stream`'s cpal error callback when the underlying
	/// stream errors out (device removed/invalidated), so `ensure_audio_stream`
	/// can reopen on the very next play instead of waiting to notice via a
	/// name mismatch or `AUDIO_STREAM_MAX_AGE`, which don't catch every case
	/// (e.g. the same device name, session invalidated anyway).
	audio_stream_failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
	audio_sink: Option<Player>,
	playing_msg_id: Option<String>,
	/// The voice message autoplay last rolled on to, so the end-of-run tone
	/// only sounds after a real run (WhatsApp plays it for sequences only).
	voice_autoplay_id: Option<String>,
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
	/// Group -> the community it's in, for groups inside a community.
	chat_parents: std::collections::HashMap<String, String>,
	/// Communities' announcement groups.
	announcement_jids: std::collections::HashSet<String>,
	/// Channels we follow, and our role in each ("owner", "admin",
	/// "subscriber").
	channel_jids: std::collections::HashSet<String>,
	channel_roles: std::collections::HashMap<String, String>,
	/// Whether the server lets us pin posts in our channels (its
	/// channels_message_pin_admin_enabled flag, off unless it says so).
	channel_pinning: bool,
	/// The "Find channels" dialog while it's open, for results to land in.
	channel_directory: Option<Rc<crate::ui::channel_directory::ChannelDirectory>>,
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
	/// Availability, modelled on WhatsApp Web's (see `PresenceModel`).
	presence: PresenceModel,
	/// Last known online state of contacts, from "presence" events - a
	/// chatstate resend only goes out while the contact is online, as in WA Web.
	contact_online: std::collections::HashMap<String, bool>,
	/// The presence last sent to the backend, so repeats are never re-sent.
	presence_sent: Option<bool>,
	/// The chat we last told "composing" and when (see `send_typing`), so a
	/// held-down keyboard sends one chatstate every few seconds rather than one
	/// per keystroke, and "paused" follows once typing stops.
	composing: Option<ComposingState>,
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
	attach_contact_state: crate::ui::attach_contact::AttachContactState,
	selected_chat: Option<String>,
	admin_group_jids: std::collections::HashSet<String>,
	initial_presence_received: bool,
	messages: Vec<whantv::Msg>,
	/// Whether `messages` runs right up to the chat's newest message. False
	/// after jumping to an old search/starred hit (`get_messages_around`),
	/// where newer messages exist that aren't loaded - live messages for the
	/// chat then aren't appended beneath that window.
	messages_at_latest: bool,
	loading_messages: bool,
	/// True once a `get_messages` page for the open chat has come back
	/// shorter than requested - the local database has nothing older left
	/// for it, so `load_more_messages` should ask the phone instead of
	/// paging locally again. Reset per chat switch.
	no_more_messages: bool,
	history_loading: bool,
	/// A newer page (see `load_newer_messages`) is in flight.
	newer_loading: bool,
	/// Set while waiting on a `history_sync` broadcast for an on-demand
	/// phone history request `load_more_messages` sent after
	/// `no_more_messages` went true. Cleared once that jid's `history_sync`
	/// arrives (successful or not) or `phone_history_deadline` elapses.
	phone_history_pending_jid: Option<String>,
	/// Deadline for `phone_history_pending_jid` - see
	/// `PHONE_HISTORY_TIMEOUT_SECS`. Checked by the same periodic tick that
	/// already polls `history_sync_last_update`.
	phone_history_deadline: Option<std::time::Instant>,
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
	/// Set while the open chat is being reloaded because history sync just
	/// wrote messages for it (it was opened before they arrived), so the
	/// reload fills the list without moving focus off wherever the user is.
	quiet_reload_jid: Option<String>,
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
	/// A WhatsApp link that arrived before we were logged in with the chat
	/// list loaded; it runs once they are.
	pending_whatsapp_link: Option<String>,
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
	/// The attach-menu option `pending_attachments` came from.
	pending_attachment_kind: whantv::AttachKind,
	pending_reply_id: Option<String>,
	pending_reply_sender: Option<String>,
	pending_reply_preview: Option<String>,
	/// Set while the composer is editing an existing message in place
	/// (rather than composing a new one) - the id of the message being
	/// edited. `Some` swaps the composer's Enter/Escape/Tab handling and the
	/// "Send"/"Type a message" wording over to editing, instead of opening
	/// a separate dialog.
	editing_message_id: Option<String>,
	/// The message's text as it was when edit mode was entered, kept
	/// alongside `editing_message_id` so saving with no actual change can be
	/// treated like Cancel rather than sending a same-text edit.
	editing_message_original_text: Option<String>,
	/// Unsent composer text per chat, saved whenever the conversation is
	/// switched away from (closing it, or opening a different chat) and
	/// restored when that chat is opened again - so leaving mid-draft (e.g.
	/// Escape) never loses what was typed. Cleared once the draft is
	/// actually sent.
	draft_messages: std::collections::HashMap<String, String>,
	/// The @mentions picked in each saved draft, kept with `draft_messages`.
	draft_mentions: std::collections::HashMap<String, Vec<ui_commands::mentions::MentionEntry>>,
	/// @mentions picked in the composer's current text (see ui_commands::mentions).
	composer_mentions: Vec<ui_commands::mentions::MentionEntry>,
	/// Who can be mentioned in the open chat, once fetched.
	mention_candidates: Option<ui_commands::mentions::MentionCandidates>,
	/// Chat a mention_candidates request is out for.
	mention_request_pending: Option<String>,
	/// The open @mention suggestions.
	mention_popup: Option<ui_commands::mentions::MentionPopup>,
	/// The "@" whose suggestions were dismissed (Escape, or one was picked),
	/// so typing on after it doesn't reopen them.
	mention_dismissed_at: Option<usize>,
	pending_receipts: std::collections::HashMap<String, String>,
	downloads_in_progress: std::collections::HashMap<String, i64>,
	pending_copy_announcements: std::collections::HashSet<String>,
	search_target_id: Option<String>,
	search_target_text: Option<String>,
	/// Message id -> unix time of the ack that rang for it, kept only for
	/// `SENT_SOUND_WINDOW_SECS` so one message never dings twice while the
	/// map stays bounded by a single window's worth of acks.
	played_sent_sounds: std::collections::HashMap<String, i64>,
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
	/// Running only while the local camera is on - see `call_video_send.rs`.
	active_call_video_send: Option<call_video_send::CallVideoSendState>,
	/// TCP port `call_connected` handed us for the video bridge - what
	/// `active_call_video_send` connects to when the camera is turned on.
	/// `None` before `call_connected`, or if the backend couldn't open the
	/// bridge at all.
	call_video_port: Option<u16>,
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
	/// Video details of the active call, reset whenever the call is torn down.
	call_video: CallVideo,
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
	group_data_media: Vec<whantv::Msg>,
	group_data_documents: Vec<whantv::Msg>,
	group_data_links: Vec<whantv::Msg>,
	/// JIDs of the community subgroups shown in the Groups tab, aligned 1:1
	/// with `groups_list` rows, so a list selection can be joined by JID.
	group_data_subgroup_jids: Vec<String>,
	/// Whether we're in each of those groups, aligned the same way.
	group_data_subgroup_joined: Vec<bool>,
	/// The community the Groups tab is listing.
	group_data_community: Option<String>,
	/// Same as the group_data_* trio above, but for the 1-on-1 conversation
	/// data dialog.
	conversation_data_media: Vec<whantv::Msg>,
	conversation_data_documents: Vec<whantv::Msg>,
	conversation_data_links: Vec<whantv::Msg>,
	own_push_name: String,
	own_about: String,
	privacy_settings: std::collections::HashMap<String, String>,
	/// Whether `privacy_settings` reflects a real server response yet. Until
	/// this is true the settings dialog's Save button stays disabled, so a
	/// user who presses it before the fetch lands can't silently overwrite
	/// their real privacy settings with the dropdowns' placeholder defaults.
	privacy_settings_loaded: bool,
}

fn main() {
	std::panic::set_hook(Box::new(|info| {
		let _ = std::fs::write(paths::log_dir().join("panic.log"), format!("{}", info));
	}));

	// If whaNTV is already running, activate its window (restoring it from
	// the tray if needed) instead of launching a second instance.
	// whaNTV is registered as the whatsapp:// link handler, so it can be
	// started with a link to act on.
	let launch_link = std::env::args().skip(1).find(|arg| whantv::parse_whatsapp_link(arg).is_some());
	let _instance_lock = match single_instance::acquire_or_activate_existing(launch_link.as_deref()) {
		Some(lock) => lock,
		None => return,
	};
	// Keeps whaNTV registered for whatsapp:// links, at the path it now runs
	// from. Off the startup path; it's a few registry writes.
	#[cfg(target_os = "windows")]
	std::thread::spawn(link_handler::register);
	for name in ["backend_process.log", "call_audio.log"] {
		paths::rotate_log(name);
	}

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

	// Read directly rather than waiting for AppState's own Settings::load()
	// further down: the backend process is spawned here, before AppState (and
	// the window it lives in) exist, so this is the only place that can act
	// on it before that first launch.
	let verbose_backend_logging = crate::settings::Settings::load().verbose_backend_logging;
	let backend = backend_exe.map(|exe| {
		Arc::new(backend_process::BackendProcess::start(
			exe,
			backend_dir,
			db_path_str,
			go_pipe_name,
			verbose_backend_logging,
		))
	});
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

		audio_device_watch::start(ui_tx.clone());

		// Let future launches of the app (see `single_instance::acquire_or_activate_existing`
		// at the top of `main`) hand off to this window instead of starting a second instance.
		let ui_tx_activate = ui_tx.clone();
		single_instance::listen_for_activation(move |link| {
			let _ = ui_tx_activate.send(UiCommand::ActivateWindow);
			if let Some(link) = link {
				let _ = ui_tx_activate.send(UiCommand::OpenWhatsAppLink(link));
			}
		});
		if let Some(link) = launch_link.clone() {
			let _ = ui_tx.send(UiCommand::OpenWhatsAppLink(link));
		}

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
								// Hand the buffer itself over rather than allocating a
								// second full copy via `line.trim().to_string()`: one
								// chat's history arrives as a single line that can run
								// to many megabytes. Allocation count is unchanged (the
								// `to_string` was already one per line), but the peak is
								// halved.
								line.truncate(line.trim_end().len());
								let leading = line.len() - line.trim_start().len();
								if leading > 0 {
									line.drain(..leading);
								}
								// `mem::take` leaves a fresh empty String behind, so -
								// unlike `clear()` - the big line's capacity goes with
								// it instead of staying pinned at the session's
								// high-water mark for the life of the connection.
								let payload = std::mem::take(&mut line);
								let _ = ui_tx_clone.send(UiCommand::ProcessWebSocketMessage(payload));
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
			keymap: Rc::default(),
			ws_tx: ws_out_tx,
			ui_tx: ui_tx.clone(),
			audio_stream: None,
			audio_stream_device_name: None,
			audio_stream_opened_at: None,
			audio_stream_failed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
			audio_sink: None,
			playing_msg_id: None,
			voice_autoplay_id: None,
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
			chat_parents: std::collections::HashMap::new(),
			announcement_jids: std::collections::HashSet::new(),
			channel_jids: std::collections::HashSet::new(),
			channel_roles: std::collections::HashMap::new(),
			channel_pinning: false,
			channel_directory: None,
			pinned_jids: std::collections::HashSet::new(),
			muted_jids: std::collections::HashSet::new(),
			unread_counts: std::collections::HashMap::new(),
			unread_divider_jid: None,
			unread_divider_count: 0,
			window_focused: true,
			presence: PresenceModel::default(),
			contact_online: std::collections::HashMap::new(),
			presence_sent: None,
			composing: None,
			background_unread_message_ids: Vec::new(),
			display_names: std::collections::HashMap::new(),
			// Seeded so Meta AI's chat shows a proper name from the first
			// message instead of running its bot JID's numeric user part
			// through the phone-number formatter.
			contacts: std::collections::HashMap::from([(types::META_AI_JID.to_string(), "Meta AI".to_string())]),
			all_contacts: Vec::new(),
			contact_picker_mode: ContactPickerMode::OpenChat,
			contact_picker_current_jids: Vec::new(),
			presence_labels: std::collections::HashMap::new(),
			typing_timestamps: std::collections::HashMap::new(),
			typing_announce_state: std::collections::HashMap::new(),
			active_call_audio: None,
			active_call_video_send: None,
			call_video_port: None,
			active_call_id: None,
			call_placing: false,
			call_pending_cancel: false,
			call_answer_deadline: None,
			call_ringing_sink: None,
			incoming_call_caller: None,
			call_participants: Vec::new(),
			call_video: CallVideo::default(),
			group_data_members: Vec::new(),
			group_data_member_jids: Vec::new(),
			group_data_member_admin: Vec::new(),
			group_data_visible_member_jids: Vec::new(),
			group_data_media: Vec::new(),
			group_data_documents: Vec::new(),
			group_data_links: Vec::new(),
			group_data_subgroup_jids: Vec::new(),
			group_data_subgroup_joined: Vec::new(),
			group_data_community: None,
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
			attach_contact_state: crate::ui::attach_contact::AttachContactState {
				all_contact_labels: Vec::new(),
				all_contact_jids: Vec::new(),
				checked_jids: std::collections::HashSet::new(),
				current_jids: Vec::new(),
			},
			selected_chat: None,
			admin_group_jids: std::collections::HashSet::new(),
			initial_presence_received: false,
			messages: Vec::new(),
			messages_at_latest: true,
			loading_messages: false,
			no_more_messages: false,
			history_loading: false,
			newer_loading: false,
			phone_history_pending_jid: None,
			phone_history_deadline: None,
			history_sync_last_update: None,
			history_sync_armed_until: None,
			db_size_last_seen: 0,
			db_growth_last_seen: None,
			db_growth_status_shown: false,
			completed_history_chats: std::collections::HashSet::new(),
			quiet_reload_jid: None,
			prev_msg_count: 0,
			active_tab: 0,
			starred_entries: Vec::new(),
			filtered_starred_message_ids: Vec::new(),
			status_groups: Vec::new(),
			filtered_status_message_ids: Vec::new(),
			search_query: String::new(),
			displayed_chat_labels: Vec::new(),
			main_ui_shown: false,
			pending_whatsapp_link: None,
			conn_online: false,
			conn_outage_announced: false,
			conn_manual_reconnect: false,
			pending_attachments: Vec::new(),
			pending_attachment_kind: whantv::AttachKind::default(),
			pending_reply_id: None,
			pending_reply_sender: None,
			pending_reply_preview: None,
			editing_message_id: None,
			editing_message_original_text: None,
			draft_messages: std::collections::HashMap::new(),
			draft_mentions: std::collections::HashMap::new(),
			composer_mentions: Vec::new(),
			mention_candidates: None,
			mention_request_pending: None,
			mention_popup: None,
			mention_dismissed_at: None,
			pending_receipts: std::collections::HashMap::new(),
			downloads_in_progress: std::collections::HashMap::new(),
			pending_copy_announcements: std::collections::HashSet::new(),
			search_target_id: None,
			search_target_text: None,
			played_sent_sounds: std::collections::HashMap::new(),
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
			ui_commands::messaging::chatstate_tick(&mut s);
			ui_commands::misc::presence_tick(&mut s);
			if s.playing_msg_id.is_none() {
				return;
			}
			// Check if sink finished naturally
			let finished = s.audio_sink.as_ref().map(|sk| sk.empty()).unwrap_or(false);
			if finished && !s.is_audio_paused {
				// Voice notes sent one after another play on as a run, like
				// WhatsApp. Selection follows along (when it was on the one
				// that just ended), so NVDA reads out whose note is starting
				// and Space pauses the one actually playing.
				let finished_id = s.playing_msg_id.clone().unwrap_or_default();
				if let Some(next_id) = whantv::next_sequential_voice(&s.messages, &finished_id).map(|m| m.id.clone()) {
					let index_of = |id: &str| {
						s.display_items.iter().position(|item| match item {
							whantv::DisplayItem::Single(m) => m.id == id,
							whantv::DisplayItem::Album { messages, .. } => messages.iter().any(|m| m.id == id),
						})
					};
					let list = &parts_timer.conversations_parts.messages_list;
					if let (Some(from), Some(to)) = (index_of(&finished_id), index_of(&next_id)) {
						if list.get_selection() == Some(from as u32) {
							list.set_selection(to as u32, true);
						}
					}
					s.audio_sink = None;
					s.audio_elapsed_secs = 0.0;
					s.audio_start_instant = None;
					s.playing_msg_id = Some(next_id.clone());
					s.voice_autoplay_id = Some(next_id.clone());
					audio::play_notification_sound(
						NotificationSound::VoiceSequenceNext,
						s.settings.sounds_enabled,
						s.settings.output_device_name.clone(),
					);
					parts_timer.conversations_parts.play_voice_btn.set_label("Loading voice message...");
					let _ = s.ui_tx.send(UiCommand::PlayVoiceMessage(next_id));
					return;
				}
				if s.voice_autoplay_id.take().is_some_and(|id| id == finished_id) {
					audio::play_notification_sound(
						NotificationSound::VoiceSequenceEnd,
						s.settings.sounds_enabled,
						s.settings.output_device_name.clone(),
					);
				}
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
					let chat_name = s
						.contacts
						.get(sel_jid)
						.or_else(|| s.contacts.get(&norm_jid))
						.cloned()
						.unwrap_or_else(|| contact_picker::display_name_for_jid(sel_jid));
					let presence_label = s
						.presence_labels
						.get(sel_jid.as_str())
						.or_else(|| s.presence_labels.get(&norm_jid))
						.map(|l| l.as_str());
					let label = websocket_events::format_conv_data_label(
						sel_jid,
						&chat_name,
						&chat_name,
						false,
						false,
						presence_label,
					);
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
					s.active_call_video_send = None;
					s.call_video_port = None;
					s.call_participants.clear();
					s.call_video = CallVideo::default();
					parts_typing_timer.call_parts.frame.show(false);
					play_call_fail_sound(&mut s);
					// The call window is hidden just above, so this falls back to the
					// main window's announcer - which is the one with focus now.
					call_helpers::announce_call(&parts_typing_timer, "No response - the call could not be answered.");
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

			// The phone stays silent when there's nothing older to send back for
			// an on-demand history request - no "history_sync" ever arrives to
			// resolve `phone_history_pending_jid` one way or the other. Without
			// this deadline, `load_more_messages` would wait on it forever (stuck
			// spinner) and, worse, would be free to fire the same request again
			// on the next scroll-to-top, which is exactly the repeated on-demand
			// traffic that risks tripping WhatsApp's abuse detection.
			if let Some(deadline) = s.phone_history_deadline {
				if now >= deadline {
					if let Some(jid) = s.phone_history_pending_jid.take() {
						eprintln!("[LOG UI] Phone history request for {} timed out; not asking again.", jid);
						s.completed_history_chats.insert(jid);
					}
					s.phone_history_deadline = None;
					s.history_loading = false;
				}
			}

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

pub(crate) fn handle_ui_command(cmd: UiCommand, state: &Rc<RefCell<AppState>>, parts: &Rc<WindowParts>) {
	match cmd {
		UiCommand::PrepareNewGroup => ui_commands::contacts_groups::prepare_new_group(state, parts),
		UiCommand::PrepareAddMembers => ui_commands::contacts_groups::prepare_add_members(state, parts),
		UiCommand::AddGroupMembers(participants) => {
			ui_commands::contacts_groups::add_group_members(state, parts, participants)
		}
		UiCommand::CreateGroup { name, participants } => {
			ui_commands::contacts_groups::create_group(state, parts, name, participants)
		}
		UiCommand::NewContact { name, phone, sync } => {
			ui_commands::contacts_groups::new_contact(state, parts, name, phone, sync)
		}
		UiCommand::PrepareAttachContact => ui_commands::attachments::prepare_attach_contact(state, parts),
		UiCommand::SendContacts(jids) => ui_commands::attachments::send_contacts(state, parts, jids),
		UiCommand::RequestQr => ui_commands::misc::request_qr(state, parts),
		UiCommand::CancelQr => ui_commands::misc::cancel_qr(state, parts),
		UiCommand::CancelPairing => ui_commands::misc::cancel_pairing(state, parts),
		UiCommand::RequestPairing(phone) => ui_commands::misc::request_pairing(state, parts, phone),
		UiCommand::SendMessage(text) => ui_commands::messaging::send_message(state, parts, text),
		UiCommand::ConversationSelectionChanged(_idx) => {}
		UiCommand::ProcessWebSocketMessage(raw) => process_websocket_message(raw, state, parts),
		UiCommand::ExitApp => ui_commands::misc::exit_app(state, parts),
		UiCommand::NavSelected(idx) => ui_commands::chat_list::nav_selected(state, parts, idx),
		UiCommand::FilterConversations(query) => ui_commands::chat_list::filter_conversations(state, parts, query),
		UiCommand::OpenConversation(idx) => ui_commands::chat_list::open_conversation(state, parts, idx),
		UiCommand::OpenChatByPhone(digits) => ui_commands::chat_list::open_chat_by_phone(state, parts, digits),
		UiCommand::OpenChatByJid(jid) => ui_commands::chat_list::open_chat_by_jid(state, parts, jid),
		UiCommand::ToggleSelectionMode => ui_commands::chat_list::toggle_selection_mode(state, parts),
		UiCommand::DeleteSelectedMessages { for_everyone } => {
			ui_commands::chat_list::delete_selected_messages(state, parts, for_everyone)
		}
		UiCommand::LoadMoreMessages => ui_commands::chat_list::load_more_messages(state, parts),
		UiCommand::LoadNewerMessages => ui_commands::chat_list::load_newer_messages(state),
		UiCommand::JumpToLatestMessage => ui_commands::chat_list::jump_to_latest_message(state, parts),
		UiCommand::SearchInConversation(query) => ui_commands::chat_list::search_in_conversation(state, parts, query),
		UiCommand::ToggleSearchPanel => ui_commands::chat_list::toggle_search_panel(state, parts),
		UiCommand::PromptLockedChats => ui_commands::chat_list::prompt_locked_chats(state, parts),
		UiCommand::CloseLockedChats => ui_commands::chat_list::close_locked_chats(state, parts),
		UiCommand::CloseSearch => ui_commands::chat_list::close_search(state, parts),
		UiCommand::JumpToSearchResult { message_id, message_text } => {
			ui_commands::chat_list::jump_to_search_result(state, parts, message_id, message_text)
		}
		UiCommand::StartRecording => ui_commands::voice::start_recording(state, parts),
		UiCommand::StopRecording => ui_commands::voice::stop_recording(state, parts),
		UiCommand::TogglePauseRecording => ui_commands::voice::toggle_pause_recording(state, parts),
		UiCommand::VoiceRecordingReady => ui_commands::voice::voice_recording_ready(state, parts),
		UiCommand::ForceSync => ui_commands::misc::force_sync(state, parts),
		UiCommand::SendVoiceMessage => ui_commands::voice::send_voice_message(state, parts),
		UiCommand::DiscardRecording | UiCommand::DiscardVoiceMessage => {
			ui_commands::voice::discard_recording(state, parts)
		}
		UiCommand::AttachFiles(kind) => ui_commands::attachments::attach_files(state, parts, kind),
		UiCommand::AddAttachment => ui_commands::attachments::add_attachment(state, parts),
		UiCommand::OpenPollDialog => ui_commands::attachments::open_poll_dialog(state, parts),
		UiCommand::OpenEventDialog => ui_commands::polls::open_event_dialog(state, parts),
		UiCommand::OpenPollVote(id) => ui_commands::polls::open_poll_vote(state, parts, id),
		UiCommand::SendPoll { question, options, multiple_answers, correct_answer } => {
			ui_commands::attachments::send_poll(state, parts, question, options, multiple_answers, correct_answer)
		}
		UiCommand::PasteAttachment => ui_commands::attachments::paste_attachment(state, parts),
		UiCommand::DiscardAttachment => ui_commands::attachments::discard_attachment(state, parts),
		UiCommand::SendAttachment { caption } => ui_commands::attachments::send_attachment(state, parts, caption),
		UiCommand::DownloadMedia => ui_commands::attachments::download_media(state, parts),
		UiCommand::OpenMedia => ui_commands::attachments::open_media(state, parts),
		UiCommand::SaveMediaAs => ui_commands::attachments::save_media_as(state, parts),
		UiCommand::CancelEdit => ui_commands::messaging::cancel_edit(state, parts),
		UiCommand::BeginEditMessage { message_id, text } => {
			ui_commands::messaging::begin_edit_message(state, parts, message_id, text)
		}
		UiCommand::ShowConversationData => ui_commands::chat_list::show_conversation_data(state, parts),
		UiCommand::RemoveQuote => ui_commands::messaging::remove_quote(state, parts),
		UiCommand::SendTyping { jid, state: typing_state, media } => {
			ui_commands::messaging::send_typing(state, parts, jid, typing_state, media)
		}
		UiCommand::ForwardMessage { message_id, to_jids } => {
			ui_commands::messaging::forward_message(state, parts, message_id, to_jids)
		}
		UiCommand::PlaceCall => ui_commands::calls::place_call(state, parts),
		UiCommand::PlaceVideoCall => ui_commands::calls::place_video_call(state, parts),
		UiCommand::AnswerCall => ui_commands::calls::answer_call(state, parts),
		UiCommand::RejectCall => ui_commands::calls::reject_call(state, parts),
		UiCommand::LeaveCall => ui_commands::calls::leave_call(state, parts),
		UiCommand::ToggleCamera => ui_commands::calls::toggle_camera(state, parts),
		UiCommand::CameraFailed(reason) => ui_commands::calls::camera_failed(state, parts, reason),
		UiCommand::FilterGroupMembers(query) => ui_commands::contacts_groups::filter_group_members(state, parts, query),
		UiCommand::GroupMemberSelectionChanged => {
			ui_commands::contacts_groups::group_member_selection_changed(state, parts)
		}
		UiCommand::DownloadMediaMessage(message_id) => {
			ui_commands::attachments::download_media_message(state, parts, message_id)
		}
		UiCommand::OpenMediaMessage(message_id) => {
			ui_commands::attachments::open_media_message(state, parts, message_id)
		}
		UiCommand::SaveMediaMessageAs(message_id) => {
			ui_commands::attachments::save_media_message_as(state, parts, message_id)
		}
		UiCommand::MuteCall => ui_commands::calls::mute_call(state, parts),
		UiCommand::SetCallInputDevice(name) => ui_commands::calls::set_call_input_device(state, parts, name),
		UiCommand::SetCallOutputDevice(name) => ui_commands::calls::set_call_output_device(state, parts, name),
		UiCommand::SetCallCameraDevice(name) => ui_commands::calls::set_call_camera_device(state, parts, name),
		UiCommand::AudioDeviceChanged => ui_commands::voice::handle_audio_device_changed(state),
		UiCommand::PrepareContactPicker(mode) => {
			ui_commands::contacts_groups::prepare_contact_picker(state, parts, mode)
		}
		UiCommand::FilterContactPicker(query) => {
			ui_commands::contacts_groups::filter_contact_picker(state, parts, query)
		}
		UiCommand::SelectContactPickerResult(idx) => {
			ui_commands::contacts_groups::select_contact_picker_result(state, parts, idx)
		}
		UiCommand::SubscribePresence(jid) => ui_commands::misc::subscribe_presence(state, parts, jid),
		UiCommand::ActivateWindow => ui_commands::misc::activate_window(state, parts),
		UiCommand::OpenWhatsAppLink(link) => ui_commands::links::open_whatsapp_link(state, parts, link),
		UiCommand::ShowGroupInviteLink => ui_commands::links::request_group_invite_link(state),
		UiCommand::FindChannels => ui_commands::links::show_channel_directory(state, parts, None),
		UiCommand::SimilarChannels { jid, name } => {
			ui_commands::links::show_channel_directory(state, parts, Some((jid, name)))
		}
		UiCommand::CreateChannel => ui_commands::channel_admin::create_channel(state, parts),
		UiCommand::JoinGroupLink(code) => {
			let payload = serde_json::json!({ "action": "join_group_link", "data": { "code": code } });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::SetPresence(available) => ui_commands::misc::set_presence(state, parts, available),
		UiCommand::WindowHidden => ui_commands::misc::window_hidden(state, parts),
		UiCommand::MarkRead { jid, message_ids } => ui_commands::chat_list::mark_read(state, parts, jid, message_ids),
		UiCommand::PlayVoiceMessage(msg_id) => ui_commands::voice::play_voice_message(state, parts, msg_id),
		UiCommand::CloseConversation => ui_commands::chat_list::close_conversation(state, parts),
		UiCommand::EditMessage { message_id, new_text } => {
			ui_commands::messaging::edit_message(state, parts, message_id, new_text)
		}
		UiCommand::DeleteMessage { message_id, sender_jid, for_everyone } => {
			ui_commands::messaging::delete_message(state, parts, message_id, sender_jid, for_everyone)
		}
		UiCommand::StarMessage { message_id, sender_jid, from_me, starred } => {
			ui_commands::messaging::star_message(state, parts, message_id, sender_jid, from_me, starred)
		}
		UiCommand::ExportChat { jid, dest_path } => ui_commands::messaging::export_chat(state, parts, jid, dest_path),
		UiCommand::ExportChatText { jid, dest_path } => {
			ui_commands::messaging::export_chat_text(state, parts, jid, dest_path)
		}
		UiCommand::PostStatus { text } => ui_commands::messaging::post_status(state, parts, text),
		UiCommand::PostMediaStatus => ui_commands::messaging::post_media_status(state, parts),
		UiCommand::GetMessageInfo { message_id } => ui_commands::messaging::get_message_info(state, parts, message_id),
		UiCommand::RetryMessage { message_id } => ui_commands::messaging::retry_message(state, parts, message_id),
		UiCommand::RetrySelectedMessage => ui_commands::messaging::retry_selected_message(state, parts),
		UiCommand::RequestMessageResend { message_id } => {
			ui_commands::messaging::request_message_resend(state, parts, message_id)
		}
		UiCommand::CopyAlbumToClipboard { message_ids } => {
			ui_commands::attachments::copy_album_to_clipboard(state, parts, message_ids)
		}
		UiCommand::SeekAudio(delta) => ui_commands::voice::seek_audio(state, parts, delta),
		UiCommand::RefreshMessages => {}
		UiCommand::DownloadAttachment(_) => {}
		UiCommand::ChangeAudioSpeed => {}
		UiCommand::UpdateGroupSettings { name, topic, announce, locked } => {
			ui_commands::contacts_groups::update_group_settings(state, parts, name, topic, announce, locked)
		}
		UiCommand::ToggleGroupMemberAdmin(idx) => {
			ui_commands::contacts_groups::toggle_group_member_admin(state, parts, idx)
		}
		UiCommand::RemoveGroupMember(idx) => ui_commands::contacts_groups::remove_group_member(state, parts, idx),
		UiCommand::JoinCommunitySubgroup(idx) => {
			ui_commands::contacts_groups::join_community_subgroup(state, parts, idx)
		}
		UiCommand::CommunitySubgroupSelectionChanged => {
			ui_commands::contacts_groups::community_subgroup_selection_changed(state, parts)
		}
		UiCommand::OpenChannelAdminInvite(message_id) => {
			ui_commands::channel_admin::open_channel_admin_invite(state, message_id)
		}
		UiCommand::OpenGroupInvite(message_id) => {
			ui_commands::contacts_groups::open_group_invite(state, parts, message_id)
		}
		UiCommand::JoinGroupInvite(message_id) => {
			ui_commands::contacts_groups::join_group_invite(state, parts, message_id)
		}
		UiCommand::SeekAudioAbsolute(slider_val) => ui_commands::voice::seek_audio_absolute(state, parts, slider_val),
		UiCommand::ReactToMessage { message_id, sender_jid, emoji } => {
			ui_commands::messaging::react_to_message(state, parts, message_id, sender_jid, emoji)
		}
		UiCommand::ToggleAudioSpeed => ui_commands::voice::toggle_audio_speed(state, parts),
		UiCommand::ToggleLockChat { jid, lock } => ui_commands::chat_list::toggle_lock_chat(state, parts, jid, lock),
		UiCommand::ChooseDisappearingTimer { jid } => {
			ui_commands::chat_list::choose_disappearing_timer(state, parts, jid)
		}
		UiCommand::ReportMessage { message_id } => ui_commands::report::report_message(state, parts, message_id),
		UiCommand::ReportToAdmins { message_id } => ui_commands::report::report_to_admins(state, parts, message_id),
		UiCommand::ReportChat { jid } => ui_commands::report::report_chat(state, parts, jid),
		UiCommand::UpdateGroupModeration { no_frequently_forwarded, allow_admin_reports, group_history } => {
			ui_commands::report::update_group_moderation(
				state,
				no_frequently_forwarded,
				allow_admin_reports,
				group_history,
			)
		}
		UiCommand::ShowReportedMessages => ui_commands::report::request_reported_messages(state),
		UiCommand::SetMemberLabel(label) => ui_commands::contacts_groups::set_member_label(state, parts, label),
		UiCommand::TogglePinChat { jid, pin } => ui_commands::chat_list::toggle_pin_chat(state, parts, jid, pin),
		UiCommand::ToggleArchiveChat { jid, archive } => {
			ui_commands::chat_list::toggle_archive_chat(state, parts, jid, archive)
		}
		UiCommand::SetArchivedAlert { jid, level } => {
			ui_commands::chat_list::set_archived_alert(state, parts, jid, level)
		}
		UiCommand::ToggleMuteChat { jid, mute } => ui_commands::chat_list::toggle_mute_chat(state, parts, jid, mute),
		UiCommand::ToggleReadChat { jid, read } => ui_commands::chat_list::toggle_read_chat(state, parts, jid, read),
		UiCommand::DeleteChat { jid } => ui_commands::chat_list::delete_chat(state, parts, jid),
		UiCommand::DeleteChatAndLeaveGroup { jid } => {
			ui_commands::chat_list::delete_chat_and_leave_group(state, parts, jid)
		}
		UiCommand::ClearChat { jid } => ui_commands::chat_list::clear_chat(state, parts, jid),
		UiCommand::LeaveGroup { jid } => ui_commands::contacts_groups::leave_group(state, parts, jid),
	}
}
