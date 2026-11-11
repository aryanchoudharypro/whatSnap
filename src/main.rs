#![windows_subsystem = "windows"]
mod audio;
mod backend;
mod calls;
mod countries;
mod error;
mod keymap;
mod media;
mod platform;
mod settings;
mod types;
mod ui;
mod ui_commands;
mod util;

use std::{
	cell::RefCell,
	rc::Rc,
	sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
	time::Duration,
};

use audio::*;
use backend::events::process_websocket_message;
use live_region::Priority;
pub use platform::clipboard::{copy_files_to_clipboard, copy_text};
use rodio::Player;
pub use types::*;
use ui::message_display::update_action_buttons;
use wxdragon::prelude::*;

use crate::ui::{
	wake::{ID_UI_WAKE, UI_WAKE_HANDLERS, UiCommandSender, UiWaker},
	window::{WindowParts, bind_input_handlers, build_main_window},
};

pub const MESSAGE_PAGE_SIZE: i32 = 500;

pub const MAX_LOADED_MESSAGES: usize = 2000;

const PHONE_HISTORY_TIMEOUT_SECS: u64 = 30;

const PRESENCE_BLUR_GRACE: Duration = Duration::from_millis(15_000);
const PRESENCE_FOCUS_SETTLE: Duration = Duration::from_millis(500);
const FOCUS_READ_DELAY: Duration = Duration::from_millis(2_500);
const PRESENCE_IDLE_CHECK_EVERY: Duration = Duration::from_millis(60_000);
const PRESENCE_IDLE_GRACE: Duration = Duration::from_millis(30_000);
const CHATSTATE_PAUSE_AFTER: Duration = Duration::from_millis(2_500);
const CHATSTATE_RESEND_EVERY: Duration = Duration::from_millis(10_000);

pub(crate) struct ComposingState {
	jid: String,
	media: String,
	last_sent: std::time::Instant,
	last_key: std::time::Instant,
}

pub(crate) struct PresenceModel {
	available: bool,
	unavailable_at: Option<std::time::Instant>,
	focus_settle_since: Option<std::time::Instant>,
	hidden: bool,
	next_idle_check: std::time::Instant,
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

impl AppState {
	/// Whether the call window should say "Incoming". That wording only
	/// applies while the call is still ringing; once it is answered and
	/// connected the window shows "call with" the person.
	fn call_shows_incoming(&self) -> bool {
		self.call_is_incoming && self.active_call_audio.is_none()
	}

