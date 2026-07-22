# Project: WinZapp Rust Migration

## Architecture
- **Backend (Go)**: Handles WhatsApp protocol via whatsmeow, exposes WebSocket on :9000.
- **Frontend (Rust)**: UI in wxdragon, communicates with Go via WebSocket.
- **Data Flow**: Frontend `src/ui_wake.rs` and `src/main.rs` handle WS commands to/from Go backend. UI built in `src/ui/`.

## Code Layout
- `backend/main.go` -> Go backend
- `src/main.rs` -> Rust frontend entry point
- `src/ui/window.rs` -> Main window setup and event bindings
- `src/ui/*.rs` -> UI components
- `src/ui_wake.rs` -> UI commands and websocket message handling

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | True Recent Chat List | Implement Recent Chat List in Rust (sorting by recent message, exact match of Python client) | none | DONE |
| 2 | Message History | Implement actual message history fetching in Go (`get_messages` stub) and Rust UI update | none | DONE |
| 3 | Typing Indicators & Presence | Wire typing indicators/presence from Go backend to Rust UI (live regions for screen reader) | none | PLANNED |
| 4 | Complete UI Wiring & Archived Chats | Wire missing UI elements, implement Archived Chats, resolve dead code warnings | M1, M2 | PLANNED |

## Interface Contracts
### Rust ↔ Go (WebSocket)
- commands: `get_chats`, `get_messages`, `search_messages`, `send`, `new_contact`, `create_group`
- events: `chats`, `conversation_messages`, `typing`, `message`
