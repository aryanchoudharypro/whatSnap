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
| 3 | Typing Indicators & Presence | Wire typing indicators/presence from Go backend to Rust UI (live regions for screen reader) | none | PLANNED |
| 4 | Complete UI Wiring & Archived Chats | Wire missing UI elements, implement Archived Chats, resolve dead code warnings | M1, M2 | PLANNED |
| 5 | Distribution | Inno Setup installer, code signing, portable zip | none | IN PROGRESS |
| 6 | Auto-update | Deferred | M5 | PLANNED |
