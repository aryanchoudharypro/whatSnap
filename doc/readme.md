# whatSnap User Manual

[whatSnap](https://github.com/aryanchoudharypro/whatSnap) is a native, keyboard-first WhatsApp client for Windows, made for screen reader users. It links to the WhatsApp account on your phone, as WhatsApp Web and WhatsApp for Windows do. It does not use a browser.

whatSnap is in development. Most features are tested with a real WhatsApp account, but some are not yet. See [Features Not Yet Tested](#features-not-yet-tested).

## System Requirements
- Windows 10 or 11, on x64.
- WhatsApp on your phone. whatSnap is a linked device, so you need a phone with WhatsApp to link it the first time.

## Core Features
- A native user interface with controls that work well with screen readers. whatSnap announces changes as they occur, such as new messages, typing, and the connection status. The message list is a custom list control that uses [AccessKit](https://accesskit.dev). Your screen reader reads the full text of each message.
- Chats, groups, communities, channels, status updates, starred messages, archived chats, and the call history.
- Messages, with:
  - Replies, forwarding, editing, deleting, reactions, and stars.
  - Photos, videos, audio files, documents, contacts, stickers, and view once media.
  - Voice messages, which you can pause while you record. whatSnap can remove background noise from them.
  - Polls, quizzes, and events.
  - Mentions, link previews, and disappearing messages.
- Voice and video calls, in one to one chats and in groups:
  - Answer or reject a call from any program, with a global shortcut.
  - Screen sharing. You can also keep your camera on while you share your screen.
  - Raise your hand, send reactions, add people, and mute or remove people in group calls.
  - Call links. Create a link, or join a call from a link.
  - Join a group call that is already in progress.
  - whatSnap connects again by itself if your network drops during a call.
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

- With a phone number. Select your country, type your phone number, and press `Continue`. whatSnap shows a pairing code. On your phone, open WhatsApp, go to `Linked devices`, select `Link a device`, then select `Link with phone number instead` and type the code.
- With a QR code. Press `Connect with QR Code`. On your phone, open WhatsApp, go to `Linked devices`, select `Link a device`, and scan the code.

After you link, whatSnap gets your recent chats from your phone. This can take a few minutes.

To unlink this computer, use `Account -> Log Out`. Log Out has no shortcut, so you can't log out by mistake.

## Main Window Layout
The main window has three parts:

- The Main Navigation list shows the sections of whatSnap: Chats, Status, Starred Messages, Archived Chats, Communities, Channels, and Calls.
- The list of the current section, for example your chats. Above it are the `Search chats` field, the `Filters` box, and the `New Chat` button.
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
| Communities | `Alt+6` | Your communities and their groups. |
| Channels | `Alt+7` | The channels that you follow. |

## Chats

### The Chat List
Each chat in the list shows its name, the number of unread messages, and the last message. Press `Enter` to open a chat. Press the Applications key or `Shift+F10` to open the actions menu of a chat. The menu has these items, when they apply to the chat:

- `Contact info`
- `Pin chat` or `Unpin chat`
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
Type in the `Search chats` field to find chats and messages. Press `Ctrl+/` to go to the field from anywhere.

The `Filters` box shows only some chats: All chats, Unread, Favorites, Groups, and your own chat lists. To make, rename, or delete chat lists, use `Chats -> Manage Lists...`. Your chat lists sync with your phone.

### New Chats and Groups
- `Ctrl+N` starts a new chat. You can select a contact, or type a phone number.
- `Ctrl+Shift+N` makes a new group.
- `Chats -> New Community...` makes a new community.

### Locked Chats
A locked chat doesn't show in the chat list. When you have locked chats, a `Locked Chats` button shows below `New Chat`. Press it and type your secret code to show them.

### Chat Notifications
`Notification settings...` in the actions menu of a chat sets how this computer tells you about the chat. It does not change your phone. The choices are:

- `Silent`: no sound and no notification.
- `Sound only`: the message sound, but no notification.
- `Notify normally`.

For an archived chat, `Archived alerts` sets how the chat tells you about new messages.

## The Open Chat
The open chat has these controls, in this order:

- The buttons at the top: `Conversation data`, `Search in conversation`, `Call`, and `Video call`. When a group call is in progress, a `Join call` button also shows.
- The Messages list.
- The buttons for the selected message, such as `Play voice message`, `Open`, `Save as`, and `Download`.
- The `Emoji and stickers` button.
- The message box, named `Type a message`.
- The `Add attachment`, `Send message`, and `Record voice message` buttons.

Press `Alt+M` to go to the Messages list. Press `Alt+E` to go to the message box. Press `Alt+E` again to go back to where you were. Press `Alt+T` to hear the status of the chat, for example online, or typing. Press `Alt+I` to open the information of the chat. Press `Escape` to close the chat.

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
| `Shift+Enter` | Show the full text of the message in a dialog. |
| `Left Arrow` | On a reply, read the message that it replies to. |
| `Left Arrow` two times | On a reply, go to the message that it replies to. |
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

While a voice message plays, `Left Arrow` and `J` go back 5 seconds, and `Right Arrow` and `L` go forward 5 seconds. To change the speed, use the speed button. Its name is the current speed, for example `1.0x`.

The actions menu of a message has these items, when they apply to the message: `Copy`, `Reply`, `Forward...`, `Edit`, `Delete`, `React...`, `Star` or `Unstar`, `Retry send`, `Request again`, `Save voice message...`, `Message Info`, `Copy all photos`, `Copy Number`, `Message Number`, and `Report...`. In a group, a submenu for the sender lets you message or call them. In a channel, a submenu lets you pin a post, or see who reacted or voted.

### Selection Mode
In selection mode, `Space` selects or clears a message. A bar shows how many messages you selected, with `Delete` and `Cancel` buttons. To leave selection mode, press `Ctrl+Space` or `Escape`.

### Writing Messages
Type your message in the message box. If `Enter is send` is on in the settings, `Enter` sends the message, and `Shift+Enter` starts a new line. If it is off, use the `Send message` button.

- To reply, press `R` on a message. The reply shows above the message box. To remove it, use `Remove quote`.
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
| `Ctrl+Enter` | Send the voice message. |

You can also use the `Record voice message`, `Pause recording`, `Send voice message`, and `Discard voice message` buttons. Select `View once` before you send, to send it as a view once voice message.

### Other Chat Actions
The Conversation menu has more actions for the open chat. It shows only the ones that fit that chat:

- `Search in Chat` (`Ctrl+Shift+F`)
- `Pinned Messages...`, not in broadcast lists.
- `Business Catalogue...`, only in a chat with a business.
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

## Calls
To call, open a chat and use `Call` or `Video call`. You can also call from the Calls section, or press `Enter` on a call in the Messages list.

When a call comes in, the call window opens and whatSnap plays the ringtone. Press `Alt+Shift+Y` to answer, or `Alt+Shift+N` to reject. These two shortcuts work from all programs, but only while a call rings. If another program uses one of them, whatSnap tells you.

The call window has these buttons:

- `Mute microphone`
- `Turn camera on`
- `Share screen`
- `Add participant`
- `Raise hand`
- `React...`
- `Leave call`

In the call window, `Ctrl+Shift+V` turns the camera on or off, and `Ctrl+Shift+S` starts or stops sharing your screen. In the main window, `Ctrl+M` mutes or unmutes your microphone, and `Ctrl+W` ends the call.

In a group call, the People list shows who is in the call. Use `Mute` to ask the selected person to mute. When you can remove people from the call, a `Remove` button also shows. In a call from a call link that you made, `Approve new participants` makes people wait until you let them in. Use `Let in` and `Don't let in` for each one. You can also select your microphone, speaker, and camera in the call window.

### Call Links
- `Call -> Create Call Link...` makes a link that anyone can use to join a call.
- `Call -> Join Call Link...` joins a call from a link.

## Status
The Status section shows the status updates of your contacts. Press `Enter` on an update to open the status viewer. The viewer plays the updates one after another.

| Key | Action |
|---|---|
| `Left Arrow` | Go to the previous update. |
| `Right Arrow` | Go to the next update. |
| `Space` | Pause or continue. |
| `Escape` | Close the viewer. |

In the viewer, you can reply, react, open the chat, and mute the person. For your own updates, you can see who viewed them and delete them.

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

### General Tab
- `Announce when someone is typing`
- `Announce when someone is recording a voice message`
- `Show an icon in the notification area; closing the window minimizes to it`
- `Start whatSnap when Windows starts`
- `Start maximized`
- `Check for updates when whatSnap starts`
- `Updates`: Stable releases or Development builds.
- `Show a button to message Meta AI in New Chat`
- `Verbose backend logging, for troubleshooting connection issues`: the change takes effect the next time whatSnap starts.
- `Keyboard Shortcuts...`
- `Add whatSnap to the Windows Share Menu...`

### Notifications Tab
- `Message notifications`
- `Show previews`: show the message text in notifications.
- `Reaction notifications`
- `Incoming sounds`
- `Outgoing sounds`
- `Group notifications`
- `Group sounds`
- `Call notifications`
- `Incoming call sounds`: play the ringtone for incoming calls.
- `No message sound while Windows Do Not Disturb, a full-screen game or a presentation is on`

### Chats Tab
- `Enter is send`
- `Replace text with emoji as you type`
- `Check spelling in the message box`
- `Say when a word you type is misspelled`
- `Spelling Language`
- Automatic downloads:
  - `Download photos automatically`
  - `Download audio automatically`
  - `Download videos automatically`
  - `Download documents automatically`
  - Voice messages always download. Other media downloads automatically only from contacts, and from chats that you wrote in.
- `Media Upload Quality`: Standard Quality or HD Quality.
- `Theme`: System Default, Light, or Dark.

### Profile & Privacy Tab
- `Name` and `About`, with `Clear About After`.
- `Change Profile Photo...` and `Remove Profile Photo...`
- `Default Message Timer for New Chats`
- `Who Can See My Last Seen`
- `Who Can See When I'm Online`
- `Who Can See My Profile Photo`
- `Who Can See My About`
- `Read receipts`: when this is off, you don't send or get read receipts. In groups, read receipts are always sent.
- `Who Can Add Me to Groups`
- `Silence Unknown Callers`
- `Turn off link previews`: links that you send go without a preview, on all your devices.
- `Show security notifications`: say in a chat when the security code of a contact changes.

The choices for the "Who can" settings are Everyone, My Contacts, My Contacts Except..., and Nobody. When you select My Contacts Except..., use the button below it to select the contacts.

### Input & Output Tab
- `Microphone (calls)`: also used for voice messages.
- `Clean up voice messages`: remove background noise and make the volume even, as WhatsApp Web does.
- `Speaker (calls & playback)`: used for calls, voice messages, and sounds.
- `Experimental: keep the camera on while sharing the screen`: the change takes effect in the next call.

## Keyboard Shortcuts
You can change the shortcuts in the tables below with `Keyboard Shortcuts...` on the General tab of the settings. The dialog has a tab for each menu, and a Messages tab for the keys of the Messages list. You can set a new shortcut, remove a shortcut, or set the default again.

### Fixed Keys
You can't change these keys:

- `Tab` and `Shift+Tab`: move between the controls.
- `Up Arrow`, `Down Arrow`, `Page Up`, `Page Down`, `Home`, and `End`: move in a list.
- `Enter`: open the selected item.
- `Escape`: close the chat, or close a dialog.
- `Ctrl+C` and `Ctrl+V`: copy and paste.
- `Shift+F10` or the Applications key: open the actions menu.
- `Ctrl+Shift+Left Arrow` and `Ctrl+Shift+Right Arrow`: move a section in the Main Navigation list.

### Default Shortcuts

| Action | Menu | Default |
|---|---|---|
| Settings... | Account | `Ctrl+,` |
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
| Emoji... | Conversation | `Alt+Shift+E` |
| Stickers... | Conversation | `Alt+Shift+S` |
| Next Spelling Error | Conversation | `F7` |
| Search in Chat | Conversation | `Ctrl+Shift+F` |
| Mute Chat | Conversation | `Ctrl+Shift+M` |
| Archive Chat | Conversation | `Ctrl+Shift+E` |
| Pin Chat | Conversation | `Ctrl+Shift+P` |
| Lock Chat | Conversation | `Ctrl+L` |
| Mark as Unread | Conversation | `Ctrl+Shift+U` |
| Star Message | Conversation | `Alt+8` |
| Start Recording | None | `Ctrl+Shift+R` |
| Pause Recording | None | `Alt+P` |
| Send Voice Message | None | `Ctrl+Enter` |
| Toggle Mute | Call | `Ctrl+M` |
| End Call | Call | `Ctrl+W` |
| Answer Incoming Call | Call, global | `Alt+Shift+Y` |
| Reject Incoming Call | Call, global | `Alt+Shift+N` |
| Reply | Messages | `R` |
| Edit | Messages | `E` |
| Star or unstar | Messages | `S` |
| Forward | Messages | `F` |
| Vote in poll | Messages | `V` |
| Delete | Messages | `Delete` |
| Voice message: back 5 seconds | Messages | `J` |
| Voice message: forward 5 seconds | Messages | `L` |
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
- The settings are in `settings.json`.
- The logs are in the `logs` folder. whatSnap keeps the logs of this session and of the previous session. If you report a problem, send these logs before you start whatSnap again.

When you uninstall whatSnap, the uninstaller asks if you want to delete your data. It deletes only the files that whatSnap made. Files that you saved to other folders, such as Downloads, stay.

## Features Not Yet Tested
These features are built, but nobody has tested them with a real WhatsApp account yet. They can have problems:

- The automatic download settings, and `Media Upload Quality`.
- `Turn off link previews`.
- Buttons and menus in messages from businesses, and `Business Catalogue...`.

If you test one of these, please tell us if it works.