	fn call_info_direction(&self) -> crate::calls::helpers::CallDirection {
		use crate::calls::helpers::CallDirection;
		if self.call_shows_incoming() {
			CallDirection::Incoming
		} else if self.call_is_incoming {
			CallDirection::Answered
		} else {
			CallDirection::Outgoing
		}
	}
}

struct AppState {
	/// The linked accounts and their backend links. Survives switching accounts.
	accounts: Rc<RefCell<ui_commands::accounts::Accounts>>,
	settings: crate::settings::Settings,
	keymap: Rc<crate::keymap::Keymap>,
	ws_tx: mpsc::Sender<String>,
	ui_tx: UiCommandSender,
	/// Alert and voice message output (notification sounds, voice messages),
	/// on the alert output device. See [`audio::OutputSlot`] for how the
	/// stream is kept alive and reopened.
	audio_out: audio::OutputSlot,
	/// Call output (ringing and call tones), on the call output device. Call
	/// audio itself has its own streams in `calls::audio`.
	call_audio_out: audio::OutputSlot,
	audio_sink: Option<Player>,
	playing_msg_id: Option<String>,
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
	voice_recording_session: Option<audio::voice_recorder::RecordingSession>,
	voice_recording_segments: Vec<String>,
	voice_recording_start: Option<std::time::Instant>,
	voice_paused_duration: std::time::Duration,
	voice_pause_instant: Option<std::time::Instant>,
	voice_preview_sink: Option<Player>,
	voice_preview_merged_path: Option<String>,
	is_voice_preview_playing: bool,
	voice_recording_generation: u64,
	/// True while a `RecordingSession::start_async` call kicked off by the
	/// current `voice_recording_generation` hasn't reported back yet - guards
	/// against a second `StartRecording`/resume landing before the first
	/// device open finishes (the record/pause-resume buttons are also
	/// disabled for this, but that's belt-and-suspenders against e.g. a
	/// screen reader's synthesized click bypassing the disabled state).
	voice_recording_opening: bool,
	voice_recording_pending: Arc<Mutex<Option<PendingVoiceRecording>>>,
	is_pairing_active: bool,
	chats: Vec<String>,
	filtered_chats: Vec<String>,
	archived_jids: std::collections::HashSet<String>,
	locked_jids: std::collections::HashSet<String>,
	chats_unlocked: bool,
	pending_locked_chat: Option<String>,
	community_jids: std::collections::HashSet<String>,
	chat_parents: std::collections::HashMap<String, String>,
	announcement_jids: std::collections::HashSet<String>,
	channel_jids: std::collections::HashSet<String>,
	channel_roles: std::collections::HashMap<String, String>,
	channel_pinning: bool,
	channel_directory: Option<Rc<crate::ui::dialogs::channel_directory::ChannelDirectory>>,
	emoji_panel: Option<Rc<crate::ui::emoji_panel::EmojiPanel>>,
	pinned_jids: std::collections::HashSet<String>,
	muted_jids: std::collections::HashSet<String>,
	group_quiet_until: std::collections::HashMap<String, std::time::Instant>,
	blocked_jids: std::collections::HashSet<String>,
	blocked_entries: Vec<crate::ui::dialogs::blocked_contacts::BlockedEntry>,
	blocked_contacts: Option<Rc<crate::ui::dialogs::blocked_contacts::BlockedContacts>>,
	/// Whether to offer scheduling messages: WA Web's
	/// scheduled_messages_sender_enabled, on until the server says it's off.
	scheduled_messages_enabled: bool,
	/// The chat and text the schedule dialog is waiting on the backend's
	/// options for.
	pending_schedule: Option<(String, String)>,
	scheduled_list: Option<Rc<crate::ui::dialogs::scheduled_messages::ScheduledList>>,
	unread_counts: std::collections::HashMap<String, u64>,
	unread_divider_jid: Option<String>,
	unread_divider_count: u64,
	window_focused: bool,
	presence: PresenceModel,
	contact_online: std::collections::HashMap<String, bool>,
	/// The presence last sent to the backend, so repeats are never re-sent.
	presence_sent: Option<bool>,
	composing: Option<ComposingState>,
	background_unread_message_ids: Vec<String>,
	/// When the window came back with a chat open: the chat to mark read
	/// once it has stayed in front this long (WA Web waits 2.5 s).
	focus_read_at: Option<(std::time::Instant, String)>,
	notification_reads: std::collections::HashSet<String>,
	display_names: std::collections::HashMap<String, String>,
	contacts: std::collections::HashMap<String, String>,
	all_contacts: Vec<Contact>,
	contact_picker_mode: ContactPickerMode,
	contact_picker_current_jids: Vec<String>,
	presence_labels: std::collections::HashMap<String, String>,
	group_state: crate::ui::dialogs::new_group::NewGroupState,
	add_members_state: crate::ui::dialogs::add_members::AddMembersState,
	attach_contact_state: crate::ui::dialogs::attach_contact::AttachContactState,
	selected_chat: Option<String>,
	admin_group_jids: std::collections::HashSet<String>,
	admins_only_jids: std::collections::HashSet<String>,
	not_member_jids: std::collections::HashSet<String>,
	business_jids: std::collections::HashSet<String>,
	initial_presence_received: bool,
	messages: Vec<whatsnap::Msg>,
	messages_at_latest: bool,
	loading_messages: bool,
	no_more_messages: bool,
	history_loading: bool,
	newer_loading: bool,
	older_unloaded: usize,
	newer_unloaded: usize,
	land_on_first: bool,
	phone_history_pending_jid: Option<String>,
	phone_history_deadline: Option<std::time::Instant>,
	history_sync_last_update: Option<std::time::Instant>,
	history_sync_armed_until: Option<std::time::Instant>,
	db_size_last_seen: u64,
	db_growth_last_seen: Option<std::time::Instant>,
	db_growth_status_shown: bool,
	/// The phone's recent history sync after linking (WA Web's "Syncing
	/// older messages, N% complete" in its You tab): running, its percent
	/// (-1 before the first chunk) and whether the phone stopped sending.
	recent_sync_active: bool,
	recent_sync_percent: i64,
	recent_sync_paused: bool,
	/// Percent of the backlog kept while whatSnap was offline, while it's
	/// still downloading (WA Web's "Downloading messages: N%").
	offline_sync_percent: Option<i64>,
	history_start_chats: std::collections::HashSet<String>,
	/// Chats that were opened with no messages and had the phone asked for
	/// their history automatically. Never cleared: once per run is enough.
	empty_chat_history_asked: std::collections::HashSet<String>,
	quiet_reload_jid: Option<String>,
	prev_msg_count: usize,
	active_tab: usize,
	starred_entries: Vec<StarredEntry>,
	filtered_starred_message_ids: Vec<String>,
	status_groups: Vec<StatusGroup>,
	filtered_status_message_ids: Vec<String>,
	/// The call id behind each Calls tab row, parallel to filtered_chats; empty
	/// for headings and rows that aren't a call from the call history.
	filtered_call_ids: Vec<String>,
	call_logs: Vec<CallLogEntry>,
	call_frequent: Vec<CallTarget>,
	call_contacts: Vec<CallTarget>,
	search_query: String,
	/// The message behind each row of the chat list's search results,
	/// parallel to filtered_chats; empty for chats, contacts and headings.
	filtered_message_ids: Vec<String>,
	message_search: MessageSearch,
	chat_search_keys: std::collections::HashMap<String, ChatSearchKeys>,
	/// Each chat's last-message line as its row's label ends with it, so a
	/// draft can take its place (see `message_display::chat_row_label`).
	chat_preview_lines: std::collections::HashMap<String, String>,
	displayed_chat_labels: Vec<String>,
	chat_row_art: std::collections::HashMap<String, crate::ui::chat_rows::RowArt>,
	main_ui_shown: bool,
	/// The startup update check, held back while the app sits in the tray after
	/// starting with Windows, so no update window opens on its own.
	pending_update_check: bool,
	pending_whatsapp_link: Option<String>,
	/// Replies typed into notifications before the chats were loaded.
	pending_notification_replies: Vec<(String, String)>,
	pending_shared_files: Vec<String>,
	conn_online: bool,
	conn_status_text: String,
	conn_outage_announced: bool,
	conn_manual_reconnect: bool,
	pending_attachments: Vec<String>,
	pending_attachment_kind: whatsnap::AttachKind,
	pending_reply_id: Option<String>,
	pending_reply_sender: Option<String>,
	pending_reply_preview: Option<String>,
	editing_message_id: Option<String>,
	editing_message_original_text: Option<String>,
	/// Unsent composer text per chat, saved whenever the conversation is
	/// switched away from (closing it, or opening a different chat) and
	/// restored when that chat is opened again - so leaving mid-draft (e.g.
	/// Escape) never loses what was typed. Cleared once the draft is
	/// actually sent.
	draft_messages: std::collections::HashMap<String, String>,
	draft_mentions: std::collections::HashMap<String, Vec<ui_commands::mentions::MentionEntry>>,
	composer_mentions: Vec<ui_commands::mentions::MentionEntry>,
	mention_candidates: Option<ui_commands::mentions::MentionCandidates>,
	mention_request_pending: Option<String>,
	mention_popup: Option<ui_commands::mentions::MentionPopup>,
	mention_dismissed_at: Option<usize>,
	pending_receipts: std::collections::HashMap<String, String>,
	downloads_in_progress: std::collections::HashMap<String, i64>,
	uploads_in_progress: std::collections::HashMap<String, i64>,
	pending_copy_announcements: std::collections::HashSet<String>,
	search_target_id: Option<String>,
	search_target_text: Option<String>,
	catch_up_alerts: whatsnap::CatchUpAlerts,
	catch_up_last_at: Option<std::time::Instant>,
	display_items: Vec<whatsnap::DisplayItem>,
	selection_mode: bool,
	selected_message_ids: std::collections::HashSet<String>,
	selection_rows: Vec<whatsnap::DisplayItem>,
	bulk_delete_pending: std::collections::HashSet<(String, String)>,
	/// Messages starred, unstarred, kept or unkept together from the select
	/// bar, which already said so once: their own confirmations stay quiet.
	bulk_quiet_pending: std::collections::HashSet<String>,
	typing_timestamps: std::collections::HashMap<String, std::time::Instant>,
	// Last announced typing/recording state per chat (0=none, 1=typing, 2=recording),
	// so a screen reader hears "X is typing" once per state change instead of on
	// every composing stanza WhatsApp resends while the sender keeps typing.
	typing_announce_state: std::collections::HashMap<String, u8>,
	active_call_audio: Option<calls::audio::CallAudioState>,
	active_call_video_send: Option<calls::video_send::CallVideoSendState>,
	call_video_port: Option<u16>,
	active_call_id: Option<String>,
	call_placing: bool,
	call_pending_cancel: bool,
	joinable_calls: Vec<ui_commands::calls::JoinableCall>,
	/// Set when we send answer_call for an incoming call, cleared on
	/// call_connected/call_ended/call_answer_failed. If it's still set once
	/// this has been elapsed too long, the periodic timer treats it as a
	/// silently-failed answer (see typing_timer's tick handler) - covers the
	/// case where the backend never hears back at all rather than erroring.
	call_answer_deadline: Option<std::time::Instant>,
	call_ringing_sink: Option<Player>,
	/// True only while the outgoing ringback itself is playing: the sink above
	/// also holds the hangup sound and ringtones, which stay set once finished.
	call_ringback_on: bool,
	call_reconnect_sink: Option<Player>,
	call_hotkeys: Option<wx_utils::global_hotkeys::GlobalHotkeys>,
	incoming_call_caller: Option<String>,
	call_start_muted: bool,
	call_connected_segment_start: Option<std::time::Instant>,
	call_connected_duration: std::time::Duration,
	call_is_reconnecting: bool,
	call_is_incoming: bool,
	call_peer_name: String,
	call_peer_is_saved: bool,
	/// Who the last call we placed went to and whether it was video, for
	/// "Call back" after the call has ended and its state was reset.
	call_last_target: Option<(String, bool)>,
	call_self_phone: String,
	call_participants: Vec<CallParticipant>,
	call_video: CallVideo,
	group_data_members: Vec<String>,
	group_data_member_jids: Vec<String>,
	group_data_member_info: Vec<GroupMember>,
	group_data_visible_member_jids: Vec<String>,
	group_data_media: Vec<whatsnap::Msg>,
	group_data_documents: Vec<whatsnap::Msg>,
	group_data_links: Vec<whatsnap::Msg>,
	group_data_subgroup_jids: Vec<String>,
	group_data_subgroup_joined: Vec<bool>,
	group_data_community: Option<String>,
	conversation_data_media: Vec<whatsnap::Msg>,
	conversation_data_documents: Vec<whatsnap::Msg>,
	conversation_data_links: Vec<whatsnap::Msg>,
	conversation_data_contact: Option<ui_commands::contacts_groups::ContactCard>,
	info_chat: Option<String>,
	/// The Overview tab asked the backend whether the chat is restricted.
	overview_probe: bool,
	own_push_name: String,
	own_about: String,
	own_about_duration: i64,
	own_about_loaded: bool,
	default_timer: Option<i64>,
	chat_filter: usize,
	favorite_jids: Vec<String>,
	chat_lists: Vec<ui_commands::chat_lists::ChatList>,
	list_manager: Option<Rc<ui_commands::chat_lists::ListManager>>,
	common_group_jids: Vec<String>,
	privacy_settings: std::collections::HashMap<String, String>,
	/// Whether `privacy_settings` came from the server since Settings was
	/// last opened. Until then the privacy dropdowns are disabled and Save
	/// leaves the privacy settings alone, so a dropdown's placeholder can
	/// never overwrite the real setting.
	privacy_settings_loaded: bool,
	app_prefs: Option<serde_json::Value>,
	privacy_lists: std::collections::HashMap<String, Result<Vec<Contact>, String>>,
	privacy_list_edits: std::collections::HashMap<String, Vec<String>>,
}

fn launch_request_from_args(args: impl Iterator<Item = String>) -> Option<platform::single_instance::LaunchRequest> {
	let args: Vec<String> = args.collect();
	if let Some(link) =
		args.iter().find(|arg| whatsnap::parse_whatsapp_link(arg).is_some() || platform::app_link::parse(arg).is_some())
	{
		return Some(platform::single_instance::LaunchRequest::Link(link.clone()));
	}
	let files: Vec<String> = args
		.iter()
		.map(|arg| std::path::Path::new(arg.trim_matches('"')))
		.filter(|path| path.is_file())
		.filter_map(|path| std::path::absolute(path).ok())
		.map(|path| path.to_string_lossy().to_string())
		.collect();
	(!files.is_empty()).then_some(platform::single_instance::LaunchRequest::Files(files))
}

fn launch_command(request: platform::single_instance::LaunchRequest) -> UiCommand {
	match request {
		platform::single_instance::LaunchRequest::Link(link) => UiCommand::OpenWhatsAppLink(link),
		platform::single_instance::LaunchRequest::Files(paths) => UiCommand::ShareFiles(paths),
		platform::single_instance::LaunchRequest::Reply { jid, account, text } => {
			UiCommand::NotificationReply { jid, account, text }
		}
	}
}

fn main() {
	platform::crash_log::install();

	let background_start = platform::autostart::note_launch_args(std::env::args().skip(1));
	let launch_request = launch_request_from_args(std::env::args().skip(1));
	// Started by Windows for a notification: what was pressed arrives through
	// the toast activator, not the command line. The window stays hidden
	// unless that turns out to be a click that asks for it.
	#[cfg(target_os = "windows")]
	let toast_start = platform::toast_activator::launched_for_toast(std::env::args().skip(1));
	#[cfg(not(target_os = "windows"))]
	let toast_start = false;
	// A start with Windows never pulls a copy that's already running to the front.
	if (background_start || toast_start) && platform::single_instance::is_running() {
		return;
	}
	if toast_start {
		platform::autostart::start_hidden();
	}
	let _instance_lock = match platform::single_instance::acquire_or_activate_existing(launch_request.as_ref()) {
		Some(lock) => lock,
		None => return,
	};
	if !background_start {
		std::thread::spawn(platform::autostart::refresh);
	}
	platform::single_instance::listen_for_activation();
	#[cfg(target_os = "windows")]
	{
		platform::toast_activator::start();
		platform::focus::track_foreground();
	}
	platform::paths::migrate_legacy_data();
	#[cfg(target_os = "windows")]
	std::thread::spawn(platform::link_handler::register);
	for name in ["backend_process.log", "call_audio.log", "call_video_send.log", "panic.log", "crash.log"] {
		platform::paths::rotate_log(name);
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

	let early_settings = crate::settings::Settings::load();
	let verbose_backend_logging = early_settings.verbose_backend_logging;
	crate::ui::theme::set_theme(early_settings.theme);

	// One backend runs every linked account, so all of them stay online.
	// The active one is opened first so its window fills in soonest.
	let accounts = ui_commands::accounts::Accounts::load_at_startup();
	backend::links::set_launcher(backend_exe, backend_dir, verbose_backend_logging);
	let mut start_order: Vec<String> = vec![accounts.registry.active.clone()];
	start_order.extend(accounts.registry.live().map(|a| a.id.clone()).filter(|id| *id != accounts.registry.active));
	let links: Vec<backend::links::Link> = start_order.iter().map(|id| backend::links::start(id)).collect();
	// Killing the backend outright skips both the WhatsApp disconnects and the
	// database closes, which strands the SQLite -wal/-shm files. Exit asks it
	// to stop on its own first. (Its kill-on-close job is the backstop if it
	// never comes down: the handle closes when this process exits.)
	platform::quit::on_backend_shutdown(backend::links::shut_down_all);

	let res = wxdragon::main(move |_| {
		crate::ui::theme::apply_to_windows();
		let parts = build_main_window();
		let frame = parts.frame;

		#[cfg(target_os = "windows")]
		{
			let tray_icon = parts.tray_icon.clone();
			platform::end_session::install(&frame, move || {
				tray_icon.remove_icon();
				platform::quit::finish();
			});
		}

		let (ui_tx_raw, ui_rx) = mpsc::channel();
		let ui_alive = Arc::new(AtomicBool::new(true));
		let ui_waker = UiWaker::new(frame, ui_alive.clone());
		let ui_tx = UiCommandSender::new(ui_tx_raw, ui_waker.clone());

		audio::device_watch::start(ui_tx.clone());

		let ui_tx_activate = ui_tx.clone();
		platform::single_instance::on_activation(move |request| {
			if !platform::single_instance::works_in_background(&request) {
				platform::autostart::show_on_start();
				let _ = ui_tx_activate.send(UiCommand::ActivateWindow);
			}
			if let Some(request) = request {
				let _ = ui_tx_activate.send(launch_command(request));
			}
		});
		if let Some(request) = launch_request.clone() {
			let _ = ui_tx.send(launch_command(request));
		}

		backend::links::attach_ui(ui_tx.clone(), ui_alive.clone());
		let ws_out_tx = links[0].ws_tx.clone();
		let accounts = Rc::new(RefCell::new(accounts.with_links(links)));

		let loaded_settings = crate::settings::Settings::load();
		let saved_audio_speed = loaded_settings.audio_speed();
		let state = Rc::new(RefCell::new(new_app_state(
			loaded_settings,
			ws_out_tx,
			ui_tx.clone(),
			audio::OutputSlot::new("alert"),
			audio::OutputSlot::new("call"),
			accounts,
		)));

		let state_clone = state.clone();
		let parts_clone = Rc::new(parts);
		let parts_ui = parts_clone.clone();

		let ui_rx = Rc::new(ui_rx);
		for slot in 0..UI_WAKE_HANDLERS {
			let (ui_waker, ui_rx, state, parts) =
				(ui_waker.clone(), ui_rx.clone(), state_clone.clone(), parts_ui.clone());
			frame.bind_with_id_internal(EventType::MENU, ID_UI_WAKE + slot as i32, move |_| {
				ui_waker.run(|| {
					while let Ok(cmd) = ui_rx.try_recv() {
						handle_ui_command(cmd, &state, &parts);
					}
				});
			});
		}

		bind_input_handlers(&parts_clone, ui_tx, &state);
		ui_commands::accounts::refresh_title(&state, &parts_clone);
		parts_clone.conversations_parts.audio_speed_btn.set_label(&format!("{:.1}x", saved_audio_speed));

		let audio_timer = Timer::new(&frame);
		let state_timer = state.clone();
		let parts_timer = parts_clone.clone();
		audio_timer.on_tick(move |_| {
			let mut s = match state_timer.try_borrow_mut() {
				Ok(s) => s,
				Err(_) => return,
			};
			ui_commands::messaging::chatstate_tick(&mut s);
			ui::chat_ui::flush_catch_up_alerts(&mut s);
			platform::paths::remove_legacy_data_if_running(s.main_ui_shown && !s.chats.is_empty());
			ui_commands::misc::presence_tick(&mut s);
			if s.call_connected_segment_start.is_some() || s.call_is_reconnecting {
				let mut total = s.call_connected_duration;
				if let Some(start) = s.call_connected_segment_start {
					total += start.elapsed();
				}
				let secs = total.as_secs();
				let info = calls::helpers::call_running_info_text(
					s.call_info_direction(),
					s.call_video.is_video,
					&s.call_peer_name,
					secs,
					s.call_is_reconnecting,
				);
				if parts_timer.call_parts.caller_info.get_value() != info {
					parts_timer.call_parts.caller_info.set_value(&info);
				}
			}
			if s.is_voice_preview_playing {
				let preview_finished = s.voice_preview_sink.as_ref().map(|sk| sk.empty()).unwrap_or(false);
				if preview_finished {
					s.voice_preview_sink = None;
					s.is_voice_preview_playing = false;
					parts_timer.conversations_parts.preview_voice_btn.set_label("Play preview");
					crate::platform::announce::announce(
						parts_timer.conversations_parts.messages_announcer,
						"Voice preview finished.",
					);
				}
			}
			if s.playing_msg_id.is_none() {
				return;
			}
			let finished = s.audio_sink.as_ref().map(|sk| sk.empty()).unwrap_or(false);
			if finished && !s.is_audio_paused {
				// Voice notes sent one after another play on as a run, like
				// WhatsApp. Selection follows along (when it was on the one
				// that just ended), so NVDA reads out whose note is starting
				// and Space pauses the one actually playing.
				let finished_id = s.playing_msg_id.clone().unwrap_or_default();
				if let Some(next_id) = whatsnap::next_sequential_voice(&s.messages, &finished_id).map(|m| m.id.clone())
				{
					let index_of = |id: &str| {
						s.display_items.iter().position(|item| match item {
							whatsnap::DisplayItem::Single(m) => m.id == id,
							whatsnap::DisplayItem::Album { messages, .. } => messages.iter().any(|m| m.id == id),
						})
					};
					let list = &parts_timer.conversations_parts.messages_list;
					if let (Some(from), Some(to)) = (index_of(&finished_id), index_of(&next_id))
						&& list.get_selection() == Some(from as u32)
					{
						list.set_selection(to as u32, true);
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
				parts_timer.conversations_parts.audio_progress_label.set_label("0:00 / 0:00");
				parts_timer.conversations_parts.audio_progress_label.show(false);
				return;
			}
			if !s.is_audio_paused
				&& let Some(inst) = s.audio_start_instant
			{
				let elapsed = s.audio_elapsed_secs + inst.elapsed().as_secs_f32() * s.audio_speed;
				let total = s.audio_total_secs;
				let max_sec = (total as i32).max(1);
				let pos = (elapsed as i32).max(0).min(max_sec);
				if parts_timer.conversations_parts.audio_slider.is_shown() {
					parts_timer.conversations_parts.audio_slider.set_range(0, max_sec);
					parts_timer.conversations_parts.audio_slider.set_value(pos);
				}
				if parts_timer.conversations_parts.audio_progress_label.is_shown() {
					let fmt_dur = |secs: i32| format!("{}:{:02}", secs / 60, secs % 60);
					let progress = format!("{} / {}", fmt_dur(pos), fmt_dur(max_sec));
					if parts_timer.conversations_parts.audio_progress_label.get_label() != progress {
						parts_timer.conversations_parts.audio_progress_label.set_label(&progress);
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
			if let Some(ref sel_jid) = selected
				&& expired.iter().any(|j| j == sel_jid)
			{
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
					.unwrap_or_else(|| ui::contact_picker::display_name_for_jid(sel_jid));
				let presence_label = s
					.presence_labels
					.get(sel_jid.as_str())
					.or_else(|| s.presence_labels.get(&norm_jid))
					.map(|l| l.as_str());
				let label = backend::events::format_conv_data_label(
					sel_jid,
					&chat_name,
					&chat_name,
					false,
					false,
					presence_label,
				);
				parts_typing_timer.conversations_parts.conv_data_btn.set_label(&label);
			}

			// Watchdog for a silently-failed call answer: if the backend never
			// sends back call_connected, call_ended, or call_answer_failed at
			// all (e.g. the callee's media path never comes up), the UI would
			// otherwise sit on "Answering..." forever with no way to tell the
			// Accept press did nothing.
			if let Some(deadline) = s.call_answer_deadline
				&& deadline.elapsed() >= std::time::Duration::from_secs(15)
			{
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
				calls::helpers::announce_call(
					&parts_typing_timer,
					"No response - the call could not be answered.",
					Priority::High,
				);
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
			let active_db = platform::accounts::db_path(&s.accounts.borrow().registry.active);
			let db_size = platform::paths::db_and_wal_size(&active_db);
			if db_size > s.db_size_last_seen {
				s.db_growth_last_seen = Some(now);
			}
			s.db_size_last_seen = db_size;

			if let Some(deadline) = s.phone_history_deadline
				&& now >= deadline
			{
				if let Some(jid) = s.phone_history_pending_jid.take() {
					eprintln!("[LOG UI] Phone history request for {} timed out; Home at the top asks again.", jid);
				}
				s.phone_history_deadline = None;
				s.history_loading = false;
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

			let sync_reported = s.recent_sync_active || s.offline_sync_percent.is_some();
			if armed && db_growth_fresh && s.conn_online {
				let label = if sync_reported {
					connected_status_text(&s)
				} else {
					"Status: Connected (Syncing...)".to_string()
				};
				parts_history_sync_timer.frame.set_status_text(&label, 0);
				s.db_growth_status_shown = true;
			} else if real_signal_just_expired || s.db_growth_status_shown {
				if s.conn_online {
					parts_history_sync_timer.frame.set_status_text(&connected_status_text(&s), 0);
				}
				s.db_growth_status_shown = false;
			}
		});
		history_sync_timer.start(1500, false);
		Box::leak(Box::new(history_sync_timer));

		// Deliberately show nothing yet - not even the frame. The backend
		// hasn't reported login state, so anything we reveal now is a guess we
		// have to correct a few seconds later. Focusing an empty frame here
		// made NVDA announce "whatSnap panel" and then, seconds later, "search
		// conversations" once the chat list appeared. Instead we wait: the
		// "connected" handler shows the fully-populated chat list and focuses
		// the search field in one step (so NVDA says "whatSnap, search
		// conversations"), and "needs_login" shows the login panel - each
		// revealing its window only once, already correct. The frame is hidden,
		// not deleted, so the event loop stays alive to receive those events.
	});
	if let Err(e) = res {
		let _ = std::fs::write(platform::paths::log_dir().join("main_error.log"), format!("{:?}", e));
	}

	// Stops the backend if Exit hasn't already, then ends the process without
	// waiting on anything else that might hang on the way out.
	platform::quit::finish();
}

/// The status bar text while online and nothing more detailed is showing:
/// "Connected", plus the sync still going on the way WA Web reports it.
/// A fresh AppState: at startup, and on switching accounts, where starting
/// from scratch is what makes sure nothing of the previous account is left.
pub(crate) fn new_app_state(
	settings: crate::settings::Settings,
	ws_tx: mpsc::Sender<String>,
	ui_tx: UiCommandSender,
	audio_out: audio::OutputSlot,
	call_audio_out: audio::OutputSlot,
	accounts: Rc<RefCell<ui_commands::accounts::Accounts>>,
) -> AppState {
	let audio_speed = settings.audio_speed();
	AppState {
		accounts,
		settings,
		keymap: Rc::default(),
		ws_tx,
		ui_tx,
		audio_out,
		call_audio_out,
		audio_sink: None,
		playing_msg_id: None,
		voice_autoplay_id: None,
		is_audio_paused: false,
		audio_path: None,
		audio_total_secs: 0.0,
		audio_elapsed_secs: 0.0,
		audio_speed,
		audio_start_instant: None,
		chats: Vec::new(),
		filtered_chats: Vec::new(),
		archived_jids: std::collections::HashSet::new(),
		locked_jids: std::collections::HashSet::new(),
		chats_unlocked: false,
		pending_locked_chat: None,
		community_jids: std::collections::HashSet::new(),
		chat_parents: std::collections::HashMap::new(),
		announcement_jids: std::collections::HashSet::new(),
		channel_jids: std::collections::HashSet::new(),
		channel_roles: std::collections::HashMap::new(),
		channel_pinning: false,
		channel_directory: None,
		emoji_panel: None,
		pinned_jids: std::collections::HashSet::new(),
		muted_jids: std::collections::HashSet::new(),
		group_quiet_until: std::collections::HashMap::new(),
		blocked_jids: std::collections::HashSet::new(),
		blocked_entries: Vec::new(),
		blocked_contacts: None,
		scheduled_messages_enabled: true,
		pending_schedule: None,
		scheduled_list: None,
		unread_counts: std::collections::HashMap::new(),
		unread_divider_jid: None,
		unread_divider_count: 0,
		window_focused: true,
		presence: PresenceModel::default(),
		contact_online: std::collections::HashMap::new(),
		presence_sent: None,
		composing: None,
		background_unread_message_ids: Vec::new(),
		focus_read_at: None,
		notification_reads: std::collections::HashSet::new(),
		display_names: std::collections::HashMap::new(),
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
		joinable_calls: Vec::new(),
		call_answer_deadline: None,
		call_ringing_sink: None,
		call_ringback_on: false,
		call_reconnect_sink: None,
		call_hotkeys: None,
		incoming_call_caller: None,
		call_start_muted: false,
		call_connected_segment_start: None,
		call_connected_duration: std::time::Duration::ZERO,
		call_is_reconnecting: false,
		call_is_incoming: false,
		call_peer_name: String::new(),
		call_peer_is_saved: false,
		call_last_target: None,
		call_self_phone: String::new(),
		call_participants: Vec::new(),
		call_video: CallVideo::default(),
		group_data_members: Vec::new(),
		group_data_member_jids: Vec::new(),
		group_data_member_info: Vec::new(),
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
		conversation_data_contact: None,
		info_chat: None,
		overview_probe: false,
		own_push_name: String::new(),
		own_about: String::new(),
		own_about_duration: -1,
		own_about_loaded: false,
		default_timer: None,
		chat_filter: 0,
		favorite_jids: Vec::new(),
		chat_lists: Vec::new(),
		list_manager: None,
		common_group_jids: Vec::new(),
		privacy_settings: std::collections::HashMap::new(),
		privacy_settings_loaded: false,
		app_prefs: None,
		privacy_lists: std::collections::HashMap::new(),
		privacy_list_edits: std::collections::HashMap::new(),
		group_state: crate::ui::dialogs::new_group::NewGroupState {
			all_contact_labels: Vec::new(),
			all_contact_jids: Vec::new(),
			checked_jids: std::collections::HashSet::new(),
			current_jids: Vec::new(),
		},
		add_members_state: crate::ui::dialogs::add_members::AddMembersState {
			group_jid: None,
			all_contact_labels: Vec::new(),
			all_contact_jids: Vec::new(),
			checked_jids: std::collections::HashSet::new(),
			current_jids: Vec::new(),
		},
		attach_contact_state: crate::ui::dialogs::attach_contact::AttachContactState {
			all_contact_labels: Vec::new(),
			all_contact_jids: Vec::new(),
			checked_jids: std::collections::HashSet::new(),
			current_jids: Vec::new(),
		},
		selected_chat: None,
		admin_group_jids: std::collections::HashSet::new(),
		admins_only_jids: std::collections::HashSet::new(),
		not_member_jids: std::collections::HashSet::new(),
		business_jids: std::collections::HashSet::new(),
		initial_presence_received: false,
		messages: Vec::new(),
		messages_at_latest: true,
		loading_messages: false,
		no_more_messages: false,
		older_unloaded: 0,
		newer_unloaded: 0,
		land_on_first: false,
		history_loading: false,
		newer_loading: false,
		phone_history_pending_jid: None,
		phone_history_deadline: None,
		history_sync_last_update: None,
		history_sync_armed_until: None,
		db_size_last_seen: 0,
		db_growth_last_seen: None,
		db_growth_status_shown: false,
		recent_sync_active: false,
		recent_sync_percent: -1,
		recent_sync_paused: false,
		offline_sync_percent: None,
		history_start_chats: std::collections::HashSet::new(),
		empty_chat_history_asked: std::collections::HashSet::new(),
		quiet_reload_jid: None,
		prev_msg_count: 0,
		active_tab: 0,
		starred_entries: Vec::new(),
		filtered_starred_message_ids: Vec::new(),
		status_groups: Vec::new(),
		filtered_status_message_ids: Vec::new(),
		filtered_call_ids: Vec::new(),
		call_logs: Vec::new(),
		call_frequent: Vec::new(),
		call_contacts: Vec::new(),
		search_query: String::new(),
		filtered_message_ids: Vec::new(),
		message_search: MessageSearch::default(),
		chat_search_keys: std::collections::HashMap::new(),
		chat_preview_lines: std::collections::HashMap::new(),
		displayed_chat_labels: Vec::new(),
		chat_row_art: std::collections::HashMap::new(),
		main_ui_shown: false,
		pending_update_check: false,
		pending_whatsapp_link: None,
		pending_notification_replies: Vec::new(),
		pending_shared_files: Vec::new(),
		conn_online: false,
		conn_status_text: "Status: Connecting...".to_string(),
		conn_outage_announced: false,
		conn_manual_reconnect: false,
		pending_attachments: Vec::new(),
		pending_attachment_kind: whatsnap::AttachKind::default(),
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
		uploads_in_progress: std::collections::HashMap::new(),
		pending_copy_announcements: std::collections::HashSet::new(),
		search_target_id: None,
		search_target_text: None,
		catch_up_alerts: whatsnap::CatchUpAlerts::default(),
		catch_up_last_at: None,
		is_voice_recording: false,
		is_voice_paused: false,
		voice_recording_path: None,
		voice_recording_session: None,
		voice_recording_segments: Vec::new(),
		voice_recording_start: None,
		voice_paused_duration: std::time::Duration::ZERO,
		voice_pause_instant: None,
		voice_preview_sink: None,
		voice_preview_merged_path: None,
		is_voice_preview_playing: false,
		voice_recording_generation: 0,
		voice_recording_opening: false,
		voice_recording_pending: Arc::new(Mutex::new(None)),
		is_pairing_active: false,
		display_items: Vec::new(),
		selection_mode: false,
		selected_message_ids: std::collections::HashSet::new(),
		selection_rows: Vec::new(),
		bulk_delete_pending: std::collections::HashSet::new(),
		bulk_quiet_pending: std::collections::HashSet::new(),
	}
}

pub(crate) fn connected_status_text(s: &AppState) -> String {
	if let Some(pct) = s.offline_sync_percent {
		format!("Status: Connected (Downloading messages: {pct}%)")
	} else if s.recent_sync_paused {
		"Status: Connected (Syncing paused. Open WhatsApp on your phone to continue syncing.)".to_string()
	} else if s.recent_sync_active && s.recent_sync_percent >= 0 {
		format!("Status: Connected (Syncing older messages, {}% complete)", s.recent_sync_percent)
	} else if s.recent_sync_active {
		"Status: Connected (Syncing older messages)".to_string()
	} else {
		"Status: Connected".to_string()
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
		UiCommand::NewContact { name, first_name, phone, sync } => {
			ui_commands::contacts_groups::new_contact(state, parts, name, first_name, phone, sync)
		}
		UiCommand::PrepareAttachContact => ui_commands::attachments::prepare_attach_contact(state, parts),
		UiCommand::SendContacts(jids) => ui_commands::attachments::send_contacts(state, parts, jids),
		UiCommand::RequestQr => ui_commands::misc::request_qr(state, parts),
		UiCommand::CancelQr => ui_commands::misc::cancel_qr(state, parts),
		UiCommand::CancelPairing => ui_commands::misc::cancel_pairing(state, parts),
		UiCommand::RequestPairing(phone) => ui_commands::misc::request_pairing(state, parts, phone),
		UiCommand::SendMessage(text) => ui_commands::messaging::send_message(state, parts, text),
		UiCommand::ConversationSelectionChanged(idx) => {
			ui_commands::chat_list::conversation_selection_changed(idx, state, parts);
		}
		UiCommand::ProcessWebSocketMessage(raw) => process_websocket_message(raw, state, parts),
		UiCommand::BackendEvent { account, payload } => {
			ui_commands::accounts::backend_event(account, payload, state, parts)
		}
		UiCommand::OpenAccounts(focus) => ui_commands::accounts::open_accounts(state, parts, focus),
		UiCommand::LeaveLogin => ui_commands::accounts::leave_login(state, parts),
		UiCommand::AccountStopped(id) => ui_commands::accounts::account_stopped(state, parts, &id),
		UiCommand::FinishAccountRemoval(id) => ui_commands::accounts::finish_removal(state, &id),
		UiCommand::ExitApp => ui_commands::misc::exit_app(state, parts),
		UiCommand::NavSelected(idx) => ui_commands::chat_list::nav_selected(state, parts, idx),
		UiCommand::FilterConversations(query) => ui_commands::chat_list::filter_conversations(state, parts, query),
		UiCommand::SearchAllMessages => ui_commands::chat_list::search_all_messages(state),
		UiCommand::MoreMessageResults => ui_commands::chat_list::more_message_results(state),
		UiCommand::ShowChannelInfo(jid) => ui_commands::chat_list::show_channel_info(state, jid),
		UiCommand::OpenConversation(idx) => ui_commands::chat_list::open_conversation(state, parts, idx),
		UiCommand::OpenChatByPhone(digits) => ui_commands::chat_list::open_chat_by_phone(state, parts, digits),
		UiCommand::OpenChatByJid(jid) => ui_commands::chat_list::open_chat_by_jid(state, parts, jid),
		UiCommand::ToggleSelectionMode => ui_commands::chat_list::toggle_selection_mode(state, parts),
		UiCommand::SelectionAction(action) => ui_commands::selection::run(state, parts, action),
		UiCommand::AskDeleteMessages(ids) => ui_commands::selection::delete_row(state, parts, ids),
		UiCommand::LoadMoreMessages => ui_commands::chat_list::load_more_messages(state, parts),
		UiCommand::LoadNewerMessages => ui_commands::chat_list::load_newer_messages(state),
		UiCommand::JumpToLatestMessage => ui_commands::chat_list::jump_to_latest_message(state, parts),
		UiCommand::JumpToOldestMessage => ui_commands::chat_list::jump_to_oldest_message(state),
		UiCommand::SearchInConversation(query) => ui_commands::chat_list::search_in_conversation(state, parts, query),
		UiCommand::ToggleSearchPanel => ui_commands::chat_list::toggle_search_panel(state, parts),
		UiCommand::PromptLockedChats => ui_commands::chat_list::prompt_locked_chats(state, parts),
		UiCommand::ChatLockHello { outcome } => ui_commands::chat_list::finish_chat_lock_hello(state, parts, outcome),
		UiCommand::CloseLockedChats => ui_commands::chat_list::close_locked_chats(state, parts),
		UiCommand::CloseSearch => ui_commands::chat_list::close_search(state, parts),
		UiCommand::JumpToSearchResult { message_id, message_text } => {
			ui_commands::chat_list::jump_to_search_result(state, parts, message_id, message_text)
		}
		UiCommand::StartRecording => ui_commands::voice::start_recording(state, parts),
		UiCommand::StopRecording => ui_commands::voice::stop_recording(state, parts),
		UiCommand::TogglePauseRecording => ui_commands::voice::toggle_pause_recording(state, parts),
		UiCommand::ToggleVoicePreview => ui_commands::voice::toggle_voice_preview(state, parts),
		UiCommand::VoiceRecordingReady => ui_commands::voice::voice_recording_ready(state, parts),
		UiCommand::ForceSync => ui_commands::misc::force_sync(state, parts),
		UiCommand::SendVoiceMessage => ui_commands::voice::send_voice_message(state, parts),
		UiCommand::DiscardRecording | UiCommand::DiscardVoiceMessage => {
			ui_commands::voice::discard_recording(state, parts)
		}
		UiCommand::AttachFiles(kind) => ui_commands::attachments::attach_files(state, parts, kind),
		UiCommand::AddAttachment => ui_commands::attachments::add_attachment(state, parts),
		UiCommand::OpenPollDialog => ui_commands::attachments::open_poll_dialog(state, parts),
		UiCommand::OpenEmojiPanel { stickers } => ui_commands::stickers::open_emoji_panel(state, parts, stickers),
		UiCommand::ShowStatusMenu(idx) => ui_commands::status_extras::show_status_menu(state, parts, idx),
		UiCommand::ShowStarredMenu(idx) => ui_commands::starred::show_starred_menu(state, parts, idx),
		UiCommand::UnstarStarredMessage(idx) => ui_commands::starred::unstar_message_at(state, parts, idx),
		UiCommand::CopyStarredMessage(idx) => ui_commands::starred::copy_message_at(state, parts, idx),
		UiCommand::ShowCallsMenu(idx) => ui_commands::calls::show_calls_menu(state, parts, idx),
		UiCommand::DeleteCall(idx) => ui_commands::calls::delete_call_at(state, parts, idx),
		UiCommand::SetChatFilter(idx) => ui_commands::chat_lists::set_filter(state, parts, idx),
		UiCommand::SetFavorite { jid, favorite } => ui_commands::chat_lists::set_favorite(state, parts, jid, favorite),
		UiCommand::ChooseChatLists(jid) => ui_commands::chat_lists::choose_lists(state, parts, jid),
		UiCommand::ManageChatLists => ui_commands::chat_lists::manage_lists(state, parts),
		UiCommand::OpenCommonGroup(idx) => ui_commands::chat_lists::open_common_group(state, parts, idx),
		UiCommand::CreateCallLink => ui_commands::call_extras::create_call_link(state, parts),
		UiCommand::AskCallLink => ui_commands::call_extras::ask_call_link(state, parts),
		UiCommand::ToggleCallHand => ui_commands::call_extras::toggle_hand(state, parts),
		UiCommand::CallReact => ui_commands::call_extras::react(state, parts),
		UiCommand::AdmitCallPerson(admit) => ui_commands::call_extras::admit_selected(state, parts, admit),
		UiCommand::MuteCallPerson => ui_commands::call_extras::act_on_person(state, parts, false),
		UiCommand::RemoveCallPerson => ui_commands::call_extras::act_on_person(state, parts, true),
		UiCommand::CallPersonSelected => ui_commands::call_extras::person_buttons(&state.borrow(), parts),
		UiCommand::SetCallApproval(on) => ui_commands::call_extras::set_approval(state, parts, on),
		UiCommand::DismissIncomingCall => ui_commands::calls::dismiss_incoming_call(state, parts),
		UiCommand::RestoreIncomingCall => ui_commands::calls::restore_incoming_call(state, parts),
		UiCommand::RedialCall => ui_commands::calls::redial_call(state, parts),
		UiCommand::CloseCallWindow => ui_commands::calls::close_call_window(parts),
		UiCommand::OpenEventDialog => ui_commands::polls::open_event_dialog(state, parts),
		UiCommand::OpenScheduleDialog => ui_commands::scheduled::open_schedule_dialog(state, parts),
		UiCommand::ShowScheduledMessages => ui_commands::scheduled::show_scheduled_messages(state, parts),
		UiCommand::OpenPollVote(id) => ui_commands::polls::open_poll_vote(state, parts, id),
		UiCommand::OpenMessageOptions(id) => ui_commands::business::open_message_options(state, parts, id),
		UiCommand::OpenOrderDetails(id) => ui_commands::business::open_order_details(state, id),
		UiCommand::OpenCatalog(message_id) => ui_commands::business::open_catalog(state, parts, message_id),
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
		UiCommand::OpenChatAtMessage { jid, message_id } => {
			state.borrow_mut().search_target_id = Some(message_id);
			ui::chat_ui::open_chat_for_jid(&jid, state, parts);
		}
		UiCommand::ShowContactInfo(jid) => ui_commands::chat_list::show_contact_info(state, parts, jid),
		UiCommand::AddOrEditContact => ui_commands::contacts_groups::add_or_edit_contact(state, parts),
		UiCommand::OpenNewContactForm => ui_commands::contacts_groups::open_new_contact_form(parts),
		UiCommand::DeleteContact => ui_commands::contacts_groups::delete_contact(state, parts),
		UiCommand::ShowChatPrivacy => ui_commands::message_extras::show_chat_privacy(state),
		UiCommand::RemoveQuote => ui_commands::messaging::remove_quote(state, parts),
		UiCommand::SendTyping { jid, state: typing_state, media } => {
			ui_commands::messaging::send_typing(state, parts, jid, typing_state, media)
		}
		UiCommand::ForwardMessage { message_id, to_jids } => {
			ui_commands::messaging::forward_message(state, parts, message_id, to_jids)
		}
		UiCommand::PlaceCall => ui_commands::calls::place_call(state, parts),
		UiCommand::PlaceVideoCall => ui_commands::calls::place_video_call(state, parts),
		UiCommand::CallContact { jid, video } => ui_commands::calls::call_contact(state, parts, jid, video),
		UiCommand::JoinGroupCall(call_id) => ui_commands::calls::join_group_call(state, parts, call_id),
		UiCommand::AnswerCall => ui_commands::calls::answer_call(state, parts),
		UiCommand::RejectCall => ui_commands::calls::reject_call(state, parts),
		UiCommand::CallHotkey(hotkey) => ui_commands::calls::call_hotkey(state, parts, hotkey),
		UiCommand::LeaveCall => ui_commands::calls::leave_call(state, parts),
		UiCommand::ToggleCamera => ui_commands::calls::toggle_camera(state, parts),
		UiCommand::ToggleCallDeviceSettings => ui_commands::calls::toggle_call_device_settings(state, parts),
		UiCommand::CameraFailed(reason) => ui_commands::calls::camera_failed(state, parts, reason),
		UiCommand::ToggleScreenShare => ui_commands::calls::toggle_screen_share(state, parts),
		UiCommand::ScreenShareFailed(reason) => ui_commands::calls::screen_share_failed(state, parts, reason),
		UiCommand::ScreenShareBlack(black) => ui_commands::calls::screen_share_black(state, parts, black),
		UiCommand::SystemAudioFailed(reason) => ui_commands::calls::system_audio_failed(state, parts, reason),
		UiCommand::ShowMemberChanges => ui_commands::member_extras::request_member_changes(state),
		UiCommand::FilterGroupMembers(query) => ui_commands::contacts_groups::filter_group_members(state, parts, query),
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
		UiCommand::NotificationReply { jid, account, text } => {
			ui_commands::links::notification_reply(state, parts, jid, account, text)
		}
		UiCommand::ShareFiles(paths) => ui_commands::links::share_files(state, parts, paths),
		UiCommand::ShareMenuResult { ok, text } => {
			let parent: &dyn wxdragon::prelude::WxWidget =
				if parts.settings_dialog.dialog.is_shown() { &parts.settings_dialog.dialog } else { &parts.frame };
			let icon = if ok { MessageDialogStyle::IconInformation } else { MessageDialogStyle::IconWarning };
			MessageDialog::builder(parent, &text, "whatSnap")
				.with_style(MessageDialogStyle::OK | icon)
				.build()
				.show_modal();
		}
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
		UiCommand::PostCustomStatus { text, background, text_color, font } => {
			ui_commands::messaging::post_custom_status(state, parts, text, background, text_color, font);
		}
		UiCommand::PostMediaStatus => ui_commands::messaging::post_media_status(state, parts),
		UiCommand::OpenStatusPrivacy => {
			let payload = serde_json::json!({ "action": "get_status_privacy", "data": {} });
			let _ = state.borrow().ws_tx.send(payload.to_string());
		}
		UiCommand::OpenStatusComposer => {
			ui_commands::messaging::open_status_composer(state, parts);
		}
		UiCommand::GetMessageInfo { message_id } => ui_commands::messaging::get_message_info(state, parts, message_id),
		UiCommand::RetryMessage { message_id } => ui_commands::messaging::retry_message(state, parts, message_id),
		UiCommand::RetrySelectedMessage => ui_commands::messaging::retry_selected_message(state, parts),
		UiCommand::CancelSelectedUpload => ui_commands::messaging::cancel_selected_upload(state, parts),
		UiCommand::RequestMessageResend { message_id } => {
			ui_commands::messaging::request_message_resend(state, parts, message_id)
		}
		UiCommand::CopyAlbumToClipboard { message_ids } => {
			ui_commands::attachments::copy_album_to_clipboard(state, parts, message_ids)
		}
		UiCommand::SeekAudio(delta) => ui_commands::voice::seek_audio(state, parts, delta),
		UiCommand::StopVoiceMessage => {
			if ui_commands::voice::stop_voice_message(&mut state.borrow_mut(), parts) {
				crate::platform::announce::announce(parts.conversations_parts.messages_announcer, "Stopped");
			}
		}
		UiCommand::RefreshMessages => {}
		UiCommand::DownloadAttachment(_) => {}
		UiCommand::ChangeAudioSpeed => {}
		UiCommand::UpdateGroupSettings { name, topic, announce, locked } => {
			ui_commands::contacts_groups::update_group_settings(state, parts, name, topic, announce, locked)
		}
		UiCommand::GroupMemberAction { jid, action } => {
			ui_commands::contacts_groups::group_member_action(state, parts, jid, action)
		}
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
		UiCommand::IncreaseAudioSpeed => ui_commands::voice::increase_audio_speed(state, parts),
		UiCommand::DecreaseAudioSpeed => ui_commands::voice::decrease_audio_speed(state, parts),
		UiCommand::ToggleLockChat { jid, lock } => ui_commands::chat_list::toggle_lock_chat(state, parts, jid, lock),
		UiCommand::ChooseDisappearingTimer { jid } => {
			ui_commands::chat_list::choose_disappearing_timer(state, parts, jid)
		}
		UiCommand::ReportMessage { message_id } => ui_commands::report::report_message(state, parts, message_id),
		UiCommand::ReportToAdmins { message_id } => ui_commands::report::report_to_admins(state, parts, message_id),
		UiCommand::GoToDate => ui_commands::chat_list::go_to_date(state, parts),
		UiCommand::ReportChat { jid } => ui_commands::report::report_chat(state, parts, jid),
		UiCommand::ToggleBlockChat { jid } => ui_commands::blocking::toggle_block(state, parts, jid),
		UiCommand::ShowBlockedContacts => ui_commands::blocking::show_blocked_contacts(state, parts),
		UiCommand::UpdateGroupModeration {
			no_frequently_forwarded,
			allow_admin_reports,
			group_history,
			admins_add_only,
			approve_members,
		} => ui_commands::report::update_group_moderation(
			state,
			no_frequently_forwarded,
			allow_admin_reports,
			group_history,
			admins_add_only,
			approve_members,
		),
		UiCommand::GroupAction(action) => ui_commands::group_extras::group_action(state, parts, action),
		UiCommand::NewCommunity => ui_commands::group_extras::new_community(state, parts),
		UiCommand::ShowReportedMessages => ui_commands::report::request_reported_messages(state),
		UiCommand::SetMemberLabel(label) => ui_commands::contacts_groups::set_member_label(state, parts, label),
		UiCommand::TogglePinChat { jid, pin } => ui_commands::chat_list::toggle_pin_chat(state, parts, jid, pin),
		UiCommand::ToggleArchiveChat { jid, archive } => {
			ui_commands::chat_list::toggle_archive_chat(state, parts, jid, archive)
		}
		UiCommand::SetArchivedAlert { jid, level } => {
			ui_commands::chat_list::set_archived_alert(state, parts, jid, level)
		}
		UiCommand::SetChatNotify { jid, notify } => ui_commands::chat_list::set_chat_notify(state, parts, jid, notify),
		UiCommand::ShowChatNotify { jid } => ui_commands::chat_list::show_chat_notify(state, parts, jid),
		UiCommand::ToggleMuteChat { jid, mute, seconds } => {
			ui_commands::chat_list::toggle_mute_chat(state, parts, jid, mute, seconds)
		}
		UiCommand::ChooseMuteDuration { jid } => ui_commands::chat_list::choose_mute_duration(state, parts, jid),
		UiCommand::ToggleReadChat { jid, read } => ui_commands::chat_list::toggle_read_chat(state, parts, jid, read),
		UiCommand::DeleteChat { jid } => ui_commands::chat_list::delete_chat(state, parts, jid),
		UiCommand::DeleteChatAndLeaveGroup { jid } => {
			ui_commands::chat_list::delete_chat_and_leave_group(state, parts, jid)
		}
		UiCommand::ClearChat { jid } => ui_commands::chat_list::clear_chat(state, parts, jid),
		UiCommand::LeaveGroup { jid } => ui_commands::contacts_groups::leave_group(state, parts, jid),
	}
}
