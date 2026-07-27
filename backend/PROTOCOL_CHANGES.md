# WhatsApp backend — protocol reference (post-enhancement)

This documents every WebSocket command (frontend → backend) and event
(backend → frontend) after this enhancement pass. Everything not listed
here (`get_qr`, `cancel_qr`/`cancel_pairing`, `pair`, `send`, `send_typing`,
`subscribe_presence`, `mark_read`, `new_contact`, `create_group`, `get_chats`,
`get_messages`, `update_settings`) kept its original request/response shape —
`send` and `get_messages` gained new *optional* fields, detailed below, but
old callers that don't send them keep working unchanged.

Media is delivered as **local file paths** (matching how `download_media`
already worked), cached under `<folder containing your .db file>/media_cache/`
(`media/`, `avatars/`, `outgoing/` subfolders). Nothing is served over HTTP.

## Commands that gained new optional fields

- **`send`**: `reply_to_id`, `reply_to_sender` (both optional) — quote another message.
- **`get_messages`**: response `messages[]` items now also include `status`,
  `mediaType`, `mediaMimetype`, `mediaFilename`, `mediaPath`, `quotedId`,
  `quotedText`, `quotedSender`, `isEdited`, `isDeleted`, `reactions[]`.

## New commands

### Contacts
| Action | Payload | Response event |
|---|---|---|
| `get_contacts` | — | `contacts` — full local contact list (not just chats with history) |
| `get_avatar` | `{jid}` | `avatar` — `{jid, path, pictureId}`, downloads+caches on first call |
| `check_contacts` | `{phones: []}` | `contacts_checked` — bulk "is this on WhatsApp" for a device-contacts-sync flow |
| `get_user_info` | `{jid}` | `user_info` — About text, picture id, device count |
| `get_business_profile` | `{jid}` | `business_profile` — address/email/hours |
| `get_blocklist` | — | `blocklist` — `{jids: []}` |
| `block_contact` / `unblock_contact` | `{jid}` | `blocklist_updated` |
| `set_about` | `{text}` | — (updates your own About text) |

### Media / voice messages
| Action | Payload | Response event |
|---|---|---|
| `send_attachment` | `{jid, type: image\|video\|document\|sticker, path or data(base64), mimetype?, filename?, caption?, reply_to_id?, reply_to_sender?}` | `message` (same shape as text sends) |
| `send_voice_message` | `{jid, path or data(base64), mimetype?, duration}` | `message` |
| `download_media` | `{jid, message_id}` | `media_downloaded` — `{path, mediaType, mimetype, filename}` |
| `open_media` | `{jid, message_id}` | opens in the OS default viewer (downloads first if needed) |
| `save_media_as` | `{jid, message_id, dest_path}` | `media_saved` |
| `play_voice_message` | `{id}` | `play_audio` — `{path}` (also sends the "played" receipt to the sender) |

`start_recording` / `toggle_pause_recording` / `discard_voice_message` are
now accepted no-ops — recording is a frontend/mic concern; only the finished
clip needs to reach the backend, via `send_voice_message`.

### Message actions
| Action | Payload | Response event |
|---|---|---|
| `react_message` | `{jid, message_id, sender_jid, emoji}` (empty emoji removes) | `message_reaction` |
| `edit_message` | `{jid, message_id, new_text}` | `message_edited` |
| `delete_message` | `{jid, message_id, sender_jid, for_everyone}` | `message_deleted` |
| `search_messages` | `{query, jid?}` | `search_results` — real search now (was a stub) |

Incoming edits/deletes/reactions from the other side also arrive live as
`message_edited` / `message_deleted` / `message_reaction`, instead of
showing up as generic new messages.

### Groups
| Action | Payload | Response event |
|---|---|---|
| `get_group_info` | `{jid}` | `group_info` |
| `leave_group` | `{jid}` | — |
| `get_group_invite_link` | `{jid, reset}` | `group_invite_link` |
| `update_group_participants` | `{jid, participants: [], action: add\|remove\|promote\|demote}` | `group_updated` |
| `set_group_name` | `{jid, name}` | `group_updated` |
| `set_group_topic` | `{jid, topic}` | `group_updated` |
| `set_group_photo` | `{jid, path or data(base64)}` | `avatar_updated` |
| `set_group_announce` | `{jid, value}` (true = only admins can post) | `group_updated` |
| `set_group_locked` | `{jid, value}` (true = only admins can edit info) | `group_updated` |

### Chat-level state
These now write through to WhatsApp's app-state (so pin/archive/mute match
what you'd see on your phone or any other linked device), not just a local
flag.

| Action | Payload |
|---|---|
| `set_chat_pinned` | `{jid, value}` |
| `set_chat_archived` | `{jid, value}` |
| `set_chat_muted` | `{jid, muted, duration_seconds}` (0 = forever) |
| `set_chat_read` | `{jid, value}` (marks read/unread, syncs to other devices) |
| `delete_chat` | `{jid}` (local-only delete) |

## New live events (no command needed — pushed as they happen)

- `message_status` — `{jid, message_ids: [], status}` — sent/delivered/read/played ticks for messages *you* sent
- `avatar_updated` — `{jid, removed}`
- `about_updated` — `{jid, status}`
- `group_updated` — `{jid}` — group metadata or membership changed; call `get_group_info` for details
- `blocklist_updated`

## Data model notes

- `messages` table gained: `status`, `media_type`, `media_mimetype`,
  `media_filename`, `media_path`, `quoted_id`, `quoted_text`,
  `quoted_sender`, `is_edited`, `is_deleted`.
- New `reactions` table (one active reaction per reactor per message).
- New `contacts_cache` table backing `get_contacts`/`check_contacts`.
- All migrations run automatically on startup (same `ALTER TABLE`-and-ignore
  pattern as before) — existing `session.db` files upgrade in place, no
  manual steps.

## Two pre-existing quirks fixed along the way

- `force_sync` and `new_contact` used to push the raw contact-store map
  mislabeled as a `"chats"` event (wrong shape). Both now correctly call
  `broadcastChats()`; use the new `get_contacts` command for the actual
  contact list.
- `play_voice_message`'s download logic is now shared with
  `download_media`/`open_media`, so a voice note only gets downloaded once
  and reused from cache instead of writing a fresh temp file every time.
