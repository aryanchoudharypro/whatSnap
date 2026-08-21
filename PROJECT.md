# Project: whantv

An accessible native Windows WhatsApp client, built because the official
desktop app moved to Edge WebView2 and degraded badly with screen readers.

## Architecture
- **Backend (Go)**: Handles WhatsApp protocol via whatsmeow. Listens on a
  Windows named pipe, one JSON object per line.
- **Frontend (Rust)**: UI in wxdragon + AccessKit. Spawns the backend as a
  child process and connects to its pipe.
- **Data Flow**: `src/main.rs` owns the pipe connection and reconnect loop;
  `src/ui_wake.rs` marshals messages onto the UI thread. UI built in `src/ui/`.

The two processes are separate programs on purpose — it keeps a license
boundary between the MIT frontend and the GPLv3 backend. See
THIRD-PARTY-NOTICES.md.

## Code Layout
- `backend/main.go` -> Go backend entry point, pipe listener
- `backend/ws_dispatch.go` -> command dispatch
- `backend/paths.go` -> writable-state locations
- `src/main.rs` -> Rust frontend entry point, backend supervision
- `src/paths.rs` -> writable-state locations (mirrors backend/paths.go)
- `src/win_job.rs` -> job object tying backend lifetime to the frontend
- `src/ui/window.rs` -> Main window setup and event bindings
- `src/ui/*.rs` -> UI components
- `src/ui_wake.rs` -> UI wakeup and command channel

## Runtime State
Everything writable lives under `%LOCALAPPDATA%\whaNTV\`:
- `session.db` (+ WAL) -> whatsmeow session and message history
- `media_cache/` -> downloaded media and avatars
- `logs/` -> `backend.log`, `panic.log`, `main_error.log`

Nothing is written next to the executable, so the app can be installed to a
directory it does not own.

## IPC Contract (Rust <-> Go)
Newline-delimited JSON over `\\.\pipe\whantv-<uuid>`. The pipe name is random
per launch, its DACL is restricted to the current user, and the frontend
verifies the server PID matches the backend it spawned before sending
anything.

- commands: `get_chats`, `get_messages`, `search_messages`, `send`,
  `send_attachment`, `send_voice_message`, `new_contact`, `create_group`, ...
  (full list in `backend/ws_dispatch.go`)
- events: `chats`, `conversation_messages`, `typing`, `message`, `connected`

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | True Recent Chat List | Implement Recent Chat List in Rust (sorting by recent message, exact match of Python client) | none | DONE |
| 2 | Message History | Implement actual message history fetching in Go (`get_messages` stub) and Rust UI update | none | DONE |
| 3 | Typing Indicators & Presence | Wire typing indicators/presence from Go backend to Rust UI (live regions for screen reader) | none | DONE |
| 4 | Complete UI Wiring & Archived Chats | Wire missing UI elements, implement Archived Chats, resolve dead code warnings | M1, M2 | DONE (Archived Chats tab is live; dead-code sweep not separately re-verified) |
| 5 | Distribution | Inno Setup installer, code signing, portable zip | none | IN PROGRESS (installer + portable zip build script done in `packaging/`; code signing needs a real cert, wired via `-SignCommand` but unexercised) |
| 6 | Auto-update | Deferred | M5 | PLANNED |
| 7 | Starred Messages | Star/unstar command (whatsmeow `appstate.BuildStar` patch), starred-messages view | none | DONE (star/unstar wired end-to-end via `appstate.BuildStar`, plus a "Starred Messages" nav entry — before Archived Chats — listing every starred message across chats and jumping to it in-context) |
| 8 | Send Disappearing/View-Once/Polls/Live Location/Stickers | Add send paths for message types already unwrapped on receive. Confirmed in whatsmeow: `SetDisappearingTimer`/`SetDefaultDisappearingTimer`, `BuildPollCreation`/`BuildPollVote`; sticker-send already exists in `cmd_media.go` for forwarding, needs a picker; view-once/live-location just need protobuf wrapping, no special API | none | PLANNED |
| 9 | Chat Export | Export a chat's messages/media to a file. Needs no whatsmeow API — `session.db`/`media_cache/` already hold everything locally | none | DONE (per-chat "Export chat..." in the chat-list context menu writes a .zip with `transcript.txt` + a `media/` folder; media not yet cached is downloaded on the fly, with one phone re-upload retry, and skipped if still unavailable. Not manually exercised end-to-end in the GUI — this dev environment has no interactive desktop session to drive it — but both binaries build clean and the media path reuses the same retry logic already proven in `handleDownloadMedia`) |
| 10 | Status/Stories | Post (confirmed: plain `SendMessage` to `types.StatusBroadcastJID`, resolved via `GetStatusPrivacy`/`getStatusBroadcastRecipients`) and view text/photo/video status updates; viewing needs incoming status-notification handling, not yet wired | none | DONE for viewing + text posting (new "Status" nav entry, after Conversations: incoming status posts intercepted in `whatsapp_events.go` and stored in the same `messages` table under `jid='status@broadcast'` — reusing every existing message-ID-keyed media download/open path for free — with 24h expiry cleanup and `MarkRead`-backed viewed tracking. Posting is text-only (`ExtendedTextMessage` to `types.StatusBroadcastJID`); photo/video status **posting** is not implemented, viewing others' photo/video statuses is) |
| 11 | Communities & Channels | Confirmed in whatsmeow: communities via `CreateGroup{IsParent}`/`LinkedParentJID`/`GetSubGroups`; channels fully implemented in `newsletter.go` (`CreateNewsletter`, `FollowNewsletter`, etc.) — API surface is ready, just unused | none | PLANNED |
| 12 | Group Video Calls | Confirmed in whatsmeow fork: `call_group.go`/`call_group_invite.go`/`call_link.go` implement group-call signaling (`InviteCallParticipant`, `RingCallParticipant`, `CreateCallLink`/`JoinCallLink`); meowcaller has matching `engine_group_*.go` scaffolding. Video-path maturity in meowcaller unverified — check before committing to video, not just group audio | none | PLANNED |
| — | ~~Linked Devices Management~~ | Investigated and dropped: whatsmeow runs as a companion device and exposes no API to enumerate or revoke *sibling* linked devices — viewing/removing companions is a phone-primary-only privileged operation, not reachable from this client | none | NOT FEASIBLE |
