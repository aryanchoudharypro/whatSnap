# whatSnap User Manual

[whatSnap](https://github.com/aryanchoudharypro/whatSnap) is a native, keyboard-first WhatsApp client for Windows, made for screen reader users. It links to the WhatsApp account on your phone, as WhatsApp Web and WhatsApp for Windows do. It does not use a browser.

whatSnap is in development. Most features are tested with a real WhatsApp account, but some are not yet. See [Features Not Yet Tested](#features-not-yet-tested).

## Disclaimer
whatSnap is an unofficial client. It is not made, supported, or endorsed by WhatsApp or Meta, and it is not affiliated with them.

Using a third-party client may go against the WhatsApp Terms of Service. WhatsApp could limit, suspend, or ban an account that uses it, at any time and without warning. The whatSnap developers can't undo that.

You use whatSnap at your own risk. If you can't afford to lose your WhatsApp account, don't use whatSnap with it. whatSnap comes as is, with no warranty. Its authors and contributors are not responsible for anything that happens to your account, your data, or your device because you used it, including restrictions, bans, and lost messages.

## System Requirements
- Windows 10 or 11, on x64.
- WhatsApp on your phone. whatSnap is a linked device, so you need a phone with WhatsApp to link it the first time.

## Core Features
- A native user interface with controls that work well with screen readers. whatSnap announces changes as they occur, such as new messages, typing, and the connection status. The message list is a custom list control that uses [AccessKit](https://accesskit.dev). Your screen reader reads the full text of each message.
- Chats, groups, communities, channels, status updates, starred messages, archived chats, and the call history.
- Messages, with:
  - Replies, forwarding, editing, deleting, reactions, and stars.
  - Photos, videos, audio files, documents, contacts, stickers, and sending view once media. Like WhatsApp Web, view once messages you receive can only be opened on your phone.
  - Voice messages, which you can pause while you record. whatSnap can remove background noise from them.
  - Polls, quizzes, and events.
  - Mentions, link previews, and disappearing messages.
  - Replies from Meta AI. whatSnap shows them as plain text without markdown symbols, and announces a reply once Meta AI finishes writing it.
- Voice and video calls, in one to one chats and in groups:
  - Answer or reject a call from any program, with a global shortcut.
  - Screen sharing. You can also keep your camera on while you share your screen.
  - Echo cancellation and noise suppression. Sound from your speakers or leaking from your headphones, such as the other person's voice or your screen reader, is removed from what your microphone sends, along with background noise.
  - Raise your hand, send reactions, add people, and mute or remove people in group calls.
  - Call links. Create a link, or join a call from a link.
  - Join a group call that is already in progress.
  - whatSnap connects again by itself if your network drops during a call.
- Several WhatsApp accounts in one window. Every account stays connected and alerts you, and you switch between them with `Ctrl+Shift+A`.
- A status viewer and a video player in the app.
- Emoji and stickers, including the WhatsApp sticker store and your favorite stickers.
- Spell checking in the message box, with the Windows spell checker.
- Chat lists and filters, chat lock, mute, pin, archive, export, and report.
- Settings that sync with WhatsApp Web and WhatsApp for Windows.
- Keyboard shortcuts that you can change.
- A tray icon, Windows notifications, and a sound for each kind of event.
- `whatsapp://` links and `wa.me` pages open in whatSnap.
- Send files to whatSnap from File Explorer or from the Windows Share menu.

## Installing
Download whatSnap from the [releases page](https://github.com/aryanchoudharypro/whatSnap/releases). There are two kinds of release:

- Stable releases have a version number, such as 0.1.0.
- The Development Build has every change as soon as it is made. It can have more problems.

Each release has two files:

- `whatSnap_setup.exe` installs whatSnap. The installer can add a desktop icon, and it can add whatSnap to the Windows Share menu.
- `whatSnap.zip` is the portable version. Extract it to a folder, then run `whatSnap.exe`. Keep the `backend` folder next to `whatSnap.exe`.

## Updating
whatSnap checks for a new version when it starts. When there is one, it shows what changed and asks if you want to update. If you say yes, whatSnap downloads the update, closes, installs it, and starts again. You don't need to link your phone again.

To check yourself, use `Account -> Check for Updates...`. To select stable releases or development builds, or to turn off the check at startup, use the General tab of the settings.

whatSnap installs an update only when its signature is correct. A portable copy updates the files in its own folder. An installed copy runs the new installer.

## Linking Your Phone
The first time you start whatSnap, it asks you to link your phone. There are two ways:

- With a phone number. Select your country (it starts on the country Windows is set to), type your phone number, and press `Continue`. whatSnap shows a pairing code. On your phone, open WhatsApp, go to `Linked devices`, select `Link a device`, then select `Link with phone number instead` and type the code.
- With a QR code. Press `Connect with QR Code`. On your phone, open WhatsApp, go to `Linked devices`, select `Link a device`, and scan the code. The code changes by itself every so often. After a few minutes it stops, and a `Reload QR Code` button gets you a new one. `Escape` goes back to the phone number.

After you link, whatSnap gets your recent chats from your phone. This can take a few minutes.

While it does, the status bar says `Syncing older messages` and how much is done, like `45% complete`, the same as WhatsApp Web. Keep WhatsApp open on your phone until it finishes. If your phone stops sending for two minutes, the status bar says `Syncing paused. Open WhatsApp on your phone to continue syncing.` When whatSnap starts after being closed for a while, the status bar shows `Downloading messages` with how much of what you missed has come in.

To unlink this computer, use `Account -> Log Out`. Log Out has no shortcut, so you can't log out by mistake.

## Several Accounts
You can link more than one WhatsApp account, for example a personal and a work number. Use `Account -> Accounts...` (`Ctrl+Shift+A`). The `Linked accounts` list shows each account by its name and number, which one is on screen, and how many unread chats the others have.

- To show another account, select it and press `Enter`, or press `Switch`. The `Switch` button is only there when the selected account isn't the one on screen. The window empties and fills in again with that account's chats. Drafts you haven't sent stay with their account.
- To add an account, press `Add Account...`. whatSnap shows the login window, and you link the new phone the same way as the first one. If you close the login window instead, whatSnap goes back to the account you were using.
- To give an account your own name, like Work, press `Rename...`. Leave the name empty to use the account's WhatsApp name.
- To choose how an account alerts you while another one is on screen, press `Notifications...`. For messages, choose notification and sound, notification only, sound only, or off. For calls, choose to switch to the account and ring, a notification only, or off. Each chat's own notification settings still count, and the account on screen always uses the normal settings.
- To remove an account, press `Remove...`. whatSnap logs it out of WhatsApp, so it also leaves your phone's linked devices, and deletes its messages and media from this computer. Your phone keeps its chats. You can't remove the only account; use `Account -> Log Out` for that.

Every account stays connected while whatSnap runs, not just the one on screen. A message to another account plays its sound and shows a notification with the account's name in brackets after the chat name, using the same notification settings. Selecting the notification switches to that account and opens the chat. `Mark as read` on the notification marks the chat read without switching. When another account gets a call, whatSnap switches to it and rings, so you can answer as usual. During a call or while you record a voice message, whatSnap doesn't switch: a notification says who is calling which account.

With more than one account, the window title starts with the name of the account on screen. When the other accounts have unread chats, the window title and the tray icon also say how many, for example `2 unread chats in other accounts`. Muted and archived chats don't count, and neither do accounts whose message notifications are off.

## Main Window Layout
The main window has three parts:

- The Main Navigation list shows the sections of whatSnap: Chats, Status, Starred Messages, Archived Chats, Communities, Channels, and Calls.
- The list of the current section, for example your chats. Above it are the search field (`Search or start a new chat` in Chats), the `Filters` box, and the `New Chat` button.
- The open chat. It has the Messages list and the message box.

Use `Tab` and `Shift+Tab` to move between the controls. Use `Alt+1` through `Alt+7` to go to a section from anywhere. You can see these shortcuts in the Go menu.

To change the order of the sections, select a section in the Main Navigation list and press `Ctrl+Shift+Left Arrow` or `Ctrl+Shift+Right Arrow`. The Go menu follows the same order. The `Alt` shortcuts stay with their sections.

## Sections

| Section | Shortcut | What it shows |
|---|---|---|
| Chats | `Alt+1` | Your chats and groups. |
| Calls | `Alt+2` | Your recent calls. |
| Status | `Alt+3` | The status updates of your contacts, and your own. |
| Starred Messages | `Alt+4` | The messages that you starred, in all chats. |
| Archived Chats | `Alt+5` | The chats that you archived. |

An archived chat comes out of the archive when a new message comes in, unless `Keep chats archived` is on in the settings of WhatsApp on your phone. whatSnap follows the setting of your phone, like WhatsApp Web.
| Communities | `Alt+6` | Your communities and their groups. |
| Channels | `Alt+7` | The channels that you follow. |

## Chats

### The Chat List
Each chat in the list shows its name, the number of unread messages, and the last message. A group also says how many of its unread messages mention you, mention everyone or reply to you, for example "3 unread messages, 1 mention". When you leave a chat with text you haven't sent, the chat shows "Draft:" and that text in place of the last message, like in WhatsApp Web, until you send it, delete it, or new messages come in. Press `Enter` to open a chat. Press the Applications key or `Shift+F10` to open the actions menu of a chat. The menu has these items, when they apply to the chat:

- `Contact info`, or `Channel info` for a channel. `Channel info` shows the channel's details without opening it, so it isn't marked as read.
- `Pin chat` or `Unpin chat`. An archived chat has no `Pin chat`; pinning a chat takes it out of the archive, and archiving a chat unpins it, like in WhatsApp Web.
- `Archive chat` or `Unarchive chat`
- `Lock chat` or `Unlock chat`
- `Mark as read` or `Mark as unread`
- `Add to favorites` or `Remove from favorites`
- `Add to list...`
- `Export chat...` and `Export text only (Markdown)...`
- `Leave group...`
- `Clear chat...`
- `Report...`
- `Block contact...` or `Unblock contact`
- `Delete chat...`
- `Mute notifications` or `Unmute notifications`
- `Notification settings...`

### Search and Filters
Type in the `Search or start a new chat` field to find chats, contacts and messages. Press `Ctrl+/` to go to the field from anywhere. Like in WhatsApp Web:

- A chat is found by its name or group name, the person's WhatsApp name or business name, their phone number, or "Me" for your own chat. Capitals and accents don't matter, and when you type more than one word, each word must be in the same name, in any order. The text of the messages, the last message and drafts don't find a chat.
- With the `All chats` filter, the results have up to three parts, each with a heading: `Chats`, including archived chats, then `Contacts` who have no chat yet, then `Messages` that contain what you typed, newest first. Messages are searched once you stop typing, when you type at least two letters, or one emoji. When you reach the last message found, whatSnap loads more. Press `Enter` on a message to open its chat at that message.
- With another filter, only the chats of that filter are searched, and the field is named after it, for example `Search unread chats`.
- In the field, `Enter` opens the first result, and `Down Arrow` at the end of the text goes to the first result. `Up Arrow` on the first result goes back to the field. `Escape` clears the search, keeps the filter, and moves to the chat list.

When nothing is found, the list says "No chats, contacts or messages found". When a filter has no chats, the list says so, for example "No unread chats".

The `Filters` box shows only some chats: All chats, Unread, Favorites, Groups, and your own chat lists. To make, rename, or delete chat lists, use `Chats -> Manage Lists...`. Your chat lists sync with your phone.

### New Chats and Groups
- `Ctrl+N` starts a new chat. You can select a contact, or type a phone number.
- `Ctrl+Shift+N` makes a new group.
- `Chats -> New Community...` makes a new community.

### Locked Chats
A locked chat doesn't show in the chat list. When you have locked chats, a `Locked Chats` button shows below `New Chat`. Press it and type your secret code to show them.

### Chat Notifications
`Notification settings...` in the actions menu of a chat opens a dialog with these settings:

- `When a message comes in`: `As set in Settings`, `Notification and sound`, `Notification only, no sound`, `Sound only, no notification`, or `Off on this computer`. This only changes this computer, not your phone.
- `Notification tone`: the sound for this chat. `Default` uses the message or group tone from Settings. You can also pick `Alert 1` to `Alert 10`, `Custom sound file`, or `None` for no sound. Each tone plays when you pick it. The tone is shared with WhatsApp for Windows and WhatsApp Web on your other computers, the same way WhatsApp shares it. A custom sound file stays on this computer.
- `Ringtone`: what calls from this chat ring with. `Default` uses the call or group call ringtone from Settings. You can also pick `WhatsApp`, a Windows ringtone, `Custom sound file`, or `None` for a silent ring. This stays on this computer.
- After you pick `Custom sound file`, press Tab to reach the `Choose ... File` button and pick a sound file, such as an MP3 or WAV file. The button shows the file's name once you pick one.
- `Hide message content`: notifications from this chat don't show what the message says.

For an archived chat, `Archived alerts` sets how the chat tells you about new messages.

## The Open Chat
The open chat has these controls, in this order:

- The buttons at the top: `Conversation data`, `Search in conversation`, `Call`, and `Video call`. When a group call is in progress, a `Join call` button also shows.
- The Messages list.
- The buttons for the selected message, such as `Play voice message`, `Open`, `Save as`, and `Download`.
- The `Emoji and stickers` button.
- The message box, named `Type a message`.
- The `Add attachment`, `Send message`, and `Record voice message` buttons.

Press `Alt+M` to go to the Messages list. Press `Alt+E` to go to the message box. Press `Alt+E` again to go back to where you were. Press `Alt+T` to hear the status of the chat, for example online, or typing. Press `Alt+L` to turn reading new messages aloud on or off for the open chat. Press `Alt+I` to open the information of the chat. Press `Escape` to close the chat.

### The Messages List
The newest message is at the bottom. When you go up, whatSnap loads older messages. Date lines between the messages tell you when each day starts.

| Key | Action |
|---|---|
| `Up Arrow` and `Down Arrow` | Move one message. |
| `Page Up` and `Page Down` | Move one page. |
| `Home` | Go to the oldest message of the chat. |
| `End` | Go to the newest message. |
| `Enter` | Play a voice message. On a call, call back or join the call. |
| `Space` | Play or pause a voice message. |
| `Shift+Space` | Stop the voice message that plays or is paused. |
| `Shift+Enter` | Show the full text of the message in a dialog. |
| `Left Arrow` | On a reply, read the message that it replies to. |
| `Left Arrow` two times | On a reply, go to the message that it replies to. |
| `Right Arrow` | Go to the next message that replies to the selected message. After going to a replied-to message with `Left Arrow` two times, this takes you back to the reply. |
| `R` | Reply. |
| `E` | Edit your message. |
| `S` | Star or unstar. |
| `F` | Forward. |
| `V` | Vote in a poll, or answer an event. |
| `Delete` | Delete. |
| `Ctrl+C` | Copy the text or the file of the message. |
| `Ctrl+V` | Send the files that you copied. |
| `Ctrl+Space` | Turn selection mode on or off. |
| `Applications key` or `Shift+F10` | Open the actions menu of the message. |

If `Typing a letter in the message list starts a message` is on in the settings, typing a letter, a digit, or a punctuation mark in the list moves you to the message box with that character. The letter keys of the list then need `Alt`: `Alt+R` replies, `Alt+F` forwards, `Alt+S` stars, `Alt+V` votes, and `Alt+J` goes back 5 seconds. `Alt+E` and `Alt+L` stay `Jump to Message Field` and `Read New Messages Aloud`, so to edit or go forward 5 seconds from the list, give those actions other keys in `Keyboard Shortcuts...`.

While a voice message plays, `Left Arrow` and `J` go back 5 seconds, and `Right Arrow` and `L` go forward 5 seconds. When it is paused, these keys do nothing to it, so it stays paused. To change the speed, use the speed button. Its name is the current speed, for example `1.0x`.

To stop a voice message, press `Shift+Space`. The next time you play it, it starts from the beginning. When you close the chat or open another chat, the voice message stops too.

The actions menu of a message has these items, when they apply to the message: `Copy`, `Reply`, `Forward...`, `Edit`, `Delete`, `React...`, `Star` or `Unstar`, `Retry send`, `Request again`, `Save voice message...`, `Message Info`, `Copy all photos`, `Copy Number`, `Message Number`, and `Report...`. In a group, a submenu for the sender lets you message or call them. In a channel, a submenu lets you pin a post, or see who reacted or voted.

### Selection Mode
In selection mode, `Space` selects or clears a message. whatSnap says "Selection mode" when you enter it and "1 message selected", "2 messages selected" and so on each time you select or clear a message. A bar shows how many messages you selected, with `Delete` and `Cancel` buttons. To leave selection mode, press `Ctrl+Space` or `Escape`.

Photos and videos sent as an album show as one item, such as `Album: 4 photos`, with the reactions to all of its photos. Press Enter on it, or use `Open album`, to list the items as `Photo 2 of 4`. In that list, Up and Down move between items, Enter opens the item (or downloads it), `Save as...` saves it, and Escape returns to the message list.

### Writing Messages
Type your message in the message box. If `Enter is send` is on in the settings, `Enter` sends the message, and `Shift+Enter` starts a new line. If it is off, use the `Send message` button. whatSnap doesn't read your own messages back to you when you send them.

New messages in the open chat are read aloud. To stop this for all chats, turn off `Read new messages aloud in the open chat` on the Notifications tab of the settings. To change it for one chat, use the notification settings of that chat, or press `Alt+L` in it. Messages you send yourself are not read, unless you turn on `Also read your own sent messages aloud`; they are then read once the message is sent.

- To reply, press `R` on a message. The reply shows above the message box. To remove it, use `Remove quote`, or press `Escape`.
- To edit, press `E` on your message. To stop editing, press `Escape`, or use `Cancel edit`.
- To mention someone in a group, type `@` and the first letters of their name.
- `Alt+Shift+E` opens the emoji panel. `Alt+Shift+S` opens the stickers.

### Spell Checking
whatSnap uses the spell checker of Windows. Press `F7` to select the next misspelled word and hear its suggestions. Press the Applications key on a misspelled word to see its suggestions, `Add to Dictionary`, and `Ignore`. To hear when you type a misspelled word, turn on `Say when a word you type is misspelled` in the settings.

### Replace Text With Emoji
When this setting is on, whatSnap changes some text into emoji as you type, for example `:-)` or `(y)`. Press `Backspace` immediately after to change it back. For the full list, use `Replace Text With Emoji: See Help...` on the Chats tab of the settings.

### Sending Files
Press `Add attachment` to open a menu:

- `Document`: send any file as a document. Photos keep their full quality.
- `Photos & videos`
- `Audio`: send audio files that play in the chat.
- `Contact`
- `Poll`: make a poll or a quiz.
- `Event`
- `Sticker`

Before you send, you can add a caption and more files. To send a photo or video that can be opened only once, select `View once`.

You can also paste files with `Ctrl+V` in the message box or the Messages list.

### Voice Messages

| Key | Action |
|---|---|
| `Ctrl+Shift+R` | Start recording. |
| `Alt+P` | Pause or continue the recording. |
| `Alt+Shift+P` | While the recording is paused, play or pause what you have recorded so far. |
| `Alt+D` or `Escape` | Discard the recording. |
| `Ctrl+Enter` | Send the voice message. |

You can also use the `Record voice message`, `Pause recording`, `Play preview`, `Send voice message`, and `Discard voice message` buttons. If you open another chat while you record, the recording is discarded. Select `View once` before you send, to send it as a view once voice message.

### Other Chat Actions
The Conversation menu has more actions for the open chat. It shows only the ones that fit that chat. When the chat list has the focus, the shortcuts for `Mute Chat`, `Archive Chat`, `Pin Chat`, `Lock Chat` and `Mark as Unread` work on the chat you are on in the list, like in WhatsApp Web:

- `Search in Chat` (`Ctrl+Shift+F`). `Escape` closes the search and goes back to the Messages list.
- `Go to Date...`: asks for a date as year-month-day, like `2026-03-27`, and goes to the first message on or after that day. A date after the last message goes to the latest one.
- `Pinned Messages...`, not in broadcast lists.
- `Business Catalogue...`, only in a chat with a business.
- `Schedule Message...` (`Ctrl+Shift+T`) and `Scheduled Messages...` (`Ctrl+Shift+K`), in a chat with one person or a group. See Scheduled Messages below.
- `Mute Chat` (`Ctrl+Shift+M`)
- `Archive Chat` (`Ctrl+Shift+E`)
- `Pin Chat` (`Ctrl+Shift+P`)
- `Lock Chat` (`Ctrl+L`)
- `Mark as Unread` (`Ctrl+Shift+U`)
- `Disappearing Messages...`, not in channels.
- `Advanced Chat Privacy...`, not in channels or broadcast lists.
- `Report...`, not in channels or broadcast lists.
- `Block or Unblock Contact`, only in a chat with one person.
- `Star Message` (`Alt+8`)

### Scheduled Messages
Like in WhatsApp Web, you can write a message now and have WhatsApp send it later. WhatsApp's servers send it at the chosen time, even when whatSnap or your computer is off. This works in chats with one person and in groups.

- `Conversation -> Schedule Message...` (`Ctrl+Shift+T`) opens the `Schedule message` dialog. The `Message` field starts with what you typed in the message box. Pick the `Date`, `Hour` and `Minute`. The time it starts on is the one WhatsApp Web suggests. The time has to be at least 10 minutes from now and at most 14 days away, unless WhatsApp sets other limits for your account. Press `Schedule message`. The message box is cleared, like after sending, and the chat shows "You scheduled a message".
- `Conversation -> Scheduled Messages...` (`Ctrl+Shift+K`) lists the chat's scheduled messages, with when each one will be sent. Select one and use `Send now`, `Delete` (or the `Delete` key) to unschedule it, or `Copy text`. A message that failed to send says `Did not send`.
- A chat can have up to 30 scheduled messages waiting. In groups, people who join after you schedule a message will not get it.
- Messages that other people schedule show up in the chat when WhatsApp sends them.
- whatSnap schedules text messages only. If WhatsApp hasn't turned scheduling on for your account, whatSnap says so, and hides the menu items when WhatsApp says the feature is off.

### Overview Tab
Contact Info (`Alt+I` in a one-to-one chat) and Group Info both open on an `Overview` tab, the first tab, like the info page of WhatsApp Web. In a group it starts with the `Group name` and `Group description` fields and a `Save name and description` button. If only admins can edit the group info and you are not an admin, the two fields are read only and the button is hidden. Then comes a short summary of the chat and only the items that fit this chat:

- `Contact details...` (one person only): opens a window with their name, phone number, about text and the `Voice call`, `Video call`, `Edit contact` and `Delete contact` buttons.
- `Lock this chat`: a switch for locking the chat.
- `Restricted chat (changed on your phone)`: shown only when the chat is restricted. Like in WhatsApp Web it is always dimmed, because only your phone can change it.
- `Notification settings...`, `Disappearing messages...` (not in channels) and `Advanced chat privacy...` (not in channels or broadcast lists).
- `Starred messages`: goes to the Starred Messages list.
- `Export chat...`, `Clear chat...`, `Block contact...` or `Unblock contact` (one person only), `Report...` and `Delete chat...`.

Chat themes are not in whatSnap yet.

### Group Info
Press `Alt+I` in a group to open its information. The Members tab has a search box, the list of members, and the `Add member`, `Invite link...` and `Join requests...` buttons when you are an admin, then `See member changes` for everyone. That one lists, newest first, who joined, left, was added or was removed. Press `Enter`, the Applications key or `Shift+F10` on a member to open their menu. Like in WhatsApp Web, the menu shows only the items that fit that member:

- `Message` and the name of the member, to open your chat with them.
- `Contact info`, to see their info, call them, or save them.
- `Add to contacts`, when they are not in your contacts.
- `Make group admin`, when you are an admin and they are not. whatSnap asks first.
- `Remove`, when you are an admin. The person who made the group can't be removed. whatSnap asks first.
- `Dismiss as admin`, when you are an admin and they are an admin, but not the person who made the group.
- `Verify security code`, for anyone but you. It shows the 60 numbers to compare with the numbers on their phone, and a QR code their phone can scan. `Copy code` copies the numbers.
- `Send message history`, for someone who joined in the last 30 days and hasn't had it yet, when you are an admin or the group lets all members share history. It sends them up to 100 messages from the 30 days before they joined. whatSnap asks first and tells you how many messages they will get.
- `Edit member tag`, on your own name. It goes to the `Your tag in this group` box.

## Calls
To call, open a chat and use `Call` or `Video call`. You can also call from the Calls section, or press `Enter` on a call in the Messages list.

When you call someone, the call window shows the name of the person you call and their phone number, if whatSnap knows it. Below that it shows how the call is going, like WhatsApp does: `Calling...` until their phone gets the call, then `Ringing...`, and `Connecting...` when they answer.

When a call comes in, the call window opens and whatSnap plays the ringtone. Press `Alt+Shift+Y` to answer, or `Alt+Shift+N` to decline. These two shortcuts work from all programs, but only while a call rings. If another program uses one of them, whatSnap tells you. `Dismiss` (or `Escape`) closes the call window and silences the ringtone, but the call keeps ringing: a bar at the top of the main window then offers `Open call window`, `Answer` and `Decline`. When a call ends, the call window stays with `Call back` and `Close`.

The call window has these buttons:

- `Mute microphone`
- `Turn camera on`
- `Share screen`
- `Device settings`
- `Add participant`
- `Raise hand`
- `React...`
- `End call`

In the call window, `Ctrl+Shift+V` turns the camera on or off, `Ctrl+Shift+S` starts or stops sharing your screen, `Ctrl+Shift+M` mutes or unmutes your microphone, and `Ctrl+Shift+D` opens or closes the device settings. In Settings, Input & Output, `Mute microphone by default when placing or joining calls` starts every call muted, and `Show frequent contacts in the Calls tab` hides or shows that list. In the main window, `Ctrl+M` mutes or unmutes your microphone, and `Ctrl+W` ends the call.

In a group call, the People list shows who is in the call. Use `Mute` to ask the selected person to mute. When you can remove people from the call, a `Remove` button also shows. In a call from a call link that you made, `Approve new participants` makes people wait until you let them in. Use `Let in` and `Don't let in` for each one. You can also select your microphone, speaker, and camera in the call window.

### Deleting Calls
In the Calls section, select a call under Recent calls and press `Delete`, or open its context menu and choose `Delete call`. To delete all your calls, choose `Clear call log...` in the same menu. whatSnap deletes a call the same way WhatsApp Web does, so the call is also deleted on your other linked devices.

### Call Links
- `Call -> Create Call Link...` makes a link that anyone can use to join a call.
- `Call -> Join Call Link...` joins a call from a link.

## Status
The Status section shows the status updates of your contacts, one row for each person, the same as WhatsApp Web. Your own updates are first, as `My status`. Then come the headings `Recent` (people with updates that you did not see yet), `Viewed`, and `Muted`. A row says how many updates the person posted, how many are new, and the time of the newest one. A person with one update shows that update, as before.

Press `Enter` on a row to open the status viewer. The viewer starts at the first update of that person that you did not see, and plays the updates one after another. After the last update of a person, it continues with the next person in the same part of the list. `My status` plays only your own updates.

| Key | Action |
|---|---|
| `Left Arrow` | Go to the previous update. |
| `Right Arrow` | Go to the next update. |
| `Space` | Pause or continue. |
| `H` | Like the update with a heart, or remove your like. `Alt+L` does the same. |
| `Escape` | Close the viewer. |

In the viewer, you can like, reply, react, open the chat, and mute the person. `Like` sends a green heart that only the person who posted the update sees, the same as the like button of WhatsApp Web. The other emoji send a reply to the person's chat. For your own updates, you can see who viewed them and delete them. The context menu of a row has the same actions. On `My status`, if you have more than one update, `Viewed by...` and `Delete...` ask which update you mean.

To post a status, use `Post Status Update...` for text, or `Post Photo/Video Status...`.

## Channels and Communities
- The Channels section shows the channels that you follow. Use `Find channels...` to search for channels, and `Create channel...` to make your own.
- The Communities section shows your communities. Open the information of a community to see its groups and join them.

## Video Player
Videos and the status viewer use the video player of whatSnap. These keys work in the player:

| Key | Action |
|---|---|
| `K` or `Space` | Play or pause. |
| `J` | Go back 10 seconds. |
| `L` | Go forward 10 seconds. |
| `M` | Mute or unmute. |
| `T` | Announce the current time. |

On the Position slider, the arrow keys move 5 seconds, `Page Up` and `Page Down` move 30 seconds, and `Home` and `End` go to the start or end. On the Volume slider, the arrow keys change the volume by 10%.

## Connection
whatSnap tells you when it loses the connection to WhatsApp, and when it connects again. To try to connect immediately, press `F5` (`Account -> Reconnect Now`).

## Window Visibility and Tray
- When `Show an icon in the notification area` is on, closing the window hides whatSnap in the tray. whatSnap continues to get messages and calls.
- The tray icon tells you the number of unread messages. Its menu has two items:
  - `Open whatSnap`
  - `Exit`
- Only one copy of whatSnap runs. If you start it again, the open copy comes to the front.

## Settings
To open the settings, press `Ctrl+,` (`Account -> Settings...`). Some settings sync with WhatsApp on your phone and your other linked devices. You can't change these settings until whatSnap gets them from WhatsApp.

Settings has ten tabs: General, Account, Chats, Notifications, Personalization, Profile, Privacy, Storage & Media, Input & Output, and Chat Lock. Each tab groups its settings under short headings.

### General Tab
- `Start whatSnap when Windows starts`: whatSnap then starts in the background without opening its window. It connects, syncs and notifies as usual; open it from the notification area icon or by starting whatSnap again.
- `Show an icon in the notification area; closing the window minimizes to it`
- `Start maximized`
- `Announce when someone is typing`
- `Announce when someone is recording a voice message`
- `Show a button to message Meta AI in New Chat`
- `Check for updates when whatSnap starts`
- `Updates`: Stable releases or Development builds.
- `Verbose backend logging, for troubleshooting connection issues`: the change takes effect the next time whatSnap starts.
- `Keyboard Shortcuts...`
- `Add whatSnap to the Windows Share Menu...`

### Account Tab
- `Show security notifications`: say in a chat when the security code of a contact changes.
- `Accounts...`: opens the Accounts dialog, the same as `Account -> Accounts...`. See [Several Accounts](#several-accounts).
- `Log Out...`: logs the account on screen out of WhatsApp on this computer, the same as `Account -> Log Out`. It asks first.

Both buttons close the settings without saving them.

### Chats Tab
- `Enter is send`
- `Hide phone numbers in groups`: when someone in a group has no saved name and no WhatsApp profile name, whatSnap says `Unknown member` instead of reading their phone number. Messages, replies, the chat list and notifications all follow it. It is off by default. People with a profile name always show that name.
- `Typing a letter in the message list starts a message`: the letter keys of the Messages list then need `Alt`.
- `Keep my place in the message list instead of jumping to the newest message`: when you send or delete a message, the list stays where you were.
- `Replace text with emoji as you type`, and `Replace Text With Emoji: See Help...` for the full list.
- `Check spelling in the message box`
- `Say when a word you type is misspelled`
- `Spelling Language`

### Notifications Tab
- `Message notifications`
- `Show previews`: show the message text in notifications.
- `Reaction notifications`
- `Incoming sounds`
- `Message notification tone`: `Default`, `Alert 1` to `Alert 10`, or `Custom sound file`. Each tone plays when you pick it. For a custom file, press Tab to the `Choose Message Tone File` button after it. A custom tone plays for up to 10 seconds and is not shared with your other computers.
- `Outgoing sounds`
- `Group notifications`
- `Group sounds`
- `Group notification tone`
- `Call notifications`
- `Incoming call sounds`: play the ringtone for incoming calls.
- `Call ringtone` and `Group call ringtone`: `WhatsApp`, the ringtones Windows comes with (`Windows Ring 1` to `10` and `Windows Alarm 1` to `10`), or `Custom sound file`, with a `Choose ... File` button after each. The ringtone repeats until you answer or the call stops. A short part plays when you pick one.
- `No message sound while Windows Do Not Disturb, a full-screen game or a presentation is on`
- `Read new messages aloud in the open chat`: each chat can change this in its notification settings, or with `Alt+L`.
- `Also read your own sent messages aloud`: off by default. It follows the setting above, so a chat that doesn't read new messages doesn't read yours either.

How accounts that aren't on screen alert you is set for each account in the Accounts dialog.

### Personalization Tab
- `Choose Theme`: System Default, Light, or Dark. System Default follows the light or dark mode of Windows.

### Profile Tab
- `Change Profile Photo...` and `Remove Profile Photo...`
- `Name`
- `About`, with `Clear About After`.

### Privacy Tab
- `Default Message Timer for New Chats`
- `Who Can See My Last Seen`
- `Who Can See When I'm Online`
- `Who Can See My Profile Photo`
- `Who Can See My About`
- `Read receipts`: when this is off, you don't send or get read receipts. In groups, read receipts are always sent.
- `Who Can Add Me to Groups`
- `Silence Unknown Callers`
- `Turn off link previews`: links that you send go without a preview, on all your devices.
- `Manage Blocked Contacts...`: the same list as `Blocked Contacts...` in the menu. It opens over the settings.

The choices for the "Who can" settings are Everyone, My Contacts, My Contacts Except..., and Nobody. When you select My Contacts Except..., use the button below it to select the contacts.

### Storage & Media Tab
- `Download photos automatically`
- `Download audio automatically`
- `Download videos automatically`
- `Download documents automatically`
- Voice messages always download. Other media downloads automatically only from contacts, and from chats that you wrote in.
- `Download Quality for Photos and Videos Sent in HD`: Standard Quality or HD Quality. People can send a photo or video in HD; with HD Quality, whatSnap downloads the HD copy instead of the standard one.
- `Media Upload Quality`: Standard Quality or HD Quality.

### Input & Output Tab
- `Microphone (calls and voice notes)`
- `Clean up voice messages`: remove background noise and make the volume even, as WhatsApp Web does.
- `Speaker for alerts and voice messages`: used for notification sounds, message tones, and voice messages.
- `Speaker for calls`
- `Experimental: keep the camera on while sharing the screen`: the change takes effect in the next call.

## Keyboard Shortcuts
You can change the shortcuts in the tables below with `Keyboard Shortcuts...` on the General tab of the settings. The dialog has a tab for each menu, and a Messages tab for the keys of the Messages list. You can set a new shortcut, remove a shortcut, or set the default again.

### Fixed Keys
You can't change these keys:

- `Tab` and `Shift+Tab`: move between the controls.
- `Up Arrow`, `Down Arrow`, `Page Up`, `Page Down`, `Home`, and `End`: move in a list.
- `Enter`: open the selected item.
- `Escape`: in the search field of the chat list, clear the search. In the search of the open chat, close the search. While you reply to a message, remove the quote. Otherwise, close the chat, or close a dialog.
- `Ctrl+C` and `Ctrl+V`: copy and paste.
- `Shift+F10` or the Applications key: open the actions menu.
- `Ctrl+Shift+Left Arrow` and `Ctrl+Shift+Right Arrow`: move a section in the Main Navigation list.

### Default Shortcuts

| Action | Menu | Default |
|---|---|---|
| Settings... | Account | `Ctrl+,` |
| Accounts... | Account | `Ctrl+Shift+A` |
| Reconnect Now | Account | `F5` |
| Chats | Go | `Alt+1` |
| Calls | Go | `Alt+2` |
| Status | Go | `Alt+3` |
| Starred Messages | Go | `Alt+4` |
| Archived Chats | Go | `Alt+5` |
| Communities | Go | `Alt+6` |
| Channels | Go | `Alt+7` |
| New Chat | Chats | `Ctrl+N` |
| New Group | Chats | `Ctrl+Shift+N` |
| Search | Chats | `Ctrl+/` |
| Chat Info | Conversation | `Alt+I` |
| Jump to Message Field | Conversation | `Alt+E` |
| Jump to Messages List | Conversation | `Alt+M` |
| Announce Chat Status | Conversation | `Alt+T` |
| Read New Messages Aloud | Conversation | `Alt+L` |
| Emoji... | Conversation | `Alt+Shift+E` |
| Stickers... | Conversation | `Alt+Shift+S` |
| Next Spelling Error | Conversation | `F7` |
| Search in Chat | Conversation | `Ctrl+Shift+F` |
| Schedule Message... | Conversation | `Ctrl+Shift+T` |
| Scheduled Messages... | Conversation | `Ctrl+Shift+K` |
| Mute Chat | Conversation | `Ctrl+Shift+M` |
| Archive Chat | Conversation | `Ctrl+Shift+E` |
| Pin Chat | Conversation | `Ctrl+Shift+P` |
| Lock Chat | Conversation | `Ctrl+L` |
| Mark as Unread | Conversation | `Ctrl+Shift+U` |
| Star Message | Conversation | `Alt+8` |
| Start Recording | None | `Ctrl+Shift+R` |
| Pause Recording | None | `Alt+P` |
| Play/Pause Voice Preview | None | `Alt+Shift+P` |
| Discard Voice Recording | None | `Alt+D` |
| Send Voice Message | None | `Ctrl+Enter` |
| Toggle Mute | Calls | `Ctrl+M` |
| End Call | Calls | `Ctrl+W` |
| Answer Incoming Call | Calls, global | `Alt+Shift+Y` |
| Reject Incoming Call | Calls, global | `Alt+Shift+N` |
| Reply | Messages | `R` |
| Edit | Messages | `E` |
| Star or unstar | Messages | `S` |
| Forward | Messages | `F` |
| Vote in poll | Messages | `V` |
| Delete | Messages | `Delete` |
| Voice message: back 5 seconds | Messages | `J` |
| Voice message: forward 5 seconds | Messages | `L` |
| Voice message: stop | Messages | `Shift+Space` |
| Selection mode on or off | Messages | `Ctrl+Space` |
| View Message Text | Messages | `Shift+Enter` |

The keys of the Messages list work only when the list has the focus. They need the exact modifiers. For example, `R` replies, but `Ctrl+R` does not.

## Sharing Files Into whatSnap
- In File Explorer, select files, then use `Send to -> whatSnap`. whatSnap asks which chats to send them to. You can select up to 5 chats and add a caption.
- To use the Windows Share menu, select `Add whatSnap to the Windows Share Menu...` on the General tab of the settings, or select the task in the installer. An administrator has to approve this one time.

## Links
whatSnap opens `whatsapp://` links, such as the "Share on WhatsApp" buttons on web pages and `wa.me` pages. If another app, such as WhatsApp for Windows, also opens these links, select whatSnap in `Settings -> Apps -> Default apps` of Windows.

## Data Folder
- whatSnap keeps your session, messages, media, and settings in `%LOCALAPPDATA%\whatSnap`.
- The settings are in `settings.json`. They are the same for every account.
- Each account you add after the first has its own folder in `accounts`, with its own session, messages, and media. All accounts share the `logs` folder. The list of accounts is in `accounts.json`.
- The logs are in the `logs` folder. whatSnap keeps the logs of this session and of the previous session. If you report a problem, send these logs before you start whatSnap again.

When you uninstall whatSnap, the uninstaller asks if you want to delete your data. It deletes only the files that whatSnap made. Files that you saved to other folders, such as Downloads, stay.

## Features Not Yet Tested
These features are built, but nobody has tested them with a real WhatsApp account yet. They can have problems:

- The automatic download settings, `Download Quality for Photos and Videos Sent in HD`, and `Media Upload Quality`.
- `Turn off link previews`.
- `Hide phone numbers in groups`.
- Buttons and menus in messages from businesses, and `Business Catalogue...`.
- Several accounts: adding, switching, removing, and the notifications and calls of the accounts that aren't on screen.

If you test one of these, please tell us if it works.
