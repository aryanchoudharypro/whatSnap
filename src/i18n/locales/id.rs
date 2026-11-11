pub fn translate(text: &str) -> Option<&'static str> {
	match text {
		// Navigation tabs & shortcut tab groups
		"Chats" => Some("Chat"),
		"Status" => Some("Status"),
		"Starred Messages" => Some("Pesan Berbintang"),
		"Archived Chats" => Some("Chat Diarsipkan"),
		"Communities" => Some("Komunitas"),
		"Channels" => Some("Saluran"),
		"Calls" => Some("Panggilan"),
		"Account" => Some("Akun"),
		"Go" => Some("Navigasi"),
		"Conversation" => Some("Percakapan"),
		"Voice" => Some("Pesan Suara"),
		"Call" => Some("Panggilan"),
		"unread" => Some("belum dibaca"),
		"Main Navigation" => Some("Navigasi Utama"),
		"Cannot move left" => Some("Tidak dapat digeser ke kiri"),
		"Cannot move right" => Some("Tidak dapat digeser ke kanan"),
		"Moved between" => Some("Dipindahkan di antara"),
		"and" => Some("dan"),
		"Moved before" => Some("Dipindahkan sebelum"),
		"Moved after" => Some("Dipindahkan setelah"),
		"Moved" => Some("Dipindahkan"),

		// Menu bar & tray menu
		"&Account" => Some("&Akun"),
		"&Go" => Some("&Navigasi"),
		"&Chats" => Some("&Chat"),
		"C&onversation" => Some("&Percakapan"),
		"Call&s" => Some("Pang&gilan"),
		"&Settings..." => Some("&Setelan..."),
		"A&ccounts..." => Some("&Akun..."),
		"&Reconnect Now" => Some("&Hubungkan Ulang Sekarang"),
		"&Blocked Contacts..." => Some("Kontak &Diblokir..."),
		"Check for &Updates..." => Some("Periksa &Pembaruan..."),
		"Log &Out" => Some("&Keluar"),
		"&Status" => Some("&Status"),
		"S&tarred Messages" => Some("Pesan &Berbintang"),
		"&Archived Chats" => Some("Chat &Diarsipkan"),
		"C&ommunities" => Some("K&omunitas"),
		"C&hannels" => Some("Sa&luran"),
		"&Calls" => Some("&Panggilan"),
		"&New Chat" => Some("Chat &Baru"),
		"New &Group" => Some("&Grup Baru"),
		"New C&ommunity..." => Some("K&omunitas Baru..."),
		"&Search" => Some("&Cari"),
		"Manage &Lists..." => Some("Kelola &Daftar..."),
		"Toggle &Mute" => Some("Beralih &Bisukan"),
		"&End Call" => Some("&Akhiri Panggilan"),
		"&Create Call Link..." => Some("&Buat Tautan Panggilan..."),
		"&Join Call Link..." => Some("&Gabung Lewat Tautan..."),
		"&Open whatSnap" => Some("&Buka whatSnap"),
		"Show the main window" => Some("Tampilkan jendela utama"),
		"E&xit" => Some("&Keluar"),
		"Quit whatSnap" => Some("Keluar dari whatSnap"),

		// Conversation menu & chat context menu
		"Search in &Chat" => Some("Cari dalam &Chat"),
		"&Mute Chat" => Some("&Bisukan Chat"),
		"&Archive Chat" => Some("&Arsipkan Chat"),
		"&Pin Chat" => Some("&Sematkan Chat"),
		"&Lock Chat" => Some("&Kunci Chat"),
		"Mark as &Unread" => Some("Tandai Belum &Dibaca"),
		"Mark as Read" => Some("Tandai Sudah Dibaca"),
		"Chat &Info" => Some("&Info Chat"),
		"&Star Message" => Some("&Bintangi Pesan"),
		"Sc&hedule Message..." => Some("&Jadwalkan Pesan..."),
		"Scheduled Messa&ges..." => Some("Pesan Ter&jadwal..."),
		"Jump to Message Fi&eld" => Some("Lompat ke Kolom &Pesan"),
		"Jump to &Messages List" => Some("Lompat ke Daftar &Pesan"),
		"Announce Chat Sta&tus" => Some("Umumkan Sta&tus Chat"),
		"Read Ne&w Messages Aloud" => Some("Bacakan Pesan &Baru"),
		"Em&oji..." => Some("Em&oji..."),
		"Ne&xt Spelling Error" => Some("Kesalahan Ejaan &Berikutnya"),
		"Stic&kers..." => Some("Sti&ker..."),
		"Go to &Date..." => Some("Buka &Tanggal..."),
		"Pi&nned Messages..." => Some("Pesan Ter&semat..."),
		"Business Catalo&gue..." => Some("Katalo&g Bisnis..."),
		"&Disappearing Messages..." => Some("Pesan &Sementara..."),
		"Ad&vanced Chat Privacy..." => Some("Privasi Chat &Lanjutan..."),
		"&Report..." => Some("&Laporkan..."),
		"&Block or Unblock Contact" => Some("&Blokir atau Buka Blokir Kontak"),
		"&Contact info" => Some("&Info kontak"),
		"C&hannel info" => Some("Info &saluran"),
		"&Pin chat" => Some("&Sematkan chat"),
		"Un&pin chat" => Some("Lepas &sematan chat"),
		"&Archive chat" => Some("&Arsipkan chat"),
		"&Unarchive chat" => Some("Batalkan &arsip chat"),
		"&Lock chat" => Some("&Kunci chat"),
		"Un&lock chat" => Some("Buka &kunci chat"),
		"Mark as &read" => Some("Tandai sudah &dibaca"),
		"Mark as &unread" => Some("Tandai belum &dibaca"),
		"Add to fa&vorites" => Some("Tambah ke fa&vorit"),
		"Remove from fa&vorites" => Some("Hapus dari fa&vorit"),
		"Add to l&ist..." => Some("Tambah ke daf&tar..."),
		"&Export chat..." => Some("&Ekspor chat..."),
		"Export &text only (Markdown)..." => Some("Ekspor &teks saja (Markdown)..."),
		"Leave &group..." => Some("Keluar dari &grup..."),
		"Clea&r chat..." => Some("Be&rsihkan chat..."),
		"Re&port..." => Some("La&porkan..."),
		"&Block contact..." => Some("&Blokir kontak..."),
		"Un&block contact" => Some("Buka &blokir kontak"),
		"&Delete chat..." => Some("&Hapus chat..."),
		"&Mute notifications" => Some("&Bisukan notifikasi"),
		"&Unmute notifications" => Some("&Bunyikan notifikasi"),

		// Menu help strings
		"Open settings" => Some("Buka setelan"),
		"Switch between, add and remove WhatsApp accounts" => Some("Beralih, menambah, dan menghapus akun WhatsApp"),
		"Retry connecting to WhatsApp right away" => Some("Coba menghubungkan ulang ke WhatsApp sekarang"),
		"See and change who you have blocked" => Some("Lihat dan ubah siapa yang Anda blokir"),
		"See if a newer version of whatSnap is out" => Some("Lihat apakah versi baru whatSnap tersedia"),
		"Log out of WhatsApp on this device" => Some("Keluar dari WhatsApp di perangkat ini"),
		"Jump to the chat list" => Some("Lompat ke daftar chat"),
		"Jump to Status" => Some("Lompat ke Status"),
		"Jump to Starred Messages" => Some("Lompat ke Pesan Berbintang"),
		"Jump to Archived Chats" => Some("Lompat ke Chat Diarsipkan"),
		"Jump to Communities" => Some("Lompat ke Komunitas"),
		"Jump to the channels you follow" => Some("Lompat ke saluran yang Anda ikuti"),
		"Jump to Calls" => Some("Lompat ke Panggilan"),
		"Start a new chat" => Some("Mulai chat baru"),
		"Start a new group" => Some("Mulai grup baru"),
		"Start a community to bring groups together" => Some("Mulai komunitas untuk menggabungkan beberapa grup"),
		"Search chats" => Some("Cari chat"),
		"Make, rename or delete your chat lists" => Some("Buat, ganti nama, atau hapus daftar chat Anda"),
		"Mute/unmute the microphone during a call" => Some("Bisukan/bunyikan mikrofon saat panggilan"),
		"End the active call" => Some("Akhiri panggilan aktif"),
		"Make a link anyone can use to join a call" => {
			Some("Buat tautan yang dapat digunakan siapa saja untuk bergabung")
		}
		"Join a call from a link someone shared" => Some("Gabung ke panggilan dari tautan yang dibagikan"),
		"Open this file" => Some("Buka file ini"),
		"Save this file to a chosen location" => Some("Simpan file ini ke lokasi yang dipilih"),
		"Download this file" => Some("Unduh file ini"),
		"Open info for the open chat" => Some("Buka info chat yang sedang terbuka"),
		"Move focus to the message box, or back to where you were" => {
			Some("Pindahkan fokus ke kolom pesan, atau kembali ke sebelumnya")
		}
		"Move focus to the messages list" => Some("Pindahkan fokus ke daftar pesan"),
		"Say the open chat's status: online, typing and so on" => {
			Some("Umumkan status chat: online, mengetik, dan sebagainya")
		}
		"Turn reading new messages aloud on or off for the open chat" => {
			Some("Nyalakan atau matikan pembacaan pesan baru untuk chat ini")
		}
		"Pick an emoji to put in the message box" => Some("Pilih emoji untuk dimasukkan ke kolom pesan"),
		"Pick a sticker to send: favorites, recent ones or WhatsApp's own packs" => {
			Some("Pilih stiker untuk dikirim: favorit, terbaru, atau paket WhatsApp")
		}
		"Select the next misspelled word in the message box and offer its fixes" => {
			Some("Pilih kata salah eja berikutnya di kolom pesan dan sarankan perbaikan")
		}
		"Search within the open chat" => Some("Cari di dalam chat yang sedang dibuka"),
		"Jump to the messages of a date in the open chat" => Some("Lompat ke pesan pada tanggal tertentu di chat ini"),
		"See and go to the open chat's pinned messages" => Some("Lihat dan buka pesan tersemat di chat ini"),
		"Browse the products of the business you're chatting with" => {
			Some("Jelajahi produk bisnis yang sedang chat dengan Anda")
		}
		"Pick a date and time for the message in the message box to be sent" => {
			Some("Pilih tanggal dan waktu pengiriman untuk pesan di kolom pesan")
		}
		"See, send now or delete the open chat's scheduled messages" => {
			Some("Lihat, kirim sekarang, atau hapus pesan terjadwal chat ini")
		}
		"Mute the open chat" => Some("Bisukan chat ini"),
		"Archive the open chat" => Some("Arsipkan chat ini"),
		"Pin the open chat" => Some("Sematkan chat ini"),
		"Lock or unlock the open chat" => Some("Kunci atau buka kunci chat ini"),
		"Toggle read/unread on the open chat" => Some("Tandai sudah/belum dibaca pada chat ini"),
		"Turn disappearing messages on or off for the open chat" => {
			Some("Nyalakan atau matikan pesan sementara untuk chat ini")
		}
		"Turn advanced chat privacy on or off for the open chat" => {
			Some("Nyalakan atau matikan privasi chat lanjutan untuk chat ini")
		}
		"Report the open chat's contact or group to WhatsApp" => Some("Laporkan kontak atau grup chat ini ke WhatsApp"),
		"Block or unblock the open chat's contact" => Some("Blokir atau buka blokir kontak chat ini"),
		"Star the selected message" => Some("Bintangi pesan yang dipilih"),
		"See this contact's info, call them, or edit or delete the contact" => {
			Some("Lihat info kontak ini, telepon, atau edit maupun hapus kontak")
		}
		"See this channel's details without opening it" => Some("Lihat detail saluran ini tanpa membukanya"),
		"Pin or unpin chat" => Some("Sematkan atau lepas sematan chat"),
		"Archive or unarchive chat" => Some("Arsipkan atau batalkan arsip chat"),
		"Lock or unlock chat" => Some("Kunci atau buka kunci chat"),
		"Mark chat as read or unread" => Some("Tandai chat sudah atau belum dibaca"),
		"Show or stop showing this chat under Favorites" => Some("Tampilkan atau hapus chat ini dari Favorit"),
		"Choose which of your lists this chat is in" => Some("Pilih daftar mana saja yang berisi chat ini"),
		"Export this chat's transcript and media" => Some("Ekspor transkrip dan media chat ini"),
		"Export just the message text as a Markdown file, no media" => {
			Some("Ekspor teks pesan saja sebagai file Markdown tanpa media")
		}
		"Leave this group" => Some("Keluar dari grup ini"),
		"Delete this chat's messages, keeping the chat itself" => {
			Some("Hapus pesan di chat ini tanpa menghapus chatnya")
		}
		"Report this group to WhatsApp" => Some("Laporkan grup ini ke WhatsApp"),
		"Report this contact to WhatsApp" => Some("Laporkan kontak ini ke WhatsApp"),
		"Let this contact message and call you again" => {
			Some("Izinkan kontak ini mengirim pesan dan menelepon Anda lagi")
		}
		"Stop this contact messaging and calling you" => Some("Hentikan kontak ini mengirim pesan dan menelepon Anda"),
		"Delete chat history" => Some("Hapus riwayat chat"),
		"Mute or unmute notifications" => Some("Bisukan atau bunyikan notifikasi"),
		"Choose how long to mute this chat" => Some("Pilih berapa lama chat ini dibisukan"),

		// Keymap actions
		"Start Recording" => Some("Mulai Merekam"),
		"Pause Recording" => Some("Jeda Perekaman"),
		"Play/Pause Voice Preview" => Some("Putar/Jeda Pratinjau Suara"),
		"Discard Voice Recording" => Some("Buang Rekaman Suara"),
		"Send Voice Message" => Some("Kirim Pesan Suara"),
		"Voice message: back 5 seconds" => Some("Pesan suara: mundur 5 detik"),
		"Voice message: forward 5 seconds" => Some("Pesan suara: maju 5 detik"),
		"Voice message: stop" => Some("Pesan suara: berhenti"),
		"Voice message: slower" => Some("Pesan suara: lebih lambat"),
		"Voice message: faster" => Some("Pesan suara: lebih cepat"),
		"Answer Incoming Call" => Some("Jawab Panggilan Masuk"),
		"Reject Incoming Call" => Some("Tolak Panggilan Masuk"),
		"Reply" => Some("Balas"),
		"Edit" => Some("Edit"),
		"Star or unstar" => Some("Bintangi atau hapus bintang"),
		"Forward" => Some("Teruskan"),
		"Vote in poll" => Some("Pilih dalam polling"),
		"Delete" => Some("Hapus"),
		"Selection mode on or off" => Some("Mode seleksi aktif atau nonaktif"),
		"View Message Text" => Some("Lihat Teks Pesan"),

		// Status bar
		"Status: Initializing..." => Some("Status: Memulai..."),
		"Status: Not logged in" => Some("Status: Belum masuk"),
		"Status: Connecting..." => Some("Status: Menghubungkan..."),
		"Status: Connected" => Some("Status: Terhubung"),
		"Status: Connected (Downloading messages)" => Some("Status: Terhubung (Mengunduh pesan)"),
		"Status: Connected (Syncing older messages)" => Some("Status: Terhubung (Menyinkronkan pesan lama)"),
		"Status: Connected (Syncing paused. Open WhatsApp on your phone to continue syncing.)" => {
			Some("Status: Terhubung (Sinkronisasi dijeda. Buka WhatsApp di ponsel Anda untuk melanjutkan.)")
		}
		"Connected to WhatsApp." => Some("Terhubung ke WhatsApp."),

		// UI Buttons, filters, and labels
		"&Open" => Some("&Buka"),
		"&Save as..." => Some("&Simpan sebagai..."),
		"&Download" => Some("&Unduh"),
		"Open" => Some("Buka"),
		"Save as" => Some("Simpan sebagai"),
		"Save as..." => Some("Simpan sebagai..."),
		"Download" => Some("Unduh"),
		"Save" => Some("Simpan"),
		"Cancel" => Some("Batal"),
		"Close" => Some("Tutup"),
		"Send" => Some("Kirim"),
		"Discard" => Some("Buang"),
		"Create" => Some("Buat"),
		"Add" => Some("Tambah"),
		"Remove" => Some("Hapus"),
		"Copy" => Some("Salin"),
		"Star" => Some("Bintangi"),
		"Keep" => Some("Simpan"),
		"Mute" => Some("Bisukan"),
		"Mark as read" => Some("Tandai sudah dibaca"),
		"Search" => Some("Cari"),
		"Search unread chats" => Some("Cari chat belum dibaca"),
		"Search favourite chats" => Some("Cari chat favorit"),
		"Search group chats" => Some("Cari chat grup"),
		"Search status updates" => Some("Cari pembaruan status"),
		"Search starred messages" => Some("Cari pesan berbintang"),
		"Search or enter phone number to place call" => Some("Cari atau masukkan nomor telepon untuk menelepon"),
		"Search or start a new chat" => Some("Cari atau mulai chat baru"),
		"Filters" => Some("Filter"),
		"All chats" => Some("Semua chat"),
		"Unread" => Some("Belum dibaca"),
		"Favorites" => Some("Favorit"),
		"Groups" => Some("Grup"),
		"New Chat" => Some("Chat Baru"),
		"New Group" => Some("Grup Baru"),
		"Status Updates" => Some("Pembaruan Status"),
		"Recent Calls" => Some("Panggilan Terbaru"),
		"Post Status Update..." => Some("Posting Pembaruan Status..."),
		"Find channels..." => Some("Cari saluran..."),
		"Create channel..." => Some("Buat saluran..."),
		"Locked Chats" => Some("Chat Dikunci"),
		"Hide Locked Chats" => Some("Sembunyikan Chat Dikunci"),
		"Post Photo/Video Status..." => Some("Posting Status Foto/Video..."),
		"Status Privacy..." => Some("Privasi Status..."),
		"Video call" => Some("Panggilan video"),
		"Conversation data" => Some("Info percakapan"),
		"Search in conversation" => Some("Cari dalam chat"),
		"Join call" => Some("Gabung panggilan"),
		"Incoming call..." => Some("Panggilan masuk..."),
		"Open call window" => Some("Buka jendela panggilan"),
		"Answer" => Some("Jawab"),
		"Decline" => Some("Tolak"),
		"Close search" => Some("Tutup pencarian"),
		"Search conversation" => Some("Cari percakapan"),
		"Messages" => Some("Pesan"),
		"Play voice message" => Some("Putar pesan suara"),
		"Read more" => Some("Baca selengkapnya"),
		"Retry send" => Some("Coba kirim lagi"),
		"Emoji and stickers" => Some("Emoji dan stiker"),
		"Type a message" => Some("Ketik pesan"),
		"You blocked this contact. Click to unblock." => Some("Anda memblokir kontak ini. Klik untuk membuka blokir."),
		"Add attachment" => Some("Tambah lampiran"),
		"Cancel edit" => Some("Batalkan edit"),
		"Remove quote" => Some("Hapus kutipan"),
		"Send message" => Some("Kirim pesan"),
		"Record voice message" => Some("Rekam pesan suara"),
		"Send voice message" => Some("Kirim pesan suara"),
		"Caption" => Some("Keterangan"),
		"Add more files" => Some("Tambah file lagi"),
		"Send attachment" => Some("Kirim lampiran"),
		"Discard voice message" => Some("Buang pesan suara"),
		"Pause recording" => Some("Jeda perekaman"),
		"Play preview" => Some("Putar pratinjau"),
		"View once" => Some("Sekali lihat"),
		"&View once" => Some("&Sekali lihat"),
		"0 selected" => Some("0 dipilih"),
		"Search name or number" => Some("Cari nama atau nomor"),
		"New Contact" => Some("Kontak Baru"),
		"Name" => Some("Nama"),
		"Surname" => Some("Nama belakang"),
		"Phone Number" => Some("Nomor Telepon"),
		"Also save to my phone's WhatsApp contacts" => Some("Simpan juga ke kontak WhatsApp di ponsel saya"),
		"Message Meta AI" => Some("Kirim pesan ke Meta AI"),
		"Group name" => Some("Nama grup"),
		"Contacts" => Some("Kontak"),
		"Search contacts" => Some("Cari kontak"),
		"Extra number (optional)" => Some("Nomor tambahan (opsional)"),
		"Create group" => Some("Buat grup"),
		"Blocked Contacts" => Some("Kontak Diblokir"),
		"Blocked contacts" => Some("Kontak diblokir"),
		"Unblock" => Some("Buka blokir"),
		"Block a contact..." => Some("Blokir kontak..."),
		"Blocked contacts will no longer be able to call you or send you messages." => {
			Some("Kontak yang diblokir tidak lagi dapat menelepon atau mengirimi Anda pesan.")
		}
		"Accounts" => Some("Akun"),
		"Linked accounts" => Some("Akun terhubung"),
		"&Switch" => Some("&Beralih"),
		"&Add Account..." => Some("&Tambah Akun..."),
		"Re&name..." => Some("Ganti &Nama..."),
		"N&otifications..." => Some("N&otifikasi..."),
		"&Remove..." => Some("&Hapus..."),
		"When a message comes in" => Some("Saat pesan masuk"),
		"When someone calls" => Some("Saat seseorang menelepon"),
		"These apply while another account is on screen. Each chat's own notification settings still count, and the account on screen always uses the normal settings." => {
			Some(
				"Ini berlaku saat akun lain sedang aktif di layar. Setelan notifikasi tiap chat tetap berlaku, dan akun di layar selalu memakai setelan normal.",
			)
		}
		"Notification and sound" => Some("Notifikasi dan suara"),
		"Notification only, no sound" => Some("Notifikasi saja, tanpa suara"),
		"Sound only, no notification" => Some("Suara saja, tanpa notifikasi"),
		"Off" => Some("Mati"),
		"On" => Some("Nyala"),
		"Switch to this account and ring" => Some("Beralih ke akun ini dan deringkan"),
		"Notification only" => Some("Notifikasi saja"),
		"Question" => Some("Pertanyaan"),
		"Create poll" => Some("Buat polling"),
		"Options, one per line (2 to 12)" => Some("Pilihan, satu per baris (2 sampai 12)"),
		"Allow multiple answers" => Some("Izinkan beberapa jawaban"),
		"Quiz (one correct answer)" => Some("Kuis (satu jawaban benar)"),
		"What to share" => Some("Yang ingin dibagikan"),
		"Share" => Some("Bagikan"),
		"Search chats and contacts" => Some("Cari chat dan kontak"),
		"Send to" => Some("Kirim ke"),
		"Caption (optional)" => Some("Keterangan (opsional)"),
		"Message" => Some("Pesan"),
		"Date" => Some("Tanggal"),
		"Hour" => Some("Jam"),
		"Minute" => Some("Menit"),
		"Schedule message" => Some("Jadwalkan pesan"),
		"Scheduled messages: loading" => Some("Pesan terjadwal: memuat"),
		"Send now" => Some("Kirim sekarang"),
		"Copy text" => Some("Salin teks"),
		"Post &Status" => Some("Posting &Status"),
		"Selected &file:" => Some("&File yang dipilih:"),
		"&Caption (optional):" => Some("&Keterangan (opsional):"),
		"&Type a status update:" => Some("&Ketik pembaruan status:"),
		"Insert &Emoji..." => Some("Sisipkan &Emoji..."),
		"Colour:" => Some("Warna:"),
		"&Next Colour" => Some("Warna &Berikutnya"),
		"Font:" => Some("Font:"),
		"Next &Font" => Some("Font &Berikutnya"),
		"Visible to your configured status audience." => Some("Terlihat oleh penonton status yang Anda tentukan."),
		"Status &Privacy..." => Some("&Privasi Status..."),

		// Call window controls
		"Microphone" => Some("Mikrofon"),
		"Speaker" => Some("Speaker"),
		"Camera" => Some("Kamera"),
		"Accept call" => Some("Terima panggilan"),
		"Decline call" => Some("Tolak panggilan"),
		"Dismiss" => Some("Tutup"),
		"Mute microphone" => Some("Bisukan mikrofon"),
		"Turn camera on" => Some("Nyalakan kamera"),
		"Share screen" => Some("Bagikan layar"),
		"Device settings" => Some("Setelan perangkat"),
		"Add participant" => Some("Tambah peserta"),
		"Raise hand" => Some("Angkat tangan"),
		"React..." => Some("Reaksi..."),
		"End call" => Some("Akhiri panggilan"),
		"Call back" => Some("Telepon balik"),
		"People" => Some("Orang"),
		"Let in" => Some("Izinkan masuk"),
		"Don't let in" => Some("Tolak masuk"),
		"Approve new participants" => Some("Setujui peserta baru"),
		"Call info" => Some("Info panggilan"),

		// Login & QR Code
		"WhatsApp Login" => Some("Masuk WhatsApp"),
		"Scan this QR Code with WhatsApp:" => Some("Pindai Kode QR ini dengan WhatsApp:"),
		"Loading QR Code..." => Some("Memuat Kode QR..."),
		"Loading new QR Code..." => Some("Memuat Kode QR baru..."),
		"Reload QR Code" => Some("Muat Ulang Kode QR"),
		"Country:" => Some("Negara:"),
		"Enter phone number:" => Some("Masukkan nomor telepon:"),
		"Continue" => Some("Lanjut"),
		"Connect with QR Code" => Some("Hubungkan dengan Kode QR"),
		"Enter this pairing code in WhatsApp on your phone:" => {
			Some("Masukkan kode penautan ini di WhatsApp pada ponsel Anda:")
		}
		"Back to Phone Number Login" => Some("Kembali ke Masuk dengan Nomor Telepon"),

		// Settings dialog
		"Settings" => Some("Setelan"),
		"General" => Some("Umum"),
		"Personalization" => Some("Personalisasi"),
		"Storage & Media" => Some("Penyimpanan & Media"),
		"Input & Output" => Some("Input & Output"),
		"Chat Lock" => Some("Kunci Chat"),
		"Backup and Restore" => Some("Cadangan dan Pemulihan"),
		"OK" => Some("Oke"),
		"Startup & Window" => Some("Mulai & Jendela"),
		"Start whatSnap when Windows starts" => Some("Mulai whatSnap saat Windows dimulai"),
		"Show an icon in the notification area; closing minimizes to it" => {
			Some("Tampilkan ikon di area notifikasi; menutup jendela akan meminimalkannya ke sana")
		}
		"Start maximized" => Some("Mulai dimaksimalkan"),
		"Accessibility & Announcements" => Some("Aksesibilitas & Pengumuman"),
		"Announce when someone is typing" => Some("Umumkan saat seseorang sedang mengetik"),
		"Announce when someone is recording a voice message" => {
			Some("Umumkan saat seseorang sedang merekam pesan suara")
		}
		"Show a button to message Meta AI in New Chat" => Some("Tampilkan tombol pesan Meta AI di Chat Baru"),
		"Updates & Troubleshooting" => Some("Pembaruan & Pemecahan Masalah"),
		"Check for updates when whatSnap starts" => Some("Periksa pembaruan saat whatSnap dimulai"),
		"Updates" => Some("Pembaruan"),
		"Stable releases" => Some("Rilis stabil"),
		"Development builds" => Some("Build pengembangan"),
		"Verbose backend logging, for troubleshooting connection issues (takes effect next time whatSnap starts)" => {
			Some("Log backend lengkap untuk mendiagnosis masalah koneksi (berlaku saat whatSnap dimulai ulang)")
		}
		"&Keyboard Shortcuts..." => Some("&Pintasan Keyboard..."),
		"Add whatSnap to the Windows Share Menu..." => Some("Tambahkan whatSnap ke Menu Bagikan Windows..."),
		"Security Notifications" => Some("Notifikasi Keamanan"),
		"Show security notifications: say in a chat when a contact's security code changes" => {
			Some("Tampilkan notifikasi keamanan: beri tahu di chat saat kode keamanan kontak berubah")
		}
		"WhatsApp protects your conversations with end-to-end encryption. Turn this on to get notified when a contact's security code changes." => {
			Some(
				"WhatsApp melindungi percakapan Anda dengan enkripsi ujung-ke-ujung. Aktifkan ini untuk mendapat notifikasi saat kode keamanan kontak berubah.",
			)
		}
		"Linked Accounts" => Some("Akun Terhubung"),
		"whatSnap is linked to WhatsApp on your phone as one of its linked devices. Accounts switches between, adds and removes accounts, and sets how each one alerts you. Log Out unlinks the account on screen from this computer. Both close Settings." => {
			Some(
				"whatSnap terhubung ke WhatsApp di ponsel Anda sebagai perangkat tertaut. Akun untuk beralih, menambah, atau menghapus akun serta mengatur notifikasinya. Keluar akan memutus tautan akun aktif dari komputer ini. Keduanya menutup Setelan.",
			)
		}
		"Accounts..." => Some("Akun..."),
		"Log Out..." => Some("Keluar..."),
		"Messaging & Navigation" => Some("Pesan & Navigasi"),
		"Enter is send: the Enter key sends your message" => Some("Enter untuk mengirim: tombol Enter mengirim pesan"),
		"Hide phone numbers in groups: show Unknown member for people with no saved name or profile name" => Some(
			"Sembunyikan nomor telepon di grup: tampilkan Anggota tidak dikenal jika tanpa nama kontak atau profil",
		),
		"Typing a letter in the message list starts a message (message shortcuts then need Alt)" => {
			Some("Mengetik huruf di daftar pesan langsung menulis pesan (pintasan pesan perlu Alt)")
		}
		"Keep my place in the message list instead of jumping to the newest message" => {
			Some("Pertahankan posisi di daftar pesan daripada langsung melompat ke pesan terbaru")
		}
		"Replace text with emoji as you type, like :-) or (y)" => {
			Some("Ganti teks menjadi emoji saat mengetik, seperti :-) atau (y)")
		}
		"Replace Text With Emoji: See Help..." => Some("Ganti Teks dengan Emoji: Lihat Bantuan..."),
		"Spelling" => Some("Ejaan"),
		"Check spelling in the message box (F7 goes to the next mistake)" => {
			Some("Periksa ejaan di kotak pesan (F7 melompat ke kesalahan berikutnya)")
		}
		"Say when a word you type is misspelled" => Some("Bacakan saat kata yang diketik salah eja"),
		"Spelling Language" => Some("Bahasa Pengecekan Ejaan"),
		"Windows default" => Some("Default Windows"),
		"Play next status update automatically" => Some("Putar pembaruan status berikutnya secara otomatis"),
		"Notifications" => Some("Notifikasi"),
		"Message notifications: show desktop notifications for new messages" => {
			Some("Notifikasi pesan: tampilkan notifikasi desktop untuk pesan baru")
		}
		"Show previews: message text in notifications" => Some("Tampilkan pratinjau: teks pesan dalam notifikasi"),
		"Reaction notifications: show notifications for reactions to your messages" => {
			Some("Notifikasi reaksi: tampilkan notifikasi untuk reaksi pada pesan Anda")
		}
		"Incoming sounds: play a sound for new messages" => Some("Suara masuk: putar suara untuk pesan baru"),
		"Message notification tone" => Some("Nada notifikasi pesan"),
		"Outgoing sounds: play sounds for messages you send" => Some("Suara keluar: putar suara untuk pesan terkirim"),
		"Group notifications: show notifications for new group messages" => {
			Some("Notifikasi grup: tampilkan notifikasi untuk pesan grup baru")
		}
		"Group sounds: play sounds for new group messages" => Some("Suara grup: putar suara untuk pesan grup baru"),
		"Group notification tone" => Some("Nada notifikasi grup"),
		"Call notifications: show notifications for incoming calls" => {
			Some("Notifikasi panggilan: tampilkan notifikasi untuk panggilan masuk")
		}
		"Incoming call sounds: play the ringtone for incoming calls" => {
			Some("Suara panggilan masuk: putar nada dering untuk panggilan masuk")
		}
		"Call ringtone" => Some("Nada dering panggilan"),
		"Group call ringtone" => Some("Nada dering panggilan grup"),
		"Quiet Hours & Screen Reader" => Some("Jam Tenang & Pembaca Layar"),
		"No message sound while Windows Do Not Disturb, a full-screen game or a presentation is on" => {
			Some("Tidak ada suara pesan saat Jangan Ganggu Windows, game layar penuh, atau presentasi aktif")
		}
		"Read new messages aloud in the open chat (Alt+L turns it on or off for one chat)" => {
			Some("Bacakan pesan baru di chat terbuka (Alt+L menyalakan atau mematikan per chat)")
		}
		"Also read your own sent messages aloud" => Some("Bacakan juga pesan yang Anda kirim sendiri"),
		"Theme" => Some("Tema"),
		"Choose Theme" => Some("Pilih Tema"),
		"Light" => Some("Terang"),
		"Dark" => Some("Gelap"),
		"System Default follows your Windows system-wide light or dark mode setting. Light and Dark themes provide dedicated color schemes." => {
			Some(
				"Default Sistem mengikuti mode terang atau gelap Windows. Tema Terang dan Gelap menyediakan skema warna khusus.",
			)
		}
		"App Language" => Some("Bahasa Aplikasi"),
		"Choose Language" => Some("Pilih Bahasa"),
		"System Default" => Some("Default Sistem"),
		"English" => Some("English"),
		"Bahasa Indonesia" => Some("Bahasa Indonesia"),
		"Choose the language for whatSnap's interface. Changes take full effect after restarting whatSnap." => {
			Some("Pilih bahasa antarmuka whatSnap. Perubahan berlaku sepenuhnya setelah whatSnap dimulai ulang.")
		}
		"Profile" => Some("Profil"),
		"Profile Photo" => Some("Foto Profil"),
		"Change Profile Photo..." => Some("Ubah Foto Profil..."),
		"Remove Profile Photo..." => Some("Hapus Foto Profil..."),
		"This is not your username or phone number. This name will be visible to your WhatsApp contacts." => {
			Some("Ini bukan nama pengguna atau nomor telepon Anda. Nama ini terlihat oleh kontak WhatsApp Anda.")
		}
		"About" => Some("Tentang"),
		"Clear About After" => Some("Hapus Info Tentang Setelah"),
		"Never" => Some("Jangan pernah"),
		"30 Minutes" => Some("30 Menit"),
		"1 Hour" => Some("1 Jam"),
		"2 Hours" => Some("2 Jam"),
		"4 Hours" => Some("4 Jam"),
		"8 Hours" => Some("8 Jam"),
		"24 Hours" => Some("24 Jam"),
		"7 Days" => Some("7 Hari"),
		"90 Days" => Some("90 Hari"),
		"Privacy" => Some("Privasi"),
		"Disappearing Messages" => Some("Pesan Sementara"),
		"Default Message Timer for New Chats" => Some("Timer Pesan Default untuk Chat Baru"),
		"Who Can See My Info" => Some("Siapa yang Dapat Melihat Info Saya"),
		"Who Can See My Last Seen" => Some("Siapa yang Dapat Melihat Terakhir Dilihat"),
		"Who Can See When I'm Online" => Some("Siapa yang Dapat Melihat Saat Saya Online"),
		"Who Can See My Profile Photo" => Some("Siapa yang Dapat Melihat Foto Profil Saya"),
		"Who Can See My About" => Some("Siapa yang Dapat Melihat Info Tentang Saya"),
		"Everyone" => Some("Semua orang"),
		"My Contacts" => Some("Kontak saya"),
		"My Contacts Except..." => Some("Kontak saya kecuali..."),
		"Nobody" => Some("Tidak seorang pun"),
		"Same as Last Seen" => Some("Sama seperti Terakhir Dilihat"),
		"Choose Contacts Excluded From Last Seen..." => Some("Pilih Kontak Dikecualikan dari Terakhir Dilihat..."),
		"Choose Contacts Excluded From Profile Photo..." => Some("Pilih Kontak Dikecualikan dari Foto Profil..."),
		"Choose Contacts Excluded From About..." => Some("Pilih Kontak Dikecualikan dari Info Tentang..."),
		"Choose Contacts Excluded From Groups..." => Some("Pilih Kontak Dikecualikan dari Grup..."),
		"Read receipts: if turned off, you won't send or receive read receipts. They're always sent in groups." => {
			Some(
				"Laporan dibaca: jika dimatikan, Anda tidak mengirim atau menerima laporan dibaca. Selalu dikirim di grup.",
			)
		}
		"Status Privacy" => Some("Privasi Status"),
		"Choose Who Can See My Status Updates..." => Some("Pilih Siapa yang Dapat Melihat Pembaruan Status..."),
		"Groups & Calls" => Some("Grup & Panggilan"),
		"Who Can Add Me to Groups" => Some("Siapa yang Dapat Menambahkan Saya ke Grup"),
		"Silence Unknown Callers" => Some("Bisukan Penelepon Tidak Dikenal"),
		"Advanced & Blocked Contacts" => Some("Lanjutan & Kontak Diblokir"),
		"Turn off link previews: links you send go without a preview, on all your devices" => {
			Some("Matikan pratinjau tautan: tautan yang Anda kirim tidak menampilkan pratinjau di semua perangkat")
		}
		"Manage Blocked Contacts..." => Some("Kelola Kontak Diblokir..."),
		"Media Auto-Download" => Some("Unduh Otomatis Media"),
		"Download photos automatically" => Some("Unduh foto secara otomatis"),
		"Download audio automatically" => Some("Unduh audio secara otomatis"),
		"Download videos automatically" => Some("Unduh video secara otomatis"),
		"Download documents automatically" => Some("Unduh dokumen secara otomatis"),
		"Voice messages are always downloaded automatically. Other media downloads by itself only from contacts and chats you've written in." => {
			Some(
				"Pesan suara selalu diunduh otomatis. Media lain hanya diunduh otomatis dari kontak dan chat yang pernah Anda kirim pesan.",
			)
		}
		"Media Quality" => Some("Kualitas Media"),
		"Download Quality for Photos and Videos Sent in HD" => {
			Some("Kualitas Unduhan untuk Foto dan Video yang Dikirim dalam HD")
		}
		"Media Upload Quality" => Some("Kualitas Unggahan Media"),
		"Standard Quality" => Some("Kualitas Standar"),
		"HD Quality" => Some("Kualitas HD"),
		"Microphone (calls and voice notes)" => Some("Mikrofon (panggilan dan pesan suara)"),
		"Used for voice/video calls and recording voice messages." => {
			Some("Digunakan untuk panggilan suara/video dan merekam pesan suara.")
		}
		"Clean up voice messages: reduce background noise and even out the volume" => {
			Some("Bersihkan pesan suara: kurangi kebisingan latar dan ratakan volume")
		}
		"Like WhatsApp Web: removes background noise and brings quiet speech up to a steady, loud level. Off sends the raw mic signal." => {
			Some(
				"Seperti WhatsApp Web: menghilangkan bising latar dan menaikkan suara pelan agar stabil. Mati mengirim sinyal mikrofon mentah.",
			)
		}
		"Speakers" => Some("Speaker"),
		"Speaker for alerts and voice messages" => Some("Speaker untuk peringatan dan pesan suara"),
		"Used for notification sounds, message tones and voice message playback." => {
			Some("Digunakan untuk suara notifikasi, nada pesan, dan pemutaran pesan suara.")
		}
		"Speaker for calls" => Some("Speaker untuk panggilan"),
		"Mute microphone by default when placing or joining calls" => {
			Some("Bisukan mikrofon secara default saat menelepon atau bergabung ke panggilan")
		}
		"When enabled, calls begin with your microphone muted. You can unmute at any time during the call." => Some(
			"Saat diaktifkan, panggilan dimulai dengan mikrofon dibisukan. Anda dapat membunyikannya kapan saja saat panggilan.",
		),
		"Show frequent contacts in the Calls tab" => Some("Tampilkan kontak yang sering dihubungi di tab Panggilan"),
		"When enabled, frequently called contacts are displayed at the top of the Calls tab." => {
			Some("Saat diaktifkan, kontak yang sering dihubungi ditampilkan di bagian atas tab Panggilan.")
		}
		"Screen Sharing" => Some("Berbagi Layar"),
		"Experimental: keep the camera on while sharing the screen (applies to the next call)" => {
			Some("Eksperimental: biarkan kamera tetap menyala saat berbagi layar (berlaku di panggilan berikutnya)")
		}
		"Unlock locked chats with Windows Hello (fingerprint, face or PIN) instead of the secret code" => Some(
			"Buka kunci chat dikunci dengan Windows Hello (sidik jari, wajah, atau PIN) sebagai pengganti kode rahasia",
		),
		"Allows unlocking hidden and locked chats using your Windows Hello biometric or PIN credentials." => {
			Some("Memungkinkan membuka chat tersembunyi dan terkunci memakai biometrik atau PIN Windows Hello.")
		}
		"Backup Settings" => Some("Cadangkan Setelan"),
		"Export your whatSnap preferences (shortcuts, notifications, audio settings, theme, and chat options) to a JSON file. Your chats, contacts, media, and account sessions are not included." => {
			Some(
				"Ekspor preferensi whatSnap Anda (pintasan, notifikasi, setelan audio, tema, dan opsi chat) ke file JSON. Chat, kontak, media, dan sesi akun tidak disertakan.",
			)
		}
		"Backup Settings..." => Some("Cadangkan Setelan..."),
		"Restore Settings" => Some("Pulihkan Setelan"),
		"Restore your whatSnap preferences from a previously saved JSON backup file. This will update your current settings." => {
			Some(
				"Pulihkan preferensi whatSnap Anda dari file cadangan JSON yang sebelumnya disimpan. Ini akan memperbarui setelan saat ini.",
			)
		}
		"Restore Settings..." => Some("Pulihkan Setelan..."),
		"Sounds" => Some("Suara"),
		"Keyboard Shortcuts" => Some("Pintasan Keyboard"),
		"Who Can See My Status" => Some("Siapa yang Dapat Melihat Status Saya"),
		"Read Receipts" => Some("Laporan Dibaca"),
		"Disappearing Messages Default Timer" => Some("Timer Default Pesan Sementara"),

		// Sound picker
		"Default" => Some("Default"),
		"Alert" => Some("Peringatan"),
		"Custom sound file" => Some("File suara khusus"),
		"None" => Some("Tidak ada"),
		"Choose a sound file" => Some("Pilih file suara"),
		"Choose Message Tone File..." => Some("Pilih File Nada Pesan..."),
		"Choose Group Tone File..." => Some("Pilih File Nada Grup..."),
		"Choose Call Ringtone File..." => Some("Pilih File Nada Dering Panggilan..."),
		"Choose Group Call Ringtone File..." => Some("Pilih File Nada Dering Panggilan Grup..."),
		"Choose Notification Tone File..." => Some("Pilih File Nada Notifikasi..."),
		"Choose Ringtone File..." => Some("Pilih File Nada Dering..."),
		"Choose Sound File..." => Some("Pilih File Suara..."),

		// Announcements & screen reader messages
		"Connecting..." => Some("Menghubungkan..."),
		"Connected." => Some("Terhubung."),
		"Reconnecting..." => Some("Menghubungkan ulang..."),
		"Reconnected." => Some("Terhubung kembali."),
		"Call connected." => Some("Panggilan terhubung."),
		"Call ended." => Some("Panggilan berakhir."),
		"That call has ended." => Some("Panggilan tersebut telah berakhir."),
		"Joining call..." => Some("Bergabung ke panggilan..."),
		"You're already in a call." => Some("Anda sudah berada dalam panggilan."),
		"Failed to start audio stream." => Some("Gagal memulai streaming audio."),
		"Microphone changed." => Some("Mikrofon diubah."),
		"Speaker changed." => Some("Speaker diubah."),
		"Camera changed." => Some("Kamera diubah."),
		"Audio and video settings opened." => Some("Setelan audio dan video dibuka."),
		"Audio and video settings closed." => Some("Setelan audio dan video ditutup."),
		"Stopped" => Some("Berhenti"),
		"Sending vote..." => Some("Mengirim pilihan..."),
		"Sending your answer..." => Some("Mengirim jawaban Anda..."),
		"Sending event..." => Some("Mengirim acara..."),
		"Sending sticker..." => Some("Mengirim stiker..."),
		"Added to favorite stickers." => Some("Ditambahkan ke stiker favorit."),
		"Removed from favorite stickers." => Some("Dihapus dari stiker favorit."),
		"Exporting chat text..." => Some("Mengekspor teks chat..."),
		"Posting status update..." => Some("Memposting pembaruan status..."),
		"Retrying send..." => Some("Mencoba mengirim ulang..."),
		"Status privacy saved." => Some("Privasi status disimpan."),
		"Saving who sees your status..." => Some("Menyimpan siapa yang melihat status Anda..."),
		"Choose at least one contact to share with." => Some("Pilih setidaknya satu kontak untuk berbagi."),
		"No answer chosen." => Some("Belum ada jawaban yang dipilih."),
		"Not a poll or event." => Some("Bukan polling atau acara."),
		"Type a reply" => Some("Ketik balasan"),

		"  Paused" => Some("  Dijeda"),
		"  Playing preview" => Some("  Memutar pratinjau"),
		" (Admin)" => Some(" (Admin)"),
		" (Super Admin)" => Some(" (Admin Super)"),
		" You liked this." => Some(" Anda menyukai ini."),
		"&Add chats..." => Some("&Tambah chat..."),
		"&Add list..." => Some("&Tambah daftar..."),
		"&Add to contacts" => Some("&Tambah ke kontak"),
		"&Audio" => Some("&Audio"),
		"&Call" => Some("&Telepon"),
		"&Chats..." => Some("&Chat..."),
		"&Contact" => Some("&Kontak"),
		"&Copy" => Some("&Salin"),
		"&Copy\tCtrl+C" => Some("&Salin\tCtrl+C"),
		"&Copy Code" => Some("&Salin Kode"),
		"&Copy Link" => Some("&Salin Tautan"),
		"&Copy Number" => Some("&Salin Nomor"),
		"&Copy text\tCtrl+C" => Some("&Salin teks\tCtrl+C"),
		"&Delete" => Some("&Hapus"),
		"&Delete Repeated Word" => Some("&Hapus Kata Berulang"),
		"&Delete call\tDel" => Some("&Hapus panggilan\tDel"),
		"&Delete channel..." => Some("&Hapus saluran..."),
		"&Delete..." => Some("&Hapus..."),
		"&Dismiss as admin" => Some("&Berhentikan sebagai admin"),
		"&Document" => Some("&Dokumen"),
		"&Edit" => Some("&Edit"),
		"&Edit name and description..." => Some("&Edit nama dan deskripsi..."),
		"&Event" => Some("A&cara"),
		"&Followers and admins..." => Some("&Pengikut dan admin..."),
		"&Forward..." => Some("&Teruskan..."),
		"&Go to message" => Some("&Buka pesan"),
		"&Insert emoji" => Some("&Sisipkan emoji"),
		"&Invite admins..." => Some("&Undang admin..."),
		"&Join call" => Some("&Gabung panggilan"),
		"&Keep in chat" => Some("&Simpan di chat"),
		"&Like" => Some("&Suka"),
		"&Message Number" => Some("&Kirim Pesan ke Nomor"),
		"&Mute" => Some("&Bisukan"),
		"&Next" => Some("&Berikutnya"),
		"&Notify normally" => Some("&Beri tahu secara normal"),
		"&Open Link" => Some("&Buka Tautan"),
		"&Open chat" => Some("&Buka chat"),
		"&Paste" => Some("&Tempel"),
		"&Photos and videos" => Some("&Foto dan video"),
		"&Position" => Some("&Posisi"),
		"&Previous" => Some("&Sebelumnya"),
		"&React..." => Some("&Reaksi..."),
		"&Remove" => Some("&Hapus"),
		"&Remove picture" => Some("&Hapus foto"),
		"&Rename..." => Some("&Ganti nama..."),
		"&Reply" => Some("&Balas"),
		"&Reply..." => Some("&Balas..."),
		"&Save voice message..." => Some("&Simpan pesan suara..."),
		"&Schedule message..." => Some("&Jadwalkan pesan..."),
		"&Send" => Some("&Kirim"),
		"&Send sticker" => Some("&Kirim stiker"),
		"&Silent" => Some("&Hening"),
		"&Star" => Some("&Bintang"),
		"&Star all" => Some("&Bintangi semua"),
		"&Sticker" => Some("&Stiker"),
		"&Undo" => Some("&Urungkan"),
		"&Unstar" => Some("&Hapus bintang"),
		"&Unstar\tDel" => Some("&Hapus bintang\tDel"),
		"&Unstar all" => Some("&Hapus bintang semua"),
		"&Verify security code" => Some("&Verifikasi kode keamanan"),
		"&Video call" => Some("&Panggilan video"),
		"&View status" => Some("&Lihat status"),
		"&Viewed by" => Some("&Dilihat oleh"),
		"&Viewed by..." => Some("&Dilihat oleh..."),
		"&Viewed by: no one yet" => Some("&Dilihat oleh: belum ada"),
		"&Volume" => Some("&Volume"),
		"(No spelling suggestions)" => Some("(Tidak ada saran ejaan)"),
		"(your reaction)" => Some("(reaksi Anda)"),
		", 1 mention" => Some(", 1 sebutan"),
		", animated" => Some(", animasi"),
		", copies a code" => Some(", menyalin kode"),
		", kept" => Some(", disimpan"),
		", link" => Some(", tautan"),
		", not linked yet" => Some(", belum ditautkan"),
		", notifications off" => Some(", notifikasi mati"),
		", on screen" => Some(", di layar"),
		", only works on your phone" => Some(", hanya berfungsi di ponsel Anda"),
		", pinned" => Some(", disematkan"),
		", starred" => Some(", berbintang"),
		"1 channel." => Some("1 saluran."),
		"1 follower" => Some("1 pengikut"),
		"1 sticker." => Some("1 stiker."),
		"1 unread message" => Some("1 pesan belum dibaca"),
		"< Back" => Some("< Kembali"),
		"A message that isn't on this computer" => Some("Pesan yang tidak ada di komputer ini"),
		"A poll can have at most 12 options." => Some("Polling maksimal memiliki 12 pilihan."),
		"A poll needs at least two options, one per line." => {
			Some("Polling membutuhkan setidaknya dua pilihan, satu per baris.")
		}
		"About this channel" => Some("Tentang saluran ini"),
		"About/Status" => Some("Info/Status"),
		"Accept" => Some("Terima"),
		"Accepting..." => Some("Menerima..."),
		"Account Already Linked" => Some("Akun Sudah Ditautkan"),
		"Actions for this message's sender" => Some("Tindakan untuk pengirim pesan ini"),
		"Add &photo/video status update..." => Some("Tambah pembaruan status &foto/video..."),
		"Add &text status update..." => Some("Tambah pembaruan status &teks..."),
		"Add Account" => Some("Tambah Akun"),
		"Add Contact" => Some("Tambah Kontak"),
		"Add Existing Group" => Some("Tambah Grup yang Ada"),
		"Add contact" => Some("Tambah kontak"),
		"Add existing group..." => Some("Tambah grup yang ada..."),
		"Add group members" => Some("Tambah anggota grup"),
		"Add member" => Some("Tambah anggota"),
		"Add to &favorite stickers" => Some("Tambah ke stiker &favorit"),
		"Add which group to the community?" => Some("Tambahkan grup mana ke komunitas?"),
		"Adding to call..." => Some("Menambahkan ke panggilan..."),
		"Advanced Chat Privacy" => Some("Privasi Chat Lanjutan"),
		"Advanced chat privacy..." => Some("Privasi chat lanjutan..."),
		"Alert exactly like an unarchived chat" => Some("Beri tahu persis seperti chat yang tidak diarsipkan"),
		"Album" => Some("Album"),
		"All (recommended)" => Some("Semua (disarankan)"),
		"All contacts" => Some("Semua kontak"),
		"All files" => Some("Semua file"),
		"All scheduled messages have been sent or deleted" => Some("Semua pesan terjadwal telah dikirim atau dihapus"),
		"All starred messages unstarred." => Some("Semua pesan berbintang telah dihapus bintangnya."),
		"Allow guests" => Some("Izinkan tamu"),
		"An option is too long. Each option allows up to 100 characters." => {
			Some("Pilihan terlalu panjang. Setiap pilihan maksimal 100 karakter.")
		}
		"Announcements" => Some("Pengumuman"),
		"Answer event" => Some("Jawab acara"),
		"Answer quiz" => Some("Jawab kuis"),
		"Approve" => Some("Setujui"),
		"Approve new members" => Some("Setujui anggota baru"),
		"Archived" => Some("Diarsipkan"),
		"Archived a&lerts" => Some("Pemberitahuan yang di&arsipkan"),
		"Ask your phone to send this message again" => Some("Minta telepon Anda untuk mengirim pesan ini lagi"),
		"At least one contact must be selected." => Some("Minimal satu kontak harus dipilih."),
		"At least one participant must be selected." => Some("Minimal satu peserta harus dipilih."),
		"Attach a contact from your WhatsApp contacts" => Some("Lampirkan kontak dari kontak WhatsApp Anda"),
		"Attach contact" => Some("Lampirkan kontak"),
		"Audio" => Some("Audio"),
		"Availability" => Some("Ketersediaan"),
		"Backup Complete" => Some("Pencadangan Selesai"),
		"Backup Failed" => Some("Pencadangan Gagal"),
		"Block contact" => Some("Blokir kontak"),
		"Block contact..." => Some("Blokir kontak..."),
		"Blocked contact" => Some("Kontak diblokir"),
		"Blocked contacts: none" => Some("Kontak yang diblokir: tidak ada"),
		"Blocked: Yes\n" => Some("Diblokir: Ya\n"),
		"Blocked: yes" => Some("Diblokir: ya"),
		"Busy, on another call with" => Some("Sibuk, sedang dalam panggilan lain dengan"),
		"Busy, on another call." => Some("Sedang sibuk, dalam panggilan lain."),
		"Buttons" => Some("Tombol"),
		"C&hannel post" => Some("Postingan sa&luran"),
		"C&lear call log..." => Some("Be&rsihkan log panggilan..."),
		"Call Link" => Some("Tautan Panggilan"),
		"Call cancelled with" => Some("Panggilan dibatalkan dengan"),
		"Call cancelled." => Some("Panggilan dibatalkan."),
		"Call declined with" => Some("Panggilan ditolak dengan"),
		"Call declined." => Some("Panggilan ditolak."),
		"Call deleted." => Some("Panggilan dihapus."),
		"Call ended" => Some("Panggilan berakhir"),
		"Call ended with" => Some("Panggilan berakhir dengan"),
		"Call with" => Some("Panggilan dengan"),
		"Call ended. Connection lost with" => Some("Panggilan berakhir. Koneksi terputus dengan"),
		"Call ended. Connection lost." => Some("Panggilan berakhir. Koneksi terputus."),
		"Call link:" => Some("Tautan panggilan:"),
		"Call log cleared." => Some("Log panggilan dibersihkan."),
		"Calling" => Some("Memanggil"),
		"Calling..." => Some("Memanggil..."),
		"Camera must be turned on to share your screen. Turn your camera on first." => {
			Some("Kamera harus menyala untuk berbagi layar. Nyalakan kamera Anda terlebih dahulu.")
		}
		"Camera off." => Some("Kamera mati."),
		"Camera off. Screen sharing stopped." => Some("Kamera mati. Berbagi layar dihentikan."),
		"Camera on." => Some("Kamera nyala."),
		"Can't connect to WhatsApp." => Some("Tidak dapat terhubung ke WhatsApp."),
		"Cancel invite" => Some("Batalkan undangan"),
		"Cancel invite..." => Some("Batalkan undangan..."),
		"Cancel this upload? The message will be removed from the chat." => {
			Some("Batalkan unggahan ini? Pesan akan dihapus dari chat.")
		}
		"Cancel upload" => Some("Batalkan unggahan"),
		"Cancel upload, 0% complete" => Some("Batalkan unggahan, 0% selesai"),
		"Catalogue" => Some("Katalog"),
		"Category:" => Some("Kategori:"),
		"Change &picture..." => Some("Ubah &foto..."),
		"Change owner" => Some("Ubah pemilik"),
		"Change the channel's name and description" => Some("Ubah nama dan deskripsi saluran"),
		"Change the tag shown next to your name in this group" => {
			Some("Ubah label yang ditampilkan di samping nama Anda di grup ini")
		}
		"Changing the channel picture..." => Some("Mengubah foto saluran..."),
		"Channel" => Some("Saluran"),
		"Channel admin invite" => Some("Undangan admin saluran"),
		"Channel info" => Some("Info saluran"),
		"Channel name" => Some("Nama saluran"),
		"Channels are read-only. Only admins can post updates." => {
			Some("Saluran bersifat hanya-baca. Hanya admin yang dapat memposting pembaruan.")
		}
		"Channels are read-only. Only admins can post." => {
			Some("Saluran bersifat hanya-baca. Hanya admin yang dapat memposting.")
		}
		"Channels show vote totals, not who voted. Admins can see voters from the Channel post menu." => Some(
			"Saluran hanya menampilkan total suara, bukan siapa yang memilih. Admin dapat melihat pemilih dari menu kiriman Saluran.",
		),
		"Chat cleared." => Some("Chat dibersihkan."),
		"Chat history" => Some("Riwayat chat"),
		"Chat overview" => Some("Ringkasan chat"),
		"Check the chats to add" => Some("Centang chat yang ingin ditambahkan"),
		"Checked contacts won't be able to see it" => Some("Kontak yang dicentang tidak akan dapat melihatnya"),
		"Checked contacts won't see your status updates" => {
			Some("Kontak yang dicentang tidak akan melihat pembaruan status Anda")
		}
		"Choose a folder to save the files into" => Some("Pilih folder untuk menyimpan file"),
		"Choose a new picture for the channel" => Some("Pilih foto baru untuk saluran"),
		"Choose a photo or video to post as status" => Some("Pilih foto atau video untuk diposting sebagai status"),
		"Choose a picture for the channel" => Some("Pilih foto untuk saluran"),
		"Choose a profile photo" => Some("Pilih foto profil"),
		"Choose a sticker to send" => Some("Pilih stiker untuk dikirim"),
		"Choose an emoji to insert:" => Some("Pilih emoji untuk disisipkan:"),
		"Choose at least one chat to send to." => Some("Pilih setidaknya satu chat tujuan."),
		"Choose audio files to send" => Some("Pilih file audio untuk dikirim"),
		"Choose files to send as documents" => Some("Pilih file untuk dikirim sebagai dokumen"),
		"Choose one answer" => Some("Pilih satu jawaban"),
		"Choose one or more answers" => Some("Pilih satu jawaban atau lebih"),
		"Choose photos or videos to send" => Some("Pilih foto atau video untuk dikirim"),
		"Choose picture (optional)..." => Some("Pilih foto (opsional)..."),
		"Choose what kind of status update to post:" => Some("Pilih jenis pembaruan status yang akan diposting:"),
		"Choose who sees your status updates" => Some("Pilih siapa yang dapat melihat pembaruan status Anda"),
		"Clear" => Some("Bersihkan"),
		"Clear call log?" => Some("Bersihkan log panggilan?"),
		"Code copied." => Some("Kode disalin."),
		"Community" => Some("Komunitas"),
		"Community deactivated." => Some("Komunitas dinonaktifkan."),
		"Community name:" => Some("Nama komunitas:"),
		"Compare the security code with this member to check your chat is end-to-end encrypted" => Some(
			"Bandingkan kode keamanan dengan anggota ini untuk memeriksa apakah chat Anda terenkripsi secara menyeluruh",
		),
		"Compose a new text status update" => Some("Tulis pembaruan status teks baru"),
		"Confirm Clear Chat" => Some("Konfirmasi Bersihkan Chat"),
		"Confirm Delete" => Some("Konfirmasi Hapus"),
		"Connected" => Some("Terhubung"),
		"Contact" => Some("Kontak"),
		"Contact Details" => Some("Detail Kontak"),
		"Contact Info" => Some("Info Kontak"),
		"Contact blocked." => Some("Kontak diblokir."),
		"Contact deleted." => Some("Kontak dihapus."),
		"Copied file to clipboard." => Some("File disalin ke papan klip."),
		"Copied message text to clipboard." => Some("Teks pesan disalin ke papan klip."),
		"Copied phone number to clipboard." => Some("Nomor telepon disalin ke papan klip."),
		"Copied voice message to clipboard." => Some("Pesan suara disalin ke papan klip."),
		"Copy Nu&mber" => Some("Salin No&mor"),
		"Copy all p&hotos" => Some("Salin semua f&oto"),
		"Copy all photos to clipboard" => Some("Salin semua foto ke papan klip"),
		"Copy code" => Some("Salin kode"),
		"Copy link" => Some("Salin tautan"),
		"Copy message text or file to clipboard" => Some("Salin teks pesan atau file ke papan klip"),
		"Copy message text to clipboard" => Some("Salin teks pesan ke papan klip"),
		"Copy phone number to clipboard" => Some("Salin nomor telepon ke papan klip"),
		"Could not load voice message" => Some("Tidak dapat memuat pesan suara"),
		"Could not play voice message" => Some("Tidak dapat memutar pesan suara"),
		"Couldn't add that person to the call." => Some("Tidak dapat menambahkan orang tersebut ke panggilan."),
		"Couldn't copy the code." => Some("Tidak dapat menyalin kode."),
		"Couldn't copy the link." => Some("Tidak dapat menyalin tautan."),
		"Couldn't copy to clipboard." => Some("Tidak dapat menyalin ke papan klip."),
		"Couldn't send the message history." => Some("Tidak dapat mengirim riwayat pesan."),
		"Couldn't share the screen on this call." => Some("Tidak dapat membagikan layar pada panggilan ini."),
		"Couldn't turn the camera on for this call." => Some("Tidak dapat menyalakan kamera untuk panggilan ini."),
		"Couldn't work out when this message can be scheduled." => {
			Some("Tidak dapat menentukan kapan pesan ini dapat dijadwalkan.")
		}
		"Create Call Link" => Some("Buat Tautan Panggilan"),
		"Create a poll or a quiz" => Some("Buat jajak pendapat atau kuis"),
		"Create an event people can answer" => Some("Buat acara yang dapat ditanggapi orang lain"),
		"Create event" => Some("Buat acara"),
		"Creating channel..." => Some("Membuat saluran..."),
		"Creating community..." => Some("Membuat komunitas..."),
		"Cu&t" => Some("Po&tong"),
		"D&elete" => Some("&Hapus"),
		"Date to go to, as year-month-day, like 2026-03-27:" => {
			Some("Buka tanggal, format tahun-bulan-hari, contoh 2026-03-27:")
		}
		"Date, like 2026-10-05 (optional)" => Some("Tanggal, seperti 2026-10-05 (opsional)"),
		"Deactivate" => Some("Nonaktifkan"),
		"Deactivate Community" => Some("Nonaktifkan Komunitas"),
		"Deactivate community..." => Some("Nonaktifkan komunitas..."),
		"Delete &contact..." => Some("Hapus &kontak..."),
		"Delete Contact" => Some("Hapus Kontak"),
		"Delete List" => Some("Hapus Daftar"),
		"Delete Status" => Some("Hapus Status"),
		"Delete channel" => Some("Hapus saluran"),
		"Delete chat and exit group" => Some("Hapus chat dan keluar grup"),
		"Delete chat only" => Some("Hapus chat saja"),
		"Delete contact" => Some("Hapus kontak"),
		"Delete for everyone" => Some("Hapus untuk semua orang"),
		"Delete for me" => Some("Hapus untuk saya"),
		"Delete message" => Some("Hapus pesan"),
		"Delete message?" => Some("Hapus pesan?"),
		"Delete messages?" => Some("Hapus pesan?"),
		"Delete one of your status updates for everyone" => {
			Some("Hapus salah satu pembaruan status Anda untuk semua orang")
		}
		"Delete scheduled message?" => Some("Hapus pesan terjadwal?"),
		"Delete the channel for everyone" => Some("Hapus saluran untuk semua orang"),
		"Delete the word said twice" => Some("Hapus kata yang berulang"),
		"Delete this call from your call log" => Some("Hapus panggilan ini dari log panggilan Anda"),
		"Delete this channel? It will be removed for all its followers, along with all its updates. This can't be undone." => {
			Some(
				"Hapus saluran ini? Saluran akan dihapus untuk semua pengikutnya beserta semua pembaruannya. Tindakan ini tidak dapat dibatalkan.",
			)
		}
		"Delete this contact from your contacts and your phone's" => {
			Some("Hapus kontak ini dari kontak Anda dan kontak ponsel Anda")
		}
		"Delete this status update? It will be deleted for everyone who received it." => {
			Some("Hapus pembaruan status ini? Status akan dihapus untuk semua orang yang menerimanya.")
		}
		"Delete which status update?" => Some("Hapus pembaruan status yang mana?"),
		"Deleting channel..." => Some("Menghapus saluran..."),
		"Deleting contact..." => Some("Menghapus kontak..."),
		"Deleting scheduled message..." => Some("Menghapus pesan terjadwal..."),
		"Deleting..." => Some("Menghapus..."),
		"Delivered" => Some("Tersampaikan"),
		"Delivered at" => Some("Tersampaikan pukul"),
		"Delivered to" => Some("Tersampaikan ke"),
		"Description (optional)" => Some("Deskripsi (opsional)"),
		"Dismiss admin" => Some("Berhentikan admin"),
		"Document" => Some("Dokumen"),
		"Documents" => Some("Dokumen"),
		"Don't allow messages forwarded many times" => Some("Jangan izinkan pesan yang diteruskan berkali-kali"),
		"Done" => Some("Selesai"),
		"Download cancelled" => Some("Unduhan dibatalkan"),
		"Downloading file before copying to clipboard..." => Some("Mengunduh file sebelum menyalin ke clipboard..."),
		"Draft: " => Some("Draf: "),
		"Duration: " => Some("Durasi: "),
		"Edit Contact" => Some("Edit Kontak"),
		"Edit contact" => Some("Edit kontak"),
		"Edit member &tag" => Some("Edit label &anggota"),
		"Edit message" => Some("Edit pesan"),
		"Edited " => Some("Diedit "),
		"Edited at" => Some("Diedit pukul"),
		"Editing cancelled - message can't be empty." => Some("Edit dibatalkan - pesan tidak boleh kosong."),
		"Editing cancelled - no changes made." => Some("Edit dibatalkan - tidak ada perubahan."),
		"Editing cancelled." => Some("Edit dibatalkan."),
		"Editing message. Press Enter to save, Escape to cancel." => {
			Some("Mengedit pesan. Tekan Enter untuk menyimpan, Escape untuk membatalkan.")
		}
		"Emoji" => Some("Emoji"),
		"End" => Some("Akhir"),
		"End the call before switching accounts." => Some("Akhiri panggilan sebelum beralih akun."),
		"Enter or paste an emoji:" => Some("Masukkan atau tempel emoji:"),
		"Enter your WhatsApp Secret Code configured on your phone:" => {
			Some("Masukkan Kode Rahasia WhatsApp yang dikonfigurasi di ponsel Anda:")
		}
		"Entire screen" => Some("Seluruh layar"),
		"Error" => Some("Kesalahan"),
		"Event" => Some("Acara"),
		"Event name" => Some("Nama acara"),
		"Exit fullscreen" => Some("Keluar dari layar penuh"),
		"Export chat as" => Some("Ekspor chat sebagai"),
		"Export chat text as" => Some("Ekspor teks chat sebagai"),
		"Exporting chat, this may take a while for chats with a lot of media..." => {
			Some("Mengekspor chat, ini mungkin perlu waktu untuk chat dengan banyak media...")
		}
		"Failed to answer call." => Some("Gagal menjawab panggilan."),
		"Failed to copy file to clipboard." => Some("Gagal menyalin file ke clipboard."),
		"Failed to parse settings" => Some("Gagal membaca setelan"),
		"Failed to place call." => Some("Gagal melakukan panggilan."),
		"Failed to resume recording." => Some("Gagal melanjutkan perekaman."),
		"Failed to send" => Some("Gagal mengirim"),
		"Failed to start microphone recording." => Some("Gagal memulai perekaman mikrofon."),
		"File" => Some("File"),
		"Files" => Some("File"),
		"Files attached" => Some("File dilampirkan"),
		"Find channels" => Some("Cari saluran"),
		"Follow" => Some("Ikuti"),
		"Forward Message" => Some("Teruskan Pesan"),
		"Forward message to" => Some("Teruskan pesan ke"),
		"Forward message to another contact" => Some("Teruskan pesan ke kontak lain"),
		"Forward messages to" => Some("Teruskan pesan ke"),
		"Forward this message to contacts or groups" => Some("Teruskan pesan ini ke kontak atau grup"),
		"Frequent contacts" => Some("Kontak yang sering dihubungi"),
		"Fullscreen" => Some("Layar penuh"),
		"Getting a fresh copy from the poster's phone. It will show when it arrives." => {
			Some("Mengambil salinan terbaru dari ponsel pengirim. Status akan ditampilkan saat tiba.")
		}
		"Give the channel a name." => Some("Beri nama saluran."),
		"Go to Date" => Some("Buka Tanggal"),
		"Go to a pinned message:" => Some("Buka pesan tersemat:"),
		"Go to message" => Some("Buka pesan"),
		"Going on now" => Some("Sedang berlangsung"),
		"Group" => Some("Grup"),
		"Group Info" => Some("Info Grup"),
		"Group Invite" => Some("Undangan Grup"),
		"Group added to the community." => Some("Grup ditambahkan ke komunitas."),
		"Group description" => Some("Deskripsi grup"),
		"Group invite link" => Some("Tautan undangan grup"),
		"Group name is required." => Some("Nama grup wajib diisi."),
		"Group removed from the community." => Some("Grup dikeluarkan dari komunitas."),
		"Group settings updated." => Some("Setelan grup diperbarui."),
		"Group video call" => Some("Panggilan video grup"),
		"Group voice call" => Some("Panggilan suara grup"),
		"Group:" => Some("Grup:"),
		"Groups in Common" => Some("Grup Bersama"),
		"Groups in common" => Some("Grup bersama"),
		"Groups in this community" => Some("Grup dalam komunitas ini"),
		"Guests you're bringing" => Some("Tamu yang Anda bawa"),
		"Hide device settings" => Some("Sembunyikan setelan perangkat"),
		"Hide message content: notifications from this chat don't show what the message says" => {
			Some("Sembunyikan isi pesan: notifikasi dari chat ini tidak akan menampilkan isi pesan")
		}
		"How this archived chat alerts you about new messages" => {
			Some("Cara chat yang diarsipkan ini memberi tahu Anda tentang pesan baru")
		}
		"How this computer alerts you about this chat, without changing your phone" => {
			Some("Cara komputer ini memberi tahu Anda tentang chat ini, tanpa mengubah setelan di telepon Anda")
		}
		"In group call with" => Some("Dalam panggilan grup dengan"),
		"In group call. No one else is here yet." => Some("Dalam panggilan grup. Belum ada orang lain di sini."),
		"Incoming call" => Some("Panggilan masuk"),
		"Incoming call from" => Some("Panggilan masuk dari"),
		"Incoming group call from" => Some("Panggilan grup masuk dari"),
		"Incoming group video call from" => Some("Panggilan video grup masuk dari"),
		"Incoming video call from" => Some("Panggilan video masuk dari"),
		"Incoming voice call from" => Some("Panggilan suara masuk dari"),
		"Incorrect Secret Code.\n\nPlease check your secret code and try again." => {
			Some("Kode Rahasia salah.\n\nSilakan periksa kode rahasia Anda lalu coba lagi.")
		}
		"Insert Emoji" => Some("Sisipkan Emoji"),
		"Invalid JSON" => Some("JSON tidak valid"),
		"Invalid Settings File" => Some("File Setelan Tidak Valid"),
		"Invite admins" => Some("Undang admin"),
		"Invite link" => Some("Tautan undangan"),
		"Invite link..." => Some("Tautan undangan..."),
		"Invite people to be admins of the channel" => Some("Undang orang untuk menjadi admin saluran"),
		"Item code" => Some("Kode item"),
		"Items" => Some("Item"),
		"JSON files (*.json)|*.json|All files (*.*)|*.*" => Some("File JSON (*.json)|*.json|Semua file (*.*)|*.*"),
		"Join Call Link" => Some("Gabung Tautan Panggilan"),
		"Join Request" => Some("Permintaan Bergabung"),
		"Join Requests" => Some("Permintaan Bergabung"),
		"Join group" => Some("Gabung grup"),
		"Join requests..." => Some("Permintaan bergabung..."),
		"Join this group call" => Some("Gabung panggilan grup ini"),
		"Joining group..." => Some("Bergabung ke grup..."),
		"Keep this message from disappearing, for everyone" => Some("Cegah pesan ini hilang, untuk semua orang"),
		"Keep this message pinned for:" => Some("Biarkan pesan ini disematkan selama:"),
		"Keep this sticker in your favorites to send it again" => {
			Some("Simpan stiker ini di favorit Anda untuk dikirim lagi")
		}
		"Leave Group" => Some("Keluar dari Grup"),
		"Leave call" => Some("Tinggalkan panggilan"),
		"Let this member change group info and settings and manage members" => {
			Some("Izinkan anggota ini mengubah info serta setelan grup dan mengelola anggota")
		}
		"Let this message disappear like the others" => Some("Biarkan pesan ini hilang seperti yang lainnya"),
		"Like removed." => Some("Suka dihapus."),
		"Liked." => Some("Disukai."),
		"Link copied." => Some("Tautan disalin."),
		"Linked groups" => Some("Grup tertaut"),
		"Links" => Some("Tautan"),
		"List name:" => Some("Nama daftar:"),
		"Lists" => Some("Daftar"),
		"Live location" => Some("Lokasi terkini"),
		"Loading catalogue..." => Some("Memuat katalog..."),
		"Loading followers..." => Some("Memuat pengikut..."),
		"Loading pending admin invites..." => Some("Memuat undangan admin yang tertunda..."),
		"Loading status privacy settings..." => Some("Memuat setelan privasi status..."),
		"Loading voice message..." => Some("Memuat pesan suara..."),
		"Loading..." => Some("Memuat..."),
		"Location" => Some("Lokasi"),
		"Location (optional)" => Some("Lokasi (opsional)"),
		"Lock this chat" => Some("Kunci chat ini"),
		"Locked" => Some("Dikunci"),
		"Locked chats hidden." => Some("Chat dikunci disembunyikan."),
		"Locked chats shown." => Some("Chat dikunci ditampilkan."),
		"Log Out" => Some("Keluar"),
		"Log out of WhatsApp on this computer? You'll need to scan a QR code or re-enter your phone number to use whatSnap again." => {
			Some(
				"Keluar dari WhatsApp di komputer ini? Anda harus memindai kode QR atau memasukkan ulang nomor telepon Anda untuk menggunakan whatSnap lagi.",
			)
		}
		"Logged Out" => Some("Keluar Akun"),
		"Looking for chats, contacts or messages\u{2026}" => Some("Mencari chat, kontak, atau pesan…"),
		"Lower hand" => Some("Turunkan tangan"),
		"Make Group Admin" => Some("Jadikan Admin Grup"),
		"Make admin" => Some("Jadikan admin"),
		"Make admin..." => Some("Jadikan admin..."),
		"Make group &admin" => Some("Jadikan &admin grup"),
		"Make group admin" => Some("Jadikan admin grup"),
		"Make owner..." => Some("Jadikan pemilik..."),
		"Make this admin an ordinary member again" => Some("Jadikan admin ini sebagai anggota biasa kembali"),
		"Manage Lists" => Some("Kelola Daftar"),
		"Manage channel..." => Some("Kelola saluran..."),
		"Markdown file (*.md)|*.md" => Some("File Markdown (*.md)|*.md"),
		"Media" => Some("Media"),
		"Member changes" => Some("Perubahan anggota"),
		"Member changes, newest first" => Some("Perubahan anggota, terbaru lebih dahulu"),
		"Members" => Some("Anggota"),
		"Members can report messages to admins" => Some("Anggota dapat melaporkan pesan ke admin"),
		"Members who join after you schedule a message will not receive it." => {
			Some("Anggota yang bergabung setelah Anda menjadwalkan pesan tidak akan menerimanya.")
		}
		"Message &Info" => Some("&Info Pesan"),
		"Message Info" => Some("Info Pesan"),
		"Message Num&ber" => Some("Kirim Pesan ke No&mor"),
		"Message Text" => Some("Teks Pesan"),
		"Message buttons" => Some("Tombol pesan"),
		"Message can't be edited." => Some("Pesan tidak dapat diedit."),
		"Message can't be starred." => Some("Pesan tidak dapat dibintangi."),
		"Message deleted." => Some("Pesan dihapus."),
		"Message edited." => Some("Pesan diedit."),
		"Message forwarded." => Some("Pesan diteruskan."),
		"Message kept." => Some("Pesan disimpan."),
		"Message pinned." => Some("Pesan disematkan."),
		"Message starred." => Some("Pesan dibintangi."),
		"Message text copied to clipboard." => Some("Teks pesan disalin ke papan klip."),
		"Message unkept." => Some("Penyimpanan pesan dibatalkan."),
		"Message unpinned." => Some("Sematan pesan dilepas."),
		"Message unstarred." => Some("Bintang pesan dihapus."),
		"Messages can only be scheduled in one-on-one chats and groups." => {
			Some("Pesan hanya dapat dijadwalkan di chat pribadi dan grup.")
		}
		"Messages members reported to the admins" => Some("Pesan yang dilaporkan anggota ke admin"),
		"More channels" => Some("Saluran lainnya"),
		"Mute notifications from this chat" => Some("Bisukan notifikasi dari chat ini"),
		"Muted" => Some("Dibisukan"),
		"Muted status updates are listed last" => Some("Pembaruan status yang dibisukan ditampilkan paling bawah"),
		"Muted updates" => Some("Pembaruan yang dibisukan"),
		"My status" => Some("Status saya"),
		"My status: No updates. Click or press Enter to add" => {
			Some("Status saya: Tidak ada pembaruan. Klik atau tekan Enter untuk menambahkan")
		}
		"Name for this account in whatSnap (leave empty to use its WhatsApp name):" => {
			Some("Nama untuk akun ini di whatSnap (biarkan kosong untuk memakai nama WhatsApp):")
		}
		"Name is required." => Some("Nama wajib diisi."),
		"Never mark this word again" => Some("Jangan tandai kata ini lagi"),
		"New" => Some("Baru"),
		"New Community" => Some("Komunitas Baru"),
		"New List" => Some("Daftar Baru"),
		"New List..." => Some("Daftar Baru..."),
		"New account" => Some("Akun baru"),
		"New message" => Some("Pesan baru"),
		"New reaction" => Some("Reaksi baru"),
		"New, muted" => Some("Baru, dibisukan"),
		"No Secret Code configured.\n\nTo lock or unlock chats on linked devices, you must first create a Secret Code in WhatsApp on your primary phone:\n\n1. Open WhatsApp on your phone\n2. Go to Settings > Privacy > Chat Lock\n3. Tap 'Secret Code' and set a code.\n\nOnce set, it will sync to this device automatically." => {
			Some(
				"Tidak ada Kode Rahasia yang dikonfigurasikan.\n\nUntuk mengunci atau membuka kunci chat di perangkat tertaut, Anda harus membuat Kode Rahasia terlebih dahulu di WhatsApp pada ponsel utama Anda:\n\n1. Buka WhatsApp di ponsel Anda\n2. Buka Pengaturan > Privasi > Kunci Chat\n3. Ketuk 'Kode Rahasia' dan atur sebuah kode.\n\nSetelah diatur, kode akan disinkronkan ke perangkat ini secara otomatis.",
			)
		}
		"No active call." => Some("Tidak ada panggilan aktif."),
		"No answer with" => Some("Tidak ada jawaban dari"),
		"No answer." => Some("Tidak ada jawaban."),
		"No archived chats" => Some("Tidak ada chat yang diarsipkan"),
		"No channels" => Some("Tidak ada saluran"),
		"No channels in this category." => Some("Tidak ada saluran dalam kategori ini."),
		"No chat open." => Some("Tidak ada chat yang terbuka."),
		"No chats, contacts or messages found" => Some("Tidak ada chat, kontak, atau pesan yang ditemukan"),
		"No emoji found." => Some("Tidak ada emoji yang ditemukan."),
		"No favorite stickers yet." => Some("Belum ada stiker favorit."),
		"No favourite chats" => Some("Tidak ada chat favorit"),
		"No groups found" => Some("Tidak ada grup yang ditemukan"),
		"No groups in common" => Some("Tidak ada grup bersama"),
		"No locked chats found" => Some("Tidak ada chat dikunci yang ditemukan"),
		"No messages are pinned in this chat." => Some("Tidak ada pesan yang disematkan di chat ini."),
		"No messages have been reported to the admins." => Some("Tidak ada pesan yang dilaporkan ke admin."),
		"No more channels." => Some("Tidak ada saluran lagi."),
		"No one else is here yet." => Some("Belum ada orang lain di sini."),
		"No one has joined, left or been added or removed." => {
			Some("Tidak ada yang bergabung, keluar, ditambahkan, atau dikeluarkan.")
		}
		"No one has seen this status update yet." => Some("Belum ada yang melihat pembaruan status ini."),
		"No one is asking to join." => Some("Tidak ada yang meminta untuk bergabung."),
		"No receipt information available." => Some("Tidak ada informasi tanda terima yang tersedia."),
		"No recent stickers." => Some("Tidak ada stiker terkini."),
		"No recent updates from contacts" => Some("Tidak ada pembaruan terkini dari kontak"),
		"No recommended channels." => Some("Tidak ada rekomendasi saluran."),
		"No response - the call could not be answered." => Some("Tidak ada respons - panggilan tidak dapat dijawab."),
		"No results" => Some("Tidak ada hasil"),
		"No results found" => Some("Tidak ada hasil yang ditemukan"),
		"No similar channels found." => Some("Tidak ada saluran serupa yang ditemukan."),
		"No sound and no notification for this chat" => Some("Tanpa suara dan tanpa notifikasi untuk chat ini"),
		"No sound is reaching you from the call. Check your internet connection." => {
			Some("Tidak ada suara dari panggilan yang sampai ke Anda. Periksa koneksi internet Anda.")
		}
		"No spelling errors" => Some("Tidak ada kesalahan ejaan"),
		"No starred messages" => Some("Tidak ada pesan berbintang"),
		"No text to copy." => Some("Tidak ada teks untuk disalin."),
		"No unread chats" => Some("Tidak ada chat yang belum dibaca"),
		"No votes yet." => Some("Belum ada suara."),
		"Not available" => Some("Tidak tersedia"),
		"Not set" => Some("Belum diatur"),
		"Notification se&ttings..." => Some("Se&telan notifikasi..."),
		"Notification tone" => Some("Nada notifikasi"),
		"Only admins can add members" => Some("Hanya admin yang dapat menambah anggota"),
		"Only admins can edit group info" => Some("Hanya admin yang dapat mengedit info grup"),
		"Only admins can send messages" => Some("Hanya admin yang dapat mengirim pesan"),
		"Only admins can send messages to this group" => Some("Hanya admin yang dapat mengirim pesan ke grup ini"),
		"Only admins can send messages to this group." => Some("Hanya admin yang dapat mengirim pesan ke grup ini."),
		"Only checked contacts will see your status updates" => {
			Some("Hanya kontak yang dicentang yang akan melihat pembaruan status Anda")
		}
		"Only contacts can be blocked." => Some("Hanya kontak yang dapat diblokir."),
		"Open &Website" => Some("Buka &Situs Web"),
		"Open Chat" => Some("Buka Chat"),
		"Open Link" => Some("Buka Tautan"),
		"Open a one-on-one chat with this message's sender" => Some("Buka chat pribadi dengan pengirim pesan ini"),
		"Open album" => Some("Buka album"),
		"Open c&hat" => Some("Buka c&hat"),
		"Open channel" => Some("Buka saluran"),
		"Open group" => Some("Buka grup"),
		"Open in &another app" => Some("Buka di aplikasi &lain"),
		"Open the chat at this message" => Some("Buka chat pada pesan ini"),
		"Open the conversation with this contact" => Some("Buka percakapan dengan kontak ini"),
		"Open the group this call is in" => Some("Buka grup tempat panggilan ini berlangsung"),
		"Open your chat with this member" => Some("Buka chat Anda dengan anggota ini"),
		"Order" => Some("Pesanan"),
		"Order details" => Some("Detail pesanan"),
		"Other emoji..." => Some("Emoji lainnya..."),
		"Outgoing call to" => Some("Panggilan keluar ke"),
		"Outgoing group call in" => Some("Panggilan grup keluar di"),
		"Outgoing group video call in" => Some("Panggilan video grup keluar di"),
		"Outgoing video call to" => Some("Panggilan video keluar ke"),
		"Overview" => Some("Ringkasan"),
		"P&oll" => Some("&Jajak Pendapat"),
		"Pa&use" => Some("&Jeda"),
		"Pasted files" => Some("File yang ditempel"),
		"Pause" => Some("Jeda"),
		"Pause preview" => Some("Jeda pratinjau"),
		"Pause voice message" => Some("Jeda pesan suara"),
		"Paused" => Some("Dijeda"),
		"Pe&nding admin invites..." => Some("Undangan admin ter&tunda..."),
		"Pending" => Some("Tertunda"),
		"Permanently delete your whole call history" => Some("Hapus seluruh riwayat panggilan Anda secara permanen"),
		"Phone number must have at least 7 digits." => Some("Nomor telepon harus memiliki minimal 7 digit."),
		"Photo" => Some("Foto"),
		"Photo or video status update..." => Some("Pembaruan status foto atau video..."),
		"Photos" => Some("Foto"),
		"Photos and videos" => Some("Foto dan video"),
		"Pi&n..." => Some("Se&matkan..."),
		"Pick a chat on the left to start messaging." => Some("Pilih chat di sebelah kiri untuk mulai berkirim pesan."),
		"Pick a date and time for this message to be sent" => Some("Pilih tanggal dan waktu pengiriman pesan ini"),
		"Pictures|*.jpg;*.jpeg;*.png;*.gif" => Some("Gambar|*.jpg;*.jpeg;*.png;*.gif"),
		"Pin Message" => Some("Sematkan Pesan"),
		"Pin or unpin this post at the top of the channel" => {
			Some("Sematkan atau lepas sematan postingan ini di bagian atas saluran")
		}
		"Pin this message at the top of the chat for everyone" => {
			Some("Sematkan pesan ini di bagian atas chat untuk semua orang")
		}
		"Pin this post, or see who reacted or voted" => {
			Some("Sematkan postingan ini, atau lihat siapa yang bereaksi atau memilih")
		}
		"Pinned" => Some("Disematkan"),
		"Pinned Messages" => Some("Pesan Tersemat"),
		"Place a video call" => Some("Lakukan panggilan video"),
		"Place a voice call" => Some("Lakukan panggilan suara"),
		"Placing call..." => Some("Memanggil..."),
		"Placing group call..." => Some("Memulai panggilan grup..."),
		"Placing video call..." => Some("Memulai panggilan video..."),
		"Play" => Some("Putar"),
		"Play next update &automatically" => Some("Putar pembaruan berikutnya secara &otomatis"),
		"Play the message sound but show no notification" => {
			Some("Putar suara pesan tetapi jangan tampilkan notifikasi")
		}
		"Played" => Some("Diputar"),
		"Played at" => Some("Diputar pukul"),
		"Played by" => Some("Diputar oleh"),
		"Playing" => Some("Memutar"),
		"Playing voice preview." => Some("Memutar pratinjau suara."),
		"Poll" => Some("Polling"),
		"Post &photo/video status update..." => Some("Posting pembaruan status &foto/video..."),
		"Post &text status update..." => Some("Posting pembaruan status &teks..."),
		"Post Status" => Some("Posting Status"),
		"Post Text Status" => Some("Posting Status Teks"),
		"Post text status. Type your status update, press Next Colour or Next Font to style, or Ctrl+Enter to post." => {
			Some(
				"Posting status teks. Ketik pembaruan status Anda, tekan Warna Berikutnya atau Font Berikutnya untuk mengubah gaya, atau tekan Ctrl+Enter untuk memposting.",
			)
		}
		"Preview" => Some("Pratinjau"),
		"Price" => Some("Harga"),
		"Product details" => Some("Detail produk"),
		"Product link is in the message box. Add your question and send." => {
			Some("Tautan produk berada di kotak pesan. Tambahkan pertanyaan Anda lalu kirim.")
		}
		"Put Link in &Message Box" => Some("Masukkan Tautan di Kolom &Pesan"),
		"QR Code expired. Click Reload to get a new code:" => {
			Some("Kode QR kedaluwarsa. Klik Muat Ulang untuk mendapatkan kode baru:")
		}
		"QR code expired. Click Reload QR Code to try again." => {
			Some("Kode QR kedaluwarsa. Klik Muat Ulang Kode QR untuk mencoba lagi.")
		}
		"QR code updated" => Some("Kode QR diperbarui"),
		"Quiz" => Some("Kuis"),
		"Quiz, choose one answer; you can't change it afterwards" => {
			Some("Kuis, pilih satu jawaban; Anda tidak dapat mengubahnya setelahnya")
		}
		"Quoted message removed." => Some("Pesan kutipan dihapus."),
		"Re&quest again" => Some("Minta &ulang"),
		"Rea&ct..." => Some("Rea&ksi..."),
		"React" => Some("Reaksi"),
		"React to Status" => Some("Reaksi ke Status"),
		"React to message" => Some("Reaksi terhadap pesan"),
		"React to their latest status update" => Some("Beri reaksi pada pembaruan status terbaru mereka"),
		"React with Emoji" => Some("Beri Reaksi dengan Emoji"),
		"React with:" => Some("Beri reaksi dengan:"),
		"React:" => Some("Reaksi:"),
		"Reaction removed." => Some("Reaksi dihapus."),
		"Read" => Some("Dibaca"),
		"Read at" => Some("Dibaca pukul"),
		"Read by" => Some("Dibaca oleh"),
		"Read new messages aloud while this chat is open (Alt+L)" => {
			Some("Bacakan pesan baru dengan suara saat chat ini terbuka (Alt+L)")
		}
		"Recent" => Some("Terkini"),
		"Recent calls" => Some("Panggilan terbaru"),
		"Recent updates" => Some("Pembaruan terkini"),
		"Recently used" => Some("Baru saja digunakan"),
		"Recording voice message..." => Some("Merekam pesan suara..."),
		"Reject" => Some("Tolak"),
		"Remove Account" => Some("Hapus Akun"),
		"Remove Group" => Some("Hapus Grup"),
		"Remove Member" => Some("Keluarkan Anggota"),
		"Remove admin" => Some("Hapus admin"),
		"Remove all messages from starred messages" => Some("Hapus semua pesan dari pesan berbintang"),
		"Remove as admin" => Some("Hapus sebagai admin"),
		"Remove from &favorite stickers" => Some("Hapus dari stiker &favorit"),
		"Remove from community..." => Some("Hapus dari komunitas..."),
		"Remove reaction" => Some("Hapus reaksi"),
		"Remove the channel's picture" => Some("Hapus foto saluran"),
		"Remove this member from the group" => Some("Keluarkan anggota ini dari grup"),
		"Remove your profile photo?" => Some("Hapus foto profil Anda?"),
		"Removing your tag..." => Some("Menghapus label Anda..."),
		"Rename Account" => Some("Ganti Nama Akun"),
		"Rename List" => Some("Ganti Nama Daftar"),
		"Replace Text With Emoji" => Some("Ganti Teks Dengan Emoji"),
		"Reply to message" => Some("Balas pesan"),
		"Reply to their latest status update in a chat with them" => {
			Some("Balas pembaruan status terbaru mereka di chat bersama mereka")
		}
		"Report" => Some("Laporkan"),
		"Report and exit group" => Some("Laporkan dan keluar dari grup"),
		"Report this message to WhatsApp" => Some("Laporkan pesan ini ke WhatsApp"),
		"Report to group &admins" => Some("Laporkan ke &admin grup"),
		"Report to group admins" => Some("Laporkan ke admin grup"),
		"Reported messages" => Some("Pesan yang dilaporkan"),
		"Reported messages..." => Some("Pesan yang dilaporkan..."),
		"Reported." => Some("Dilaporkan."),
		"Reporting..." => Some("Melaporkan..."),
		"Requested this message from your phone" => Some("Meminta pesan ini dari ponsel Anda"),
		"Requests" => Some("Permintaan"),
		"Res&ume" => Some("&Lanjutkan"),
		"Reset link" => Some("Atur ulang tautan"),
		"Reset this group's invite link? The current link will stop working, and anyone who has it won't be able to join with it." => {
			Some(
				"Atur ulang tautan undangan grup ini? Tautan saat ini tidak akan berfungsi lagi, dan siapa pun yang memilikinya tidak akan dapat bergabung menggunakannya.",
			)
		}
		"Restart Required" => Some("Perlu Mulai Ulang"),
		"Restore Complete" => Some("Pemulihan Selesai"),
		"Restore Failed" => Some("Pemulihan Gagal"),
		"Restoring settings will overwrite your current settings. Do you want to continue?" => {
			Some("Memulihkan setelan akan menimpa setelan Anda saat ini. Anda ingin melanjutkan?")
		}
		"Restricted Chat" => Some("Chat Dibatasi"),
		"Restricted chat (changed on your phone)" => Some("Chat dibatasi (diubah di ponsel Anda)"),
		"Resume recording" => Some("Lanjutkan rekaman"),
		"Retry &send" => Some("Coba lagi &kirim"),
		"Retry sending this message" => Some("Coba kirim ulang pesan ini"),
		"Ringing" => Some("Menderingkan"),
		"Ringing..." => Some("Berdering..."),
		"Ringtone" => Some("Nada dering"),
		"Sale price" => Some("Harga diskon"),
		"Save all media" => Some("Simpan semua media"),
		"Save all photos" => Some("Simpan semua foto"),
		"Save all videos" => Some("Simpan semua video"),
		"Save file as" => Some("Simpan file sebagai"),
		"Save name and description" => Some("Simpan nama dan deskripsi"),
		"Save settings" => Some("Simpan setelan"),
		"Save this member as a contact" => Some("Simpan anggota ini sebagai kontak"),
		"Save voice message to device" => Some("Simpan pesan suara ke perangkat"),
		"Saving..." => Some("Menyimpan..."),
		"Scheduled messages" => Some("Pesan terjadwal"),
		"Scheduled messages: none" => Some("Pesan terjadwal: tidak ada"),
		"Scheduling message..." => Some("Menjadwalkan pesan..."),
		"Screen" => Some("Layar"),
		"Screen sharing isn't ready yet - wait for the call to connect." => {
			Some("Berbagi layar belum siap, tunggu panggilan terhubung.")
		}
		"Search a contact to add to the call, then press Enter" => {
			Some("Cari kontak untuk ditambahkan ke panggilan, lalu tekan Enter")
		}
		"Search a contact to block, then press Enter" => Some("Cari kontak untuk diblokir, lalu tekan Enter"),
		"Search channels, then press Enter" => Some("Cari saluran, lalu tekan Enter"),
		"Search contacts or chats:" => Some("Cari kontak atau chat:"),
		"Search emoji" => Some("Cari emoji"),
		"Search members" => Some("Cari anggota"),
		"Searching..." => Some("Mencari..."),
		"Secret Code Required" => Some("Kode Rahasia Diperlukan"),
		"See and cancel admin invites not accepted yet" => {
			Some("Lihat dan batalkan undangan admin yang belum diterima")
		}
		"See member changes" => Some("Lihat perubahan anggota"),
		"See this member's info, call them, or save them as a contact" => {
			Some("Lihat info anggota ini, telepon mereka, atau simpan sebagai kontak")
		}
		"See who follows the channel, and manage its admins" => {
			Some("Lihat siapa yang mengikuti saluran, dan kelola adminnya")
		}
		"See who has seen one of your status updates, and their reactions" => {
			Some("Lihat siapa yang telah melihat salah satu pembaruan status Anda beserta reaksinya")
		}
		"See who has seen:" => Some("Lihat siapa yang telah melihat:"),
		"See who reacted to this post" => Some("Lihat siapa yang bereaksi terhadap postingan ini"),
		"See who voted for each option" => Some("Lihat siapa yang memilih tiap pilihan"),
		"Select A&ll" => Some("Pilih &Semua"),
		"Select all" => Some("Pilih semua"),
		"Select a contact or call from the list first." => {
			Some("Pilih kontak atau panggilan dari daftar terlebih dahulu.")
		}
		"Select a link to open:" => Some("Pilih tautan untuk dibuka:"),
		"Select a reaction emoji:" => Some("Pilih emoji reaksi:"),
		"Select folder to save all media into" => Some("Pilih folder untuk menyimpan semua media"),
		"Select folder to save all photos into" => Some("Pilih folder untuk menyimpan semua foto"),
		"Select folder to save all videos into" => Some("Pilih folder untuk menyimpan semua video"),
		"Select messages" => Some("Pilih pesan"),
		"Select someone in the call first." => Some("Pilih seseorang dalam panggilan terlebih dahulu."),
		"Select someone who wants to join first." => Some("Pilih seseorang yang ingin bergabung terlebih dahulu."),
		"Selection mode off." => Some("Mode seleksi nonaktif."),
		"Selection mode. Press Space to select a message. 0 messages selected." => {
			Some("Mode seleksi. Tekan Spasi untuk memilih pesan. 0 pesan dipilih.")
		}
		"Send Message History" => Some("Kirim Riwayat Pesan"),
		"Send Pasted Files" => Some("Kirim File yang Ditempel"),
		"Send a favorite, recent or WhatsApp sticker, or a sticker file" => {
			Some("Kirim stiker favorit, terbaru, stiker WhatsApp, atau file stiker")
		}
		"Send a sticker &file (.webp)..." => Some("Kirim &file stiker (.webp)..."),
		"Send any file as a document, photos at full quality" => {
			Some("Kirim file apa pun sebagai dokumen, foto dalam kualitas penuh")
		}
		"Send as:" => Some("Kirim sebagai:"),
		"Send audio files to play in the chat" => Some("Kirim file audio untuk diputar di chat"),
		"Send call link" => Some("Kirim tautan panggilan"),
		"Send invite link" => Some("Kirim tautan undangan"),
		"Send link..." => Some("Kirim tautan..."),
		"Send message &history" => Some("Kirim &riwayat pesan"),
		"Send or cancel the voice message before switching accounts." => {
			Some("Kirim atau batalkan pesan suara sebelum beralih akun.")
		}
		"Send photos and videos" => Some("Kirim foto dan video"),
		"Send reply" => Some("Kirim balasan"),
		"Send this message now? It won't be sent again at the scheduled time." => {
			Some("Kirim pesan ini sekarang? Pesan tidak akan dikirim lagi pada waktu yang dijadwalkan.")
		}
		"Send this message to the group's admins to review" => Some("Kirim pesan ini ke admin grup untuk ditinjau"),
		"Send this message to the group's admins to review? The sender won't be told who reported it." => Some(
			"Kirim pesan ini ke admin grup untuk ditinjau? Pengirim tidak akan diberi tahu siapa yang melaporkannya.",
		),
		"Send this new member recent messages from before they joined" => {
			Some("Kirim pesan terbaru sebelum anggota baru ini bergabung")
		}
		"Sending admin invites..." => Some("Mengirim undangan admin..."),
		"Sending poll..." => Some("Mengirim polling..."),
		"Sending quiz..." => Some("Mengirim kuis..."),
		"Sending to the admins..." => Some("Mengirim ke admin..."),
		"Sending..." => Some("Mengirim..."),
		"Sent" => Some("Terkirim"),
		"Sent at" => Some("Dikirim pukul"),
		"Sent to" => Some("Dikirim ke"),
		"Sent to the group's admins." => Some("Terkirim ke admin grup."),
		"Set tag" => Some("Atur label"),
		"Setting your tag..." => Some("Mengatur label Anda..."),
		"Settings backup must be a JSON object" => Some("Cadangan setelan harus berupa objek JSON"),
		"Share recent messages with people added" => Some("Bagikan pesan terbaru dengan orang yang baru ditambahkan"),
		"Share screen and sound" => Some("Bagikan layar dan suara"),
		"Share screen only" => Some("Bagikan layar saja"),
		"Sharing" => Some("Berbagi"),
		"Similar channels..." => Some("Saluran serupa..."),
		"Someone" => Some("Seseorang"),
		"Someone in the call muted you." => Some("Seseorang dalam panggilan membisukan Anda."),
		"Something went wrong." => Some("Terjadi kesalahan."),
		"Sound &only" => Some("Suara &saja"),
		"Sound from the call isn't getting through." => Some("Suara dari panggilan tidak terdengar."),
		"Sound on" => Some("Suara aktif"),
		"Speed 1x" => Some("Kecepatan 1x"),
		"Spell check is off. Turn it on in Settings, on the Chats tab." => {
			Some("Pemeriksa ejaan nonaktif. Aktifkan di Setelan, pada tab Chat.")
		}
		"Star or unstar this message" => Some("Bintangi atau hapus bintang pesan ini"),
		"Starred" => Some("Berbintang"),
		"Starred messages" => Some("Pesan berbintang"),
		"Start a call with this message's sender" => Some("Mulai panggilan dengan pengirim pesan ini"),
		"Start a chat with this phone number" => Some("Mulai chat dengan nomor telepon ini"),
		"Start time, like 18:30 (optional)" => Some("Waktu mulai, seperti 18:30 (opsional)"),
		"Status &privacy..." => Some("&Privasi status..."),
		"Status &update" => Some("&Pembaruan status"),
		"Status update cannot be empty." => Some("Pembaruan status tidak boleh kosong."),
		"Status update deleted." => Some("Pembaruan status dihapus."),
		"Status: Connected (Syncing...)" => Some("Status: Terhubung (Menyinkronkan...)"),
		"Status: My Contacts Except..." => Some("Status: Kontak Saya Kecuali..."),
		"Status: Only Share With..." => Some("Status: Hanya Bagikan Dengan..."),
		"Sticker" => Some("Stiker"),
		"Stickers" => Some("Stiker"),
		"Stickers (*.webp)|*.webp" => Some("Stiker (*.webp)|*.webp"),
		"Stickers from:" => Some("Stiker dari:"),
		"Stop marking this word until whatSnap restarts" => {
			Some("Jangan tandai kata ini sampai whatSnap dimulai ulang")
		}
		"Stop sharing screen" => Some("Hentikan berbagi layar"),
		"Stopped sharing your screen." => Some("Berhenti membagikan layar."),
		"Switch Account" => Some("Beralih Akun"),
		"Take this message off your starred messages" => Some("Hapus pesan ini dari pesan berbintang Anda"),
		"Take this sticker off your favorites" => Some("Hapus stiker ini dari favorit Anda"),
		"Text" => Some("Teks"),
		"Text status update..." => Some("Pembaruan status teks..."),
		"Text that turns into emoji" => Some("Teks yang berubah menjadi emoji"),
		"That is not a date. Use year-month-day, like 2026-03-27." => {
			Some("Itu bukan format tanggal. Gunakan tahun-bulan-hari, contoh 2026-03-27.")
		}
		"That isn't a WhatsApp call link." => Some("Tautan tersebut bukan tautan panggilan WhatsApp."),
		"That message isn't on this computer, so it can't be shown." => {
			Some("Pesan tersebut tidak ada di komputer ini, sehingga tidak dapat ditampilkan.")
		}
		"The camera isn't ready yet - wait for the call to connect." => {
			Some("Kamera belum siap, tunggu panggilan terhubung.")
		}
		"The channel" => Some("Saluran"),
		"The file does not appear to contain whatSnap settings" => {
			Some("File ini tampaknya tidak memuat setelan whatSnap")
		}
		"The interface language was changed. Restart whatSnap now so the change takes full effect?" => Some(
			"Bahasa antarmuka telah diubah. Mulai ulang whatSnap sekarang agar perubahan langsung diterapkan sepenuhnya?",
		),
		"The message is empty." => Some("Pesan kosong."),
		"The notification tone is shared with WhatsApp on your other computers, unless it is a custom sound file; the rest only change this computer. To mute this chat on all your devices, use Mute in its menu." => {
			Some(
				"Nada notifikasi disinkronkan dengan WhatsApp di komputer Anda yang lain, kecuali jika berupa file audio kustom; setelan lainnya hanya berlaku untuk komputer ini. Untuk membisukan chat ini di semua perangkat, gunakan menu Bisukan.",
			)
		}
		"The other person" => Some("Lawan bicara"),
		"The question is too long. Polls allow up to 255 characters." => {
			Some("Pertanyaan terlalu panjang. Polling maksimal 255 karakter.")
		}
		"The video couldn't be decoded. The file may be damaged." => {
			Some("Video tidak dapat didekode. File mungkin rusak.")
		}
		"The video couldn't be played." => Some("Video tidak dapat diputar."),
		"This button only works in WhatsApp on your phone." => {
			Some("Tombol ini hanya berfungsi di WhatsApp pada ponsel Anda.")
		}
		"This channel is suspended" => Some("Saluran ini ditangguhkan"),
		"This chat has no message box." => Some("Chat ini tidak memiliki kotak pesan."),
		"This chat has no scheduled messages." => Some("Chat ini tidak memiliki pesan terjadwal."),
		"This group" => Some("Grup ini"),
		"This is the first status update." => Some("Ini adalah pembaruan status pertama."),
		"This is the only account in whatSnap. To unlink it, use Account, Log Out." => {
			Some("Ini satu-satunya akun di whatSnap. Untuk memutus tautan, gunakan Akun, Keluar.")
		}
		"This is the start of the chat." => Some("Ini adalah awal chat."),
		"This message was deleted" => Some("Pesan ini telah dihapus"),
		"This message will not be sent. This action cannot be undone." => {
			Some("Pesan ini tidak akan dikirim. Tindakan ini tidak dapat dibatalkan.")
		}
		"This pack has no stickers." => Some("Paket ini tidak memiliki stiker."),
		"This photo couldn't be shown." => Some("Foto ini tidak dapat ditampilkan."),
		"This video's format can't be played here." => Some("Format video ini tidak dapat diputar di sini."),
		"Today" => Some("Hari ini"),
		"Turn camera off" => Some("Matikan kamera"),
		"Turn off" => Some("Matikan"),
		"Turn on" => Some("Nyalakan"),
		"Two options are the same. Each option must be different." => {
			Some("Dua pilihan bernilai sama. Setiap pilihan harus berbeda.")
		}
		"Type a message to schedule." => Some("Ketik pesan yang akan dijadwalkan."),
		"Type a name for the event." => Some("Ketik nama untuk acara tersebut."),
		"Type a question for the poll." => Some("Ketik pertanyaan untuk polling."),
		"Type a reply first." => Some("Ketik balasan terlebih dahulu."),
		"Type an update" => Some("Ketik pembaruan"),
		"Un&keep" => Some("Batalkan &simpan"),
		"Un&like" => Some("&Batal suka"),
		"Unblock contact" => Some("Buka blokir kontak"),
		"Unfollow" => Some("Batal mengikuti"),
		"Unfollow channel" => Some("Batal mengikuti saluran"),
		"Unfollow this channel? Its posts will be removed from whatSnap." => {
			Some("Batal mengikuti saluran ini? Postingannya akan dihapus dari whatSnap.")
		}
		"Unkeep" => Some("Batalkan simpan"),
		"Unknown" => Some("Tidak dikenal"),
		"Unknown contact" => Some("Kontak tidak dikenal"),
		"Unknown error" => Some("Kesalahan tidak dikenal"),
		"Unknown member" => Some("Anggota tidak dikenal"),
		"Unlock Failed" => Some("Gagal Membuka Kunci"),
		"Unlock Locked Chats" => Some("Buka Kunci Chat Dikunci"),
		"Unlock Your Chats Faster" => Some("Buka Kunci Chat Anda Lebih Cepat"),
		"Unlock your locked chats in whatSnap" => Some("Buka kunci chat dikunci Anda di whatSnap"),
		"Unmute" => Some("Bunyikan"),
		"Unmute microphone" => Some("Bunyikan mikrofon"),
		"Unnamed channel" => Some("Saluran tanpa nama"),
		"Unnamed product" => Some("Produk tanpa nama"),
		"Unpi&n" => Some("Lepas &sematan"),
		"Unpin this message for everyone in the chat" => Some("Lepas sematan pesan ini untuk semua orang di chat"),
		"Unstar &all..." => Some("Hapus &semua bintang..."),
		"Unstar All Messages" => Some("Hapus Bintang Semua Pesan"),
		"Unstar message" => Some("Hapus bintang pesan"),
		"Unstar messages" => Some("Hapus bintang pesan"),
		"Upload cancelled." => Some("Unggahan dibatalkan."),
		"Upload progress" => Some("Kemajuan unggahan"),
		"Uploading" => Some("Mengunggah"),
		"Use this spelling" => Some("Gunakan ejaan ini"),
		"Verified channel" => Some("Saluran terverifikasi"),
		"Verify Security Code" => Some("Verifikasi Kode Keamanan"),
		"Video" => Some("Video"),
		"Video call connected. Your camera is off." => Some("Panggilan video terhubung. Kamera Anda mati."),
		"Video call with" => Some("Panggilan video dengan"),
		"Video you sent" => Some("Video yang Anda kirim"),
		"Videos" => Some("Video"),
		"View channel..." => Some("Lihat saluran..."),
		"View delivery info, or a poll's votes" => Some("Lihat info pengiriman, atau suara jajak pendapat"),
		"View once voice message sent." => Some("Pesan suara sekali lihat terkirim."),
		"View their status updates" => Some("Lihat pembaruan status mereka"),
		"View your status updates" => Some("Lihat pembaruan status Anda"),
		"Viewed" => Some("Sudah dilihat"),
		"Viewed By" => Some("Dilihat Oleh"),
		"Viewed updates" => Some("Pembaruan yang telah dilihat"),
		"Viewed, muted" => Some("Sudah dilihat, dibisukan"),
		"Voice call" => Some("Panggilan suara"),
		"Voice call with" => Some("Panggilan suara dengan"),
		"Voice message" => Some("Pesan suara"),
		"Voice message sent." => Some("Pesan suara terkirim."),
		"Voice note" => Some("Pesan suara"),
		"Voice note status." => Some("Status pesan suara."),
		"Voice preview finished." => Some("Pratinjau pesan suara selesai."),
		"Voice preview paused." => Some("Pratinjau suara dijeda."),
		"Voice recording discarded." => Some("Rekaman suara dibuang."),
		"Voice recording paused. Preview available." => Some("Perekaman suara dijeda. Pratinjau tersedia."),
		"Voice recording resumed." => Some("Perekaman suara dilanjutkan."),
		"Voice recording stopped. Ready to send." => Some("Perekaman suara dihentikan. Siap dikirim."),
		"Vote" => Some("Beri Suara"),
		"Vote removed." => Some("Pilihan dihapus."),
		"Voted." => Some("Pilihan dikirim."),
		"Votes" => Some("Pilihan"),
		"Waiting for someone in the call to let you in." => {
			Some("Menunggu seseorang dalam panggilan untuk mengizinkan Anda masuk.")
		}
		"Waiting for someone in the call to let you in..." => {
			Some("Menunggu seseorang dalam panggilan untuk mengizinkan Anda masuk...")
		}
		"Website" => Some("Situs web"),
		"What kind of call?" => Some("Jenis panggilan apa?"),
		"What's this community for? (You can leave this empty.)" => {
			Some("Untuk apa komunitas ini? (Anda dapat mengosongkannya.)")
		}
		"WhatsApp Login - Pairing Code" => Some("Login WhatsApp - Kode Penautan"),
		"WhatsApp Login - QR Code" => Some("Login WhatsApp - Kode QR"),
		"WhatsApp Name" => Some("Nama WhatsApp"),
		"WhatsApp hasn't shared this person's phone number, so they can't be saved as a contact." => {
			Some("WhatsApp belum membagikan nomor telepon orang ini, sehingga tidak dapat disimpan sebagai kontak.")
		}
		"WhatsApp hasn't turned on scheduling messages for your account yet." => {
			Some("WhatsApp belum mengaktifkan penjadwalan pesan untuk akun Anda.")
		}
		"WhatsApp hasn't turned on scheduling messages for your account." => {
			Some("WhatsApp belum mengaktifkan penjadwalan pesan untuk akun Anda.")
		}
		"WhatsApp sends these at the time shown, even when this computer is off." => {
			Some("WhatsApp akan mengirimkan pesan ini pada waktu yang ditentukan, meskipun komputer ini mati.")
		}
		"Which answer is correct?" => Some("Jawaban mana yang benar?"),
		"Who &reacted..." => Some("Siapa yang &bereaksi..."),
		"Who &voted..." => Some("Siapa yang &memilih..."),
		"Who can see your status updates:" => Some("Siapa yang dapat melihat pembaruan status Anda:"),
		"Window" => Some("Jendela"),
		"Yesterday" => Some("Kemarin"),
		"You" => Some("Anda"),
		"You are already on a call. End the current call before placing a new one." => {
			Some("Anda sudah berada dalam panggilan. Akhiri panggilan saat ini sebelum melakukan panggilan baru.")
		}
		"You can't send messages to this group because you're no longer a member" => {
			Some("Anda tidak dapat mengirim pesan ke grup ini karena Anda bukan anggota lagi")
		}
		"You can't send messages to this group because you're no longer a member." => {
			Some("Anda tidak dapat mengirim pesan ke grup ini karena Anda bukan anggota lagi.")
		}
		"You cannot send messages to this chat." => Some("Anda tidak dapat mengirim pesan ke chat ini."),
		"You deleted this message" => Some("Anda menghapus pesan ini"),
		"You don't follow this channel" => Some("Anda tidak mengikuti saluran ini"),
		"You follow this channel" => Some("Anda mengikuti saluran ini"),
		"You have no locked chats" => Some("Anda tidak memiliki chat yang dikunci"),
		"You left the group." => Some("Anda keluar dari grup."),
		"You own this channel" => Some("Anda adalah pemilik saluran ini"),
		"You're all caught up." => Some("Semua pesan sudah dibaca."),
		"You're an admin of this channel" => Some("Anda adalah admin saluran ini"),
		"You're sharing a window:" => Some("Anda sedang membagikan jendela:"),
		"You're sharing your screen." => Some("Anda sedang membagikan layar."),
		"You: " => Some("Anda: "),
		"Your answer" => Some("Jawaban Anda"),
		"Your call history will be permanently deleted." => {
			Some("Riwayat panggilan Anda akan dihapus secara permanen.")
		}
		"Your personal messages are end-to-end encrypted." => Some("Pesan pribadi Anda terenkripsi secara end-to-end."),
		"Your phone didn't share any message history with whatSnap. New messages will still arrive." => {
			Some("Ponsel Anda tidak membagikan riwayat pesan dengan whatSnap. Pesan baru akan tetap masuk.")
		}
		"Your phone shared only your recent messages with whatSnap, not your full chat history. Older messages load from your phone when you scroll to the top of a chat." => {
			Some(
				"Ponsel Anda hanya membagikan pesan terkini dengan whatSnap, bukan seluruh riwayat chat. Pesan yang lebih lama akan dimuat dari ponsel Anda saat Anda menggulir ke bagian atas chat.",
			)
		}
		"Your profile photo was changed." => Some("Foto profil Anda telah diubah."),
		"Your profile photo was removed." => Some("Foto profil Anda telah dihapus."),
		"Your status update can't exceed 700 characters or 10 lines." => {
			Some("Pembaruan status Anda tidak boleh melebihi 700 karakter atau 10 baris.")
		}
		"Your tag in this group" => Some("Label Anda di grup ini"),
		"Your tag was removed." => Some("Label Anda telah dihapus."),
		"Your whatSnap settings were successfully backed up." => Some("Setelan whatSnap Anda berhasil dicadangkan."),
		"Your whatSnap settings were successfully restored." => Some("Setelan whatSnap Anda berhasil dipulihkan."),
		"Your whatSnap settings were successfully restored. The interface language was changed. Restart whatSnap now so the change takes full effect?" => {
			Some(
				"Setelan whatSnap Anda berhasil dipulihkan. Bahasa antarmuka telah diubah. Mulai ulang whatSnap sekarang agar perubahan langsung diterapkan sepenuhnya?",
			)
		}
		"Zip archive (*.zip)|*.zip" => Some("Arsip zip (*.zip)|*.zip"),
		"\n\nAccept? As an admin you can post updates and change the channel's details." => {
			Some("\n\nTerima? Sebagai admin Anda dapat memposting pembaruan dan mengubah detail saluran.")
		}
		"a channel" => Some("suatu saluran"),
		"admin" => Some("admin"),
		"as set in Settings" => Some("seperti yang diatur di Setelan"),
		"channel" => Some("saluran"),
		"couldn't allocate a surface for the window" => Some("tidak dapat mengalokasikan permukaan untuk jendela"),
		"couldn't allocate the capture bitmap" => Some("tidak dapat mengalokasikan bitmap tangkapan layar"),
		"couldn't create a capture surface" => Some("tidak dapat membuat permukaan tangkapan layar"),
		"couldn't open the screen for capture" => Some("tidak dapat membuka layar untuk perekaman"),
		"declined" => Some("menolak"),
		"declined the call." => Some("menolak panggilan."),
		"delivered" => Some("tersampaikan"),
		"didn't answer." => Some("tidak menjawab."),
		"following" => Some("mengikuti"),
		"group video call from" => Some("panggilan video grup dari"),
		"group voice call from" => Some("panggilan suara grup dari"),
		"in" => Some("di"),
		"in the call" => Some("dalam panggilan"),
		"invited" => Some("diundang"),
		"joined the call." => Some("bergabung ke panggilan."),
		"joining" => Some("bergabung"),
		"left the call." => Some("meninggalkan panggilan."),
		"muted" => Some("dibisukan"),
		"no" => Some("tidak"),
		"no one" => Some("tidak ada"),
		"notification and sound" => Some("notifikasi dan suara"),
		"notification only, no sound" => Some("notifikasi saja, tanpa suara"),
		"notify normally" => Some("beritahu seperti biasa"),
		"off on this computer" => Some("mati di komputer ini"),
		"on" => Some("aktif"),
		"owner" => Some("pemilik"),
		"played" => Some("diputar"),
		"quoted message" => Some("pesan yang dikutip"),
		"read" => Some("dibaca"),
		"recording audio..." => Some("sedang merekam audio..."),
		"recording audio…" => Some("merekam audio…"),
		"ringing" => Some("berdering"),
		"silent" => Some("senyap"),
		"sound only" => Some("hanya suara"),
		"sound only, no notification" => Some("suara saja, tanpa notifikasi"),
		"started sharing their screen." => Some("mulai membagikan layarnya."),
		"stopped sharing their screen." => Some("berhenti membagikan layarnya."),
		"the channel" => Some("saluran"),
		"the group" => Some("grup"),
		"the screen couldn't be read" => Some("layar tidak dapat dibaca"),
		"the window can't be read right now" => Some("jendela tidak dapat dibaca saat ini"),
		"the window couldn't be read" => Some("jendela tidak dapat dibaca"),
		"the window to share is gone" => Some("jendela yang dibagikan sudah ditutup"),
		"them" => Some("mereka"),
		"this group" => Some("grup ini"),
		"turned their camera off." => Some("mematikan kameranya."),
		"turned their camera on." => Some("menyalakan kameranya."),
		"typing..." => Some("sedang mengetik..."),
		"typing…" => Some("mengetik…"),
		"unknown error" => Some("kesalahan tidak dikenal"),
		"verified" => Some("terverifikasi"),
		"video call from" => Some("panggilan video dari"),
		"voice call from" => Some("panggilan suara dari"),
		"wants to switch to video. Turn on your camera (Ctrl+Shift+V) to switch to video." => {
			Some("ingin beralih ke video. Nyalakan kamera Anda (Ctrl+Shift+V) untuk beralih ke video.")
		}
		"whatSnap for Windows" => Some("whatSnap untuk Windows"),
		"yes" => Some("ya"),
		"&8 hours" => Some("&8 jam"),
		"&Always" => Some("&Selalu"),
		"&Pin post" => Some("&Sematkan postingan"),
		"&Unpin post" => Some("&Lepas sematan postingan"),
		"1 &week" => Some("1 &minggu"),
		"24 hours" => Some("24 jam"),
		"30 days" => Some("30 hari"),
		"7 days" => Some("7 hari"),
		"90 days" => Some("90 hari"),
		"Activities" => Some("Aktivitas"),
		"Advanced chat privacy is off in this chat. When it's on, people in this chat can't export it, media isn't saved to their devices automatically, and messages can't be used with AI features. Turn it on?" => {
			Some(
				"Privasi chat lanjutan nonaktif di chat ini. Saat aktif, peserta chat tidak dapat mengekspor chat, media tidak disimpan otomatis ke perangkat mereka, dan pesan tidak dapat digunakan dengan fitur AI. Aktifkan?",
			)
		}
		"Advanced chat privacy is on in this chat. When it's on, people in this chat can't export it, media isn't saved to their devices automatically, and messages can't be used with AI features. Turn it off?" => {
			Some(
				"Privasi chat lanjutan aktif di chat ini. Saat aktif, peserta chat tidak dapat mengekspor chat, media tidak disimpan otomatis ke perangkat mereka, dan pesan tidak dapat digunakan dengan fitur AI. Matikan?",
			)
		}
		"Animals & Nature" => Some("Hewan & Alam"),
		"Anyone with the link can now join straight away." => {
			Some("Siapa saja yang memiliki tautan kini dapat langsung bergabung.")
		}
		"As set in Settings" => Some("Seperti diatur di Setelan"),
		"Birthday Cake" => Some("Kue Ulang Tahun"),
		"Business" => Some("Bisnis"),
		"Channel updated." => Some("Saluran diperbarui."),
		"Channels are public: anyone can find and follow yours, and followers can't see your phone number. By creating a channel you agree to WhatsApp's Channels Guidelines." => {
			Some(
				"Saluran bersifat publik: siapa saja dapat menemukan dan mengikuti saluran Anda, dan pengikut tidak dapat melihat nomor telepon Anda. Dengan membuat saluran, Anda menyetujui Pedoman Saluran WhatsApp.",
			)
		}
		"Choose at least one contact to exclude." => Some("Pilih minimal satu kontak untuk dikecualikan."),
		"Clapping Hands" => Some("Tepuk Tangan"),
		"Clear chat..." => Some("Bersihkan chat..."),
		"Colour 1" => Some("Warna 1"),
		"Colour 10" => Some("Warna 10"),
		"Colour 11" => Some("Warna 11"),
		"Colour 12" => Some("Warna 12"),
		"Colour 13" => Some("Warna 13"),
		"Colour 14" => Some("Warna 14"),
		"Colour 15" => Some("Warna 15"),
		"Colour 16" => Some("Warna 16"),
		"Colour 17" => Some("Warna 17"),
		"Colour 18" => Some("Warna 18"),
		"Colour 19" => Some("Warna 19"),
		"Colour 2" => Some("Warna 2"),
		"Colour 20" => Some("Warna 20"),
		"Colour 21" => Some("Warna 21"),
		"Colour 3" => Some("Warna 3"),
		"Colour 4" => Some("Warna 4"),
		"Colour 5" => Some("Warna 5"),
		"Colour 6" => Some("Warna 6"),
		"Colour 7" => Some("Warna 7"),
		"Colour 8" => Some("Warna 8"),
		"Colour 9" => Some("Warna 9"),
		"Contact details..." => Some("Detail kontak..."),
		"Create channel" => Some("Buat saluran"),
		"Delete chat..." => Some("Hapus chat..."),
		"Disappearing messages..." => Some("Pesan sementara..."),
		"Edit channel" => Some("Edit saluran"),
		"Entertainment" => Some("Hiburan"),
		"Export chat..." => Some("Ekspor chat..."),
		"Face with Tears of Joy" => Some("Wajah Tertawa Bahagia"),
		"Fire" => Some("Api"),
		"Flags" => Some("Bendera"),
		"Folded Hands" => Some("Tangan Terlipat"),
		"Food & Drink" => Some("Makanan & Minuman"),
		"Going" => Some("Hadir"),
		"Heart Eyes" => Some("Mata Hati"),
		"Hot Beverage" => Some("Minuman Panas"),
		"Hundred Points" => Some("Seratus Poin"),
		"In group call" => Some("Dalam panggilan grup"),
		"Last Seen" => Some("Terakhir Dilihat"),
		"Lifestyle" => Some("Gaya Hidup"),
		"Maybe" => Some("Mungkin"),
		"Microphone muted." => Some("Mikrofon dibisukan."),
		"Microphone unmuted." => Some("Mikrofon dibunyikan."),
		"My contacts" => Some("Kontak saya"),
		"My contacts except..." => Some("Kontak saya kecuali..."),
		"New messages in this chat will be read aloud." => Some("Pesan baru di chat ini akan dibacakan."),
		"New messages in this chat won't be read aloud." => Some("Pesan baru di chat ini tidak akan dibacakan."),
		"New people now wait for you to let them in." => {
			Some("Peserta baru kini menunggu persetujuan Anda untuk masuk.")
		}
		"News & Information" => Some("Berita & Informasi"),
		"Not going" => Some("Tidak hadir"),
		"Notification settings..." => Some("Setelan notifikasi..."),
		"Objects" => Some("Objek"),
		"Off on this computer" => Some("Mati di komputer ini"),
		"Online" => Some("Online"),
		"Only share with..." => Some("Hanya bagikan dengan..."),
		"Organizations" => Some("Organisasi"),
		"Party Popper" => Some("Terompet Pesta"),
		"People & Body" => Some("Orang & Tubuh"),
		"Pick someone on the left, then call them." => Some("Pilih seseorang di sebelah kiri, lalu telepon mereka."),
		"Pictures (*.jpg;*.jpeg;*.png;*.webp;*.bmp;*.gif)|*.jpg;*.jpeg;*.png;*.webp;*.bmp;*.gif" => {
			Some("Gambar (*.jpg;*.jpeg;*.png;*.webp;*.bmp;*.gif)|*.jpg;*.jpeg;*.png;*.webp;*.bmp;*.gif")
		}
		"Post Photo Status" => Some("Posting Status Foto"),
		"Post Video Status" => Some("Posting Status Video"),
		"Quoted message" => Some("Pesan dikutip"),
		"Reactions" => Some("Reaksi"),
		"Red Heart" => Some("Hati Merah"),
		"Report this message to WhatsApp? WhatsApp receives this message. The sender and the group won't be notified." => {
			Some(
				"Laporkan pesan ini ke WhatsApp? WhatsApp akan menerima pesan ini. Pengirim dan grup tidak akan diberi tahu.",
			)
		}
		"Report..." => Some("Laporkan..."),
		"Sans-Serif (Default)" => Some("Sans-Serif (Bawaan)"),
		"Script" => Some("Skrip"),
		"Serif" => Some("Serif"),
		"Smileys & Emotion" => Some("Smiley & Emosi"),
		"Smiling Face" => Some("Wajah Tersenyum"),
		"Sparkles" => Some("Kilauan"),
		"Sports" => Some("Olahraga"),
		"Sunglasses" => Some("Kacamata Hitam"),
		"Symbols" => Some("Simbol"),
		"They won't be let in." => Some("Mereka tidak diizinkan masuk."),
		"They're being let in." => Some("Mereka diizinkan masuk."),
		"This copy of whatSnap doesn't include the Share menu helper. Reinstall whatSnap and try again." => Some(
			"Salinan whatSnap ini tidak menyertakan komponen bantu menu Bagikan. Instal ulang whatSnap lalu coba lagi.",
		),
		"This message can no longer be edited - the 15 minute edit window has passed." => {
			Some("Pesan ini tidak dapat diedit lagi karena batas waktu edit 15 menit telah lewat.")
		}
		"This message can no longer be edited - the 30 day edit window has passed." => {
			Some("Pesan ini tidak dapat diedit lagi karena batas waktu edit 30 hari telah lewat.")
		}
		"Thumbs Up" => Some("Jempol ke Atas"),
		"Travel & Places" => Some("Perjalanan & Tempat"),
		"Unlock your chats faster. Next time, use Windows Hello, with your fingerprint, face or PIN, instead of typing the secret code. You can change this in Settings, Chats.\n\nTurn on Windows Hello for locked chats?" => {
			Some(
				"Buka kunci chat Anda lebih cepat. Lain kali, gunakan Windows Hello dengan sidik jari, wajah, atau PIN Anda daripada mengetik kode rahasia. Anda dapat mengubah ini di Setelan, Chat.\n\nAktifkan Windows Hello untuk chat dikunci?",
			)
		}
		"You aren't an admin of any group that could be added. A group can be in one community at a time." => Some(
			"Anda bukan admin di grup mana pun yang dapat ditambahkan. Satu grup hanya bisa berada di satu komunitas.",
		),
		"You lowered your hand." => Some("Anda menurunkan tangan."),
		"Your computer's sound is shared too, screen reader speech included. Muting your microphone mutes it as well." => {
			Some(
				"Suara komputer Anda juga dibagikan, termasuk suara pembaca layar. Membisukan mikrofon juga akan membisukannya.",
			)
		}
		"Your hand is raised." => Some("Tangan Anda diangkat."),
		"Your screen is being shared as a black picture. If your screen reader's screen curtain is on, turn it off so the others can see your screen." => {
			Some(
				"Layar Anda dibagikan sebagai gambar hitam. Jika tirai layar pembaca layar Anda aktif, matikan agar peserta lain dapat melihat layar Anda.",
			)
		}
		"Your screen is visible to the others now." => Some("Layar Anda kini terlihat oleh peserta lain."),
		"beaming face with smiling eyes" => Some("wajah berseri dengan mata tersenyum"),
		"confused face" => Some("wajah bingung"),
		"contact" => Some("kontak"),
		"contacts" => Some("kontak"),
		"crying face" => Some("wajah menangis"),
		"excluded" => Some("dikecualikan"),
		"face blowing a kiss" => Some("wajah memberi ciuman"),
		"face with open mouth" => Some("wajah mulut terbuka"),
		"face with tongue" => Some("wajah menjulurkan lidah"),
		"grinning face" => Some("wajah menyeringai"),
		"grinning squinting face" => Some("wajah menyeringai mata menyipit"),
		"neutral face" => Some("wajah netral"),
		"not available for your postcode" => Some("tidak tersedia untuk kode pos Anda"),
		"not excluded" => Some("tidak dikecualikan"),
		"out of stock" => Some("stok habis"),
		"red heart" => Some("hati merah"),
		"slightly frowning face" => Some("wajah cemberut tipis"),
		"slightly smiling face" => Some("wajah tersenyum tipis"),
		"thumbs down" => Some("jempol ke bawah"),
		"thumbs up" => Some("jempol ke atas"),
		"whatSnap couldn't be added to the Share menu. Try again, or reinstall whatSnap." => {
			Some("whatSnap tidak dapat ditambahkan ke menu Bagikan. Coba lagi, atau instal ulang whatSnap.")
		}
		"whatSnap is already running, but this copy can't reach it. It may have been started as administrator. Close whatSnap in Task Manager, then start it again." => {
			Some(
				"whatSnap sudah berjalan, tetapi salinan ini tidak dapat menghubunginya. Aplikasi mungkin dijalankan sebagai administrator. Tutup whatSnap di Task Manager, lalu jalankan ulang.",
			)
		}
		"winking face" => Some("wajah mengedip"),
				// Formatted strings (placeholders are filled by tr_fmt)
		"{}'s battery is low." => Some("Baterai {} lemah."),
		"{} is on a phone call and can't hear you right now." => Some("{} sedang menerima panggilan telepon lain dan tidak dapat mendengar Anda sekarang."),
		"{} is back on the call." => Some("{} kembali ke panggilan."),
		"Ringing {}." => Some("Memanggil {}."),
		"Missed call from {}." => Some("Panggilan tak terjawab dari {}."),
		"Call ended with {}\r\nDuration: {}" => Some("Panggilan berakhir dengan {}\r\nDurasi: {}"),
		"Call ended with {}" => Some("Panggilan berakhir dengan {}"),
		"Could not register the call key {}; another program is using it" => Some("Tidak dapat mendaftarkan tombol panggilan {}; program lain sedang menggunakannya"),
		"In {}" => Some("Di {}"),
		"{} unread messages" => Some("{} pesan belum dibaca"),
		"{} reacted with {} to: {}" => Some("{} bereaksi {} ke: {}"),
		"Parse error: {} for json: {}" => Some("Kesalahan penguraian: {} untuk json: {}"),
		"Message from {}" => Some("Pesan dari {}"),
		"Couldn't mark {} as read. Check your connection and try again." => Some("Tidak dapat menandai {} sebagai sudah dibaca. Periksa koneksi Anda lalu coba lagi."),
		"Status: Exporting chat... {}/{}" => Some("Status: Mengekspor chat... {}/{}"),
		"Chat text exported to {}. {} messages." => Some("Teks chat diekspor ke {}. {} pesan."),
		"Chat exported to {}. {} media files included, {} could not be downloaded and were skipped." => Some("Chat diekspor ke {}. {} file media disertakan, {} tidak dapat diunduh dan dilewati."),
		"Chat exported to {}. {} media files included." => Some("Chat diekspor ke {}. {} file media disertakan."),
		"{} is recording audio..." => Some("{} sedang merekam audio..."),
		"{} is typing..." => Some("{} sedang mengetik..."),
		"{} (last seen {})" => Some("{} (terakhir dilihat {})"),
		"{}, last seen {}" => Some("{}, terakhir dilihat {}"),
		"{} is now online" => Some("{} sekarang online"),
		"Sent: {}" => Some("Terkirim: {}"),
		"Message edited: {}" => Some("Pesan diedit: {}"),
		"Reaction removed by {}" => Some("Reaksi dihapus oleh {}"),
		"{} reacted with {}" => Some("{} bereaksi dengan {}"),
		"Reacted {} to \"{}\"" => Some("Bereaksi {} ke \"{}\""),
		"Reacted {}" => Some("Bereaksi {}"),
		"Copied {} files to clipboard." => Some("Menyalin {} file ke papan klip."),
		"{} downloaded" => Some("{} diunduh"),
		"Download {}%" => Some("Mengunduh {}%"),
		"Download failed: {}" => Some("Unduhan gagal: {}"),
		"File saved to {}" => Some("File disimpan ke {}"),
		"%b %e, %Y" => Some("%e %b %Y"),
		"{}, tag: {}" => Some("{}, label: {}"),
		"Groups in {}" => Some("Grup di {}"),
		"Your tag is now {}." => Some("Label Anda sekarang {}."),
		"Joined {}." => Some("Bergabung ke {}."),
		"{}\n{} member(s)" => Some("{}\n{} anggota"),
		"Following {}. It's in the Channels tab, Alt+6." => Some("Mengikuti {}. Saluran ini ada di tab Saluran, Alt+6."),
		"Unfollowed {}." => Some("Berhenti mengikuti {}."),
		"Name: {}\nPhone: {}\n" => Some("Nama: {}\nTelepon: {}\n"),
		"Conversation data loaded for {}" => Some("Data percakapan dimuat untuk {}"),
		"whatSnap was logged out of WhatsApp: {}" => Some("whatSnap dikeluarkan dari WhatsApp: {}"),
		"Pairing code updated: {}" => Some("Kode penautan diperbarui: {}"),
		"Status: {} Reconnecting..." => Some("Status: {} Menghubungkan ulang..."),
		"Status: Offline. {} Retrying in {} seconds." => Some("Status: Offline. {} Mencoba lagi dalam {} detik."),
		"Status: Offline. {} Retrying in {} minutes." => Some("Status: Offline. {} Mencoba lagi dalam {} menit."),
		"Status: Offline. {} Retrying in {} hours." => Some("Status: Offline. {} Mencoba lagi dalam {} jam."),
		"Status: Offline. {} Retrying automatically." => Some("Status: Offline. {} Mencoba lagi secara otomatis."),
		"{} whatSnap will keep trying to reconnect." => Some("{} whatSnap akan terus mencoba menghubungkan kembali."),
		"Status: Connected (Syncing older messages, {}% complete: {} messages, {} chats so far)" => Some("Status: Terhubung (Menyinkronkan pesan lama, {}% selesai: {} pesan, {} chat sejauh ini)"),
		"Status: Connected (Syncing history... {} messages, {} chats so far)" => Some("Status: Terhubung (Menyinkronkan riwayat... {} pesan, {} chat sejauh ini)"),
		"{}: deleted this message, received at {}" => Some("{}: menghapus pesan ini, diterima pukul {}"),
		"{}: deleted this message" => Some("{}: menghapus pesan ini"),
		", replied to quoted message: \"{}\"" => Some(", membalas pesan yang dikutip: \"{}\""),
		", replied to quoted message from {}: \"{}\"" => Some(", membalas pesan yang dikutip dari {}: \"{}\""),
		", edited" => Some(", diedit"),
		"{}: {}{}{} received at {}{}{}{}" => Some("{}: {}{}{} diterima pukul {}{}{}{}"),
		"Album: {} photos" => Some("Album: {} foto"),
		"Album: {} videos" => Some("Album: {} video"),
		"Album: {} media items" => Some("Album: {} media"),
		"{} photos" => Some("{} foto"),
		"{} videos" => Some("{} video"),
		"{} media items" => Some("{} media"),
		"{}: [Album: {}], received at {}{}" => Some("{}: [Album: {}], diterima pukul {}{}"),
		"{}: [Album: {}]{}" => Some("{}: [Album: {}]{}"),
		"Delete all messages in the chat with {}? The chat itself will stay in your list." => Some("Hapus semua pesan di chat dengan {}? Chat ini akan tetap ada di daftar Anda."),
		"Delete chat with {}?" => Some("Hapus chat dengan {}?"),
		"Are you sure you want to delete chat with {}?" => Some("Yakin ingin menghapus chat dengan {}?"),
		"Notifications for {}" => Some("Setelan notifikasi untuk {}"),
		"Blocked contacts ({})" => Some("Kontak diblokir ({})"),
		"Unblocking {}..." => Some("Membuka blokir {}..."),
		"Picture: {} (change...)" => Some("Foto: {} (ubah...)"),
		"Picture chosen: {}." => Some("Foto dipilih: {}."),
		"Followers of {}" => Some("Pengikut {}"),
		"Invite {} to be an admin of this channel? They'll get the invite in your chat with them, and become an admin once they accept it." => Some("Undang {} menjadi admin saluran ini? Mereka akan menerima undangan di chat Anda dengan mereka, dan menjadi admin setelah menerimanya."),
		"Remove {} as an admin of this channel?" => Some("Hapus {} sebagai admin saluran ini?"),
		"Make {} the owner of this channel? You'll become an admin, and only the new owner can undo this." => Some("Jadikan {} pemilik saluran ini? Anda akan menjadi admin, dan hanya pemilik baru yang dapat membatalkannya."),
		"Pending admin invites for {}" => Some("Undangan admin tertunda untuk {}"),
		"Cancel the invite for {} to be an admin of this channel?" => Some("Batalkan undangan untuk {} agar menjadi admin saluran ini?"),
		"{} followers" => Some("{} pengikut"),
		" in {}" => Some(" di {}"),
		"Loading recommended channels{}..." => Some("Memuat saluran rekomendasi{}..."),
		"No channels found for {}." => Some("Tidak ada saluran untuk {}."),
		"{} more channels." => Some("{} saluran tambahan."),
		"{} channels." => Some("{} saluran."),
		"Notification settings for {}" => Some("Setelan notifikasi untuk {}"),
		"{}: My Contacts Except..." => Some("{}: Kontak Saya Kecuali..."),
		"{} shown. {}" => Some("{} ditampilkan. {}"),
		"Scheduled for {}: {}" => Some("Dijadwalkan untuk {}: {}"),
		"Did not send, {}: {}" => Some("Tidak terkirim, {}: {}"),
		"Couldn't be scheduled, {}: {}" => Some("Tidak dapat dijadwalkan, {}: {}"),
		"Scheduled messages ({}, {} of {} waiting)" => Some("Pesan terjadwal ({}, {} dari {} menunggu)"),
		"Scheduled messages for {}" => Some("Pesan terjadwal untuk {}"),
		"Sending: {}" => Some("Mengirim: {}"),
		"Send to (up to {})" => Some("Kirim ke (maksimal {})"),
		"You can only send to {} chats at once. {} not selected." => Some("Anda hanya dapat mengirim ke {} chat sekaligus. {} tidak dipilih."),
		"{}: selected. {}" => Some("{}: dipilih. {}"),
		"{}: not selected. {}" => Some("{}: tidak dipilih. {}"),
		"{} / 700 characters" => Some("{} / 700 karakter"),
		"Background colour: {}" => Some("Warna latar: {}"),
		"Font style: {}" => Some("Gaya font: {}"),
		"Inserted {}" => Some("{} disisipkan"),
		"1 emoji: {}" => Some("1 emoji: {}"),
		"{} emoji." => Some("{} emoji."),
		"{}, by {}" => Some("{}, oleh {}"),
		"Loading {}..." => Some("Memuat {}..."),
		"{} stickers." => Some("{} stiker."),
		"0:00 of 0:00" => Some("0:00 dari 0:00"),
		"Speed {}x" => Some("Kecepatan {}x"),
		"{} of {}" => Some("{} dari {}"),
		"{} | Draft: {}" => Some("{} | Draf: {}"),
		"No {} chats" => Some("Tidak ada chat di {}"),
		"Cancel download ({}%)" => Some("Batalkan unduhan ({}%)"),
		"Downloading album ({} items remaining)" => Some("Mengunduh album ({} item tersisa)"),
		"Download album ({} items remaining, {})" => Some("Unduh album ({} item tersisa, {})"),
		"Download album ({} items remaining)" => Some("Unduh album ({} item tersisa)"),
		"Cancel upload, {}% complete" => Some("Batalkan unggahan, {}% selesai"),
		"Misspelled: {}" => Some("Salah eja: {}"),
		" You reacted {}." => Some(" Anda bereaksi {}."),
		"{}, {} of {}, {}" => Some("{}, {} dari {}, {}"),
		"Reply sent to {}." => Some("Balasan dikirim ke {}."),
		"Reacted {}." => Some("Bereaksi {}."),
		"{}, status {} of {}, posted {}. {}" => Some("{}, status {} dari {}, dikirim {}. {}"),
		"Un&mute {}" => Some("&Bunyikan {}"),
		"&Mute {}" => Some("&Bisukan {}"),
		"You reacted {}." => Some("Anda bereaksi {}."),
		"Couldn't save the file: {}" => Some("Tidak dapat menyimpan file: {}"),
		"&Message {}" => Some("&Kirim pesan ke {}"),
		"Failed to write backup file:\n{}" => Some("Gagal menulis file cadangan:\n{}"),
		"Failed to format settings:\n{}" => Some("Gagal memformat setelan:\n{}"),
		"Failed to read backup file:\n{}" => Some("Gagal membaca file cadangan:\n{}"),
		"The selected file could not be restored:\n{}" => Some("File yang dipilih tidak dapat dipulihkan:\n{}"),
		"Choose the contacts excluded from {} first, with the button under it." => Some("Pilih terlebih dahulu kontak yang dikecualikan dari {}, dengan tombol di bawahnya."),
		"Are you sure you want to leave {}?" => Some("Anda yakin ingin keluar dari {}?"),
		"&Call {}" => Some("&Telepon {}"),
		"&User actions for {}" => Some("Tindakan &pengguna untuk {}"),
		"Replying to {}: {}" => Some("Membalas {}: {}"),
		"Replying to {}: {}. Type your reply and press Enter." => Some("Membalas {}: {}. Ketik balasan Anda lalu tekan Enter."),
		"Log {} out of WhatsApp on this computer? You'll need to scan a QR code or enter its phone number to use it in whatSnap again." => Some("Keluarkan {} dari WhatsApp di komputer ini? Anda harus memindai kode QR atau memasukkan nomor teleponnya untuk menggunakannya di whatSnap lagi."),
		"This phone number is already linked in whatSnap as {}, so the new link is being removed." => Some("Nomor telepon ini sudah ditautkan di whatSnap sebagai {}, sehingga tautan baru ini dihapus."),
		"{} was logged out of WhatsApp: {}" => Some("{} keluar dari WhatsApp: {}"),
		"{} is calling {}." => Some("{} menelepon {}."),
		"Switched to {}." => Some("Beralih ke {}."),
		"Couldn't add an account: {}" => Some("Tidak dapat menambahkan akun: {}"),
		"Remove {} from whatSnap?\n\nThis logs it out of WhatsApp on this computer and deletes its messages and media saved here. Your phone keeps its chats." => Some("Hapus {} dari whatSnap?\n\nTindakan ini akan mengeluarkan akun dari WhatsApp di komputer ini serta menghapus pesan dan media yang tersimpan di sini. Chat di ponsel Anda tetap aman."),
		"Remove {} from whatSnap?" => Some("Hapus {} dari whatSnap?"),
		"Removed {}." => Some("Akun {} dihapus."),
		"Downloading {}" => Some("Mengunduh {}"),
		"Downloading album of {} files..." => Some("Mengunduh album berisi {} file..."),
		"Video from {}" => Some("Video dari {}"),
		"Block {}? They won't be able to call you or send you messages. They won't be told." => Some("Blokir {}? Mereka tidak akan dapat menelepon atau mengirimi Anda pesan. Mereka tidak akan diberi tahu."),
		"Blocking {}..." => Some("Memblokir {}..."),
		"{} is already blocked." => Some("{} sudah diblokir."),
		"You blocked {}. Unblock them to message or call them?" => Some("Anda memblokir {}. Buka blokir mereka untuk mengirim pesan atau menelepon?"),
		"Sending {}..." => Some("Mengirim {}..."),
		"{}'s catalogue" => Some("Katalog {}"),
		"{}, wants to join" => Some("{}, ingin bergabung"),
		"{} muted you." => Some("{} membisukan Anda."),
		"{} reacted {}." => Some("{} memberi reaksi {}."),
		"Starting group call in {}" => Some("Memulai panggilan grup di {}"),
		"Incoming call from {}" => Some("Panggilan masuk dari {}"),
		"Couldn't share your computer's sound: {}" => Some("Tidak dapat membagikan suara komputer Anda: {}"),
		"Couldn't share the screen: {}" => Some("Tidak dapat membagikan layar: {}"),
		"Couldn't start the camera: {}" => Some("Tidak dapat menyalakan kamera: {}"),
		"Delete {} from your contacts? They'll be removed from your phone's contacts too." => Some("Hapus {} dari kontak Anda? Kontak ini juga akan dihapus dari kontak ponsel Anda."),
		"You invited {} to be an admin of {}. Cancel this invite?" => Some("Anda mengundang {} menjadi admin di {}. Batalkan undangan ini?"),
		"This invite to be an admin of {} has expired." => Some("Undangan untuk menjadi admin di {} ini telah kedaluwarsa."),
		"{} invited you to be an admin of the channel {}." => Some("{} mengundang Anda menjadi admin saluran {}."),
		"Channel {} created." => Some("Saluran {} dibuat."),
		"{} was deleted." => Some("{} telah dihapus."),
		"Inviting {} to be an admin..." => Some("Mengundang {} menjadi admin..."),
		"Removing {} as admin..." => Some("Menghapus {} sebagai admin..."),
		"Making {} the owner..." => Some("Menjadikan {} pemilik saluran..."),
		"Cancelling the invite for {}..." => Some("Membatalkan undangan untuk {}..."),
		"Going to {}." => Some("Membuka tanggal {}."),
		"Name: {}\nPhone: {}\n{}WhatsApp Name: Loading...\nAbout/Status: Loading..." => Some("Nama: {}\nTelepon: {}\n{}Nama WhatsApp: Memuat...\nInfo/Status: Memuat..."),
		"Make new messages in {} disappear after:" => Some("Buat pesan baru di {} menghilang setelah:"),
		"Archived alerts for {} set to {}." => Some("Peringatan arsip untuk {} diatur ke {}."),
		"Chat {} deleted." => Some("Chat {} dihapus."),
		"Left and deleted chat {}." => Some("Keluar dan menghapus chat {}."),
		"Chat {} cleared." => Some("Chat {} dibersihkan."),
		"Lists {} is in" => Some("Daftar yang berisi {}"),
		"Name a new list for {}:" => Some("Beri nama daftar baru untuk {}:"),
		"Chats in {}" => Some("Chat di {}"),
		"Delete the list {}? The chats in it stay as they are." => Some("Hapus daftar {}? Chat di dalamnya tetap ada seperti biasa."),
		"Add Chats to {}" => Some("Tambahkan Chat ke {}"),
		"created" => Some("dibuat"),
		"renamed" => Some("diganti namanya"),
		"List {} {}." => Some("Daftar {} {}."),
		"List {} deleted." => Some("Daftar {} dihapus."),
		"Make {} an admin for \"{}\" group?" => Some("Jadikan {} sebagai admin untuk grup \"{}\"?"),
		"Remove {} from \"{}\" group?" => Some("Keluarkan {} dari grup \"{}\"?"),
		"Left group {}." => Some("Keluar dari grup {}."),
		"Remove {} from the community? It stays a group of its own, and its members leave the community." => Some("Keluarkan {} dari komunitas? Grup ini tetap ada sebagai grup mandiri, dan anggotanya keluar dari komunitas."),
		"Community {} created." => Some("Komunitas {} dibuat."),
		"{}, asked {}" => Some("{}, meminta pada {}"),
		"Your reply to {} wasn't sent because you blocked them." => Some("Balasan Anda kepada {} tidak terkirim karena Anda memblokirnya."),
		"Your reply to {} wasn't sent. {}." => Some("Balasan Anda kepada {} tidak terkirim. {}."),
		"Send {}" => Some("Kirim {}"),
		"Send {} files" => Some("Kirim {} file"),
		"Sending to {}." => Some("Mengirim ke {}."),
		"Sent to {}." => Some("Terkirim ke {}."),
		"Anyone with this link can join {}." => Some("Siapa saja yang memiliki tautan ini dapat bergabung ke {}."),
		"Follow this link to join my WhatsApp group: {}" => Some("Ikuti tautan ini untuk bergabung ke grup WhatsApp saya: {}"),
		"Couldn't get the security code with {}. {}" => Some("Tidak dapat mengambil kode keamanan dengan {}. {}"),
		"Security code with {}" => Some("Kode keamanan dengan {}"),
		"To check that your messages and calls with {} are end-to-end encrypted, compare these 60 \
			 numbers with the ones on their phone, or let them scan the QR code with their phone. The code is \
			 the same on all your devices." => Some("Untuk memastikan pesan dan panggilan Anda dengan {} terenkripsi ujung-ke-ujung, \
			 bandingkan 60 angka ini dengan yang ada di ponselnya, atau minta ia memindai kode QR \
			 dengan ponselnya. Kode ini sama di semua perangkat Anda."),
		"{}, {} of {}" => Some("{}, {} dari {}"),
		"Mention suggestions. {}" => Some("Saran sebutan. {}"),
		"{} mentioned" => Some("{} disebut"),
		"Couldn't restart whatSnap automatically:\n{}\n\nPlease close and reopen the app manually." => Some("Gagal memulai ulang whatSnap secara otomatis:\n{}\n\nSilakan tutup lalu buka ulang aplikasi secara manual."),
		"You've already answered this quiz: {}." => Some("Anda sudah menjawab kuis ini: {}."),
		"Report this message to WhatsApp? WhatsApp receives this message. {} won't be notified." => Some("Laporkan pesan ini ke WhatsApp? WhatsApp akan menerima pesan ini. {} tidak akan diberi tahu."),
		"Report {} to WhatsApp? WhatsApp receives the last message they sent you. They won't be notified." => Some("Laporkan {} ke WhatsApp? WhatsApp akan menerima pesan terakhir yang dikirimnya kepada Anda. Ia tidak akan diberi tahu."),
		"Report {} to WhatsApp? WhatsApp receives the most recent message someone else sent here. The group won't be notified." => Some("Laporkan {} ke WhatsApp? WhatsApp akan menerima pesan terbaru yang dikirim orang lain di sini. Grup tidak akan diberi tahu."),
		"Report and block {}" => Some("Laporkan dan blokir {}"),
		"{}{}. Reported by {}" => Some("{}{}. Dilaporkan oleh {}"),
		"Delete message from {}?" => Some("Hapus pesan dari {}?"),
		"As an admin, you are deleting {}'s message for everyone in this chat. They will see that you deleted the message." => Some("Sebagai admin, Anda menghapus pesan {} untuk semua orang di chat ini. Mereka akan melihat bahwa Anda menghapus pesan ini."),
		"Message forwarded to {}." => Some("Pesan diteruskan ke {}."),
		"Are you sure you want to unstar all {} starred messages?" => Some("Yakin ingin menghapus bintang dari semua {} pesan berbintang?"),
		"Un&mute {}'s status updates" => Some("&Bunyikan pembaruan status {}"),
		"&Mute {}'s status updates" => Some("&Bisukan pembaruan status {}"),
		"Remove my reaction ({})" => Some("Hapus reaksi saya ({})"),
		"Replying to {}'s status: {}" => Some("Membalas status {}: {}"),
		"{}. Type your reply and press Enter." => Some("{}. Ketik balasan Anda lalu tekan Enter."),
		"{} reacted {} to your status." => Some("{} memberi reaksi {} pada status Anda."),
		"Open chat with {}?" => Some("Buka chat dengan {}?"),
		"Volume {}%" => Some("Volume {}%"),
		"Channels like {}" => Some("Saluran serupa {}"),
		"Loading channels like {}..." => Some("Memuat saluran serupa {}..."),
		"Mute {}" => Some("Bisukan {}"),
		"Remove {}" => Some("Keluarkan {}"),
		"Removing {} from the call." => Some("Mengeluarkan {} dari panggilan."),
		"Asking {} to mute." => Some("Meminta {} untuk membisukan."),
		"&Open chat with {}" => Some("&Buka chat dengan {}"),
		"&Reply to {}" => Some("&Balas {}"),
		", reacted {}" => Some(", bereaksi {}"),
		"{} updates" => Some("{} pembaruan"),
		"{} new" => Some("{} baru"),
		"My status: {}" => Some("Status saya: {}"),
		"you reacted {}" => Some("Anda bereaksi {}"),
		"{} message selected" => Some("{} pesan dipilih"),
		"{} messages selected" => Some("{} pesan dipilih"),
		"{} shown. {} contact selected." => Some("{} ditampilkan. {} kontak dipilih."),
		"{} shown. {} contacts selected." => Some("{} ditampilkan. {} kontak dipilih."),
		"{}: {}. {} contact selected." => Some("{}: {}. {} kontak dipilih."),
		"{}: {}. {} contacts selected." => Some("{}: {}. {} kontak dipilih."),
		"selected" => Some("dipilih"),
		"not selected" => Some("tidak dipilih"),
		" {} couldn't be saved." => Some(" {} tidak dapat disimpan."),
		" Downloading {} more." => Some(" Mengunduh {} lainnya."),
		"Saved {} to {}." => Some("{} disimpan ke {}."),
		"{} forwarded to {}." => Some("{} diteruskan ke {}."),
		"{} deleted." => Some("{} dihapus."),
		"{} kept." => Some("{} disimpan."),
		"{} unkept." => Some("Penyimpanan {} dibatalkan."),
		"{} starred." => Some("{} dibintangi."),
		"{} unstarred." => Some("Bintang {} dihapus."),
		"Copied {} to clipboard." => Some("{} disalin ke papan klip."),
		"message" => Some("pesan"),
		"messages" => Some("pesan"),
		"file" => Some("file"),
		"files" => Some("file"),
		"Chat {} marked as read." => Some("Chat {} ditandai sudah dibaca."),
		"Chat {} marked as unread." => Some("Chat {} ditandai belum dibaca."),
		"Notifications from {} muted{}." => Some("Notifikasi dari {} dibisukan{}."),
		"Notifications from {} unmuted." => Some("Notifikasi dari {} dibunyikan."),
		" for 8 hours" => Some(" selama 8 jam"),
		" for 1 week" => Some(" selama 1 minggu"),
		", message content hidden" => Some(", isi pesan disembunyikan"),
		"Notifications for {}: {}{}." => Some("Notifikasi untuk {}: {}{}."),
		"Call {}" => Some("Telepon {}"),
		"Video call {}" => Some("Panggilan video {}"),
		"Chat {} archived." => Some("Chat {} diarsipkan."),
		"Chat {} unarchived." => Some("Arsip chat {} dibatalkan."),
		"Chat {} pinned." => Some("Chat {} disematkan."),
		"Chat {} unpinned." => Some("Sematan chat {} dilepas."),
		"Locked chat {}." => Some("Chat {} dikunci."),
		"Unlocked chat {}." => Some("Kunci chat {} dibuka."),
		"Disappearing messages turned off for {}." => Some("Pesan sementara dimatikan untuk {}."),
		"Disappearing messages set to {} for {}." => Some("Pesan sementara diatur ke {} untuk {}."),
		"Join my WhatsApp {} call: {}" => Some("Gabung ke panggilan {} WhatsApp saya: {}"),
		"voice" => Some("suara"),
		"video" => Some("video"),
		"Anyone with WhatsApp can use this link to join your {} call." => Some("Siapa saja yang memiliki WhatsApp dapat menggunakan tautan ini untuk bergabung ke panggilan {} Anda."),
		"{} wants to join the call. Choose Let in or Don't let in under People." => Some("{} ingin bergabung ke panggilan. Pilih Izinkan masuk atau Tolak masuk di bagian Orang."),
		"{} want to join the call. Choose Let in or Don't let in under People." => Some("{} ingin bergabung ke panggilan. Pilih Izinkan masuk atau Tolak masuk di bagian Orang."),
		"{} raised their hand." => Some("{} mengangkat tangan."),
		"{} lowered their hand." => Some("{} menurunkan tangan."),
		"{} asking to join:" => Some("{} meminta bergabung:"),
		"person" => Some("orang"),
		"people" => Some("orang"),
		"Approve everyone ({})" => Some("Setujui semua ({})"),
		"Reject everyone ({})" => Some("Tolak semua ({})"),
		"Approved {}." => Some("Disetujui {}."),
		"Rejected {}." => Some("Ditolak {}."),
		" {} couldn't be." => Some(" {} tidak dapat diproses."),
		" started by {}" => Some(" dimulai oleh {}"),
		", 1 person in it" => Some(", 1 orang bergabung"),
		", {} people in it" => Some(", {} orang bergabung"),
		"Send message history to {}?\n\n{} will get {} from this group." => Some("Kirim riwayat pesan ke {}?\n\n{} akan menerima {} dari grup ini."),
		"recent message" => Some("pesan terbaru"),
		"recent messages" => Some("pesan terbaru"),
		"Message history sent to {}: {}." => Some("Riwayat pesan dikirim ke {}: {}."),
		", {} unread chat" => Some(", {} chat belum dibaca"),
		", {} unread chats" => Some(", {} chat belum dibaca"),
		" You can send up to {} files at once; {} file weren't sent." => Some(" Anda dapat mengirim maksimal {} file sekaligus; {} file tidak dikirim."),
		" You can send up to {} files at once; {} files weren't sent." => Some(" Anda dapat mengirim maksimal {} file sekaligus; {} file tidak dikirim."),
		"Couldn't block {}: {}" => Some("Tidak dapat memblokir {}: {}"),
		"Couldn't unblock {}: {}" => Some("Tidak dapat membuka blokir {}: {}"),
		"{} blocked." => Some("{} diblokir."),
		"{} unblocked." => Some("Blokir {} dibuka."),
		"{} {} of {}{}" => Some("{} {} dari {}{}"),
		", not downloaded" => Some(", belum diunduh"),
		"Saved {} {} to folder." => Some("{} {} disimpan ke folder."),
		"Sending {} contact..." => Some("Mengirim {} kontak..."),
		"Sending {} contacts..." => Some("Mengirim {} kontak..."),
		"You can send up to {} files at once; {} file weren't added." => Some("Anda dapat mengirim maksimal {} file sekaligus; {} file tidak ditambahkan."),
		"You can send up to {} files at once; {} files weren't added." => Some("Anda dapat mengirim maksimal {} file sekaligus; {} file tidak ditambahkan."),
		"1 group in common" => Some("1 grup bersama"),
		"{} groups in common" => Some("{} grup bersama"),
		"Chats in {}: none yet" => Some("Chat di {}: belum ada"),
		"Chats in {} ({})" => Some("Chat di {} ({})"),
		"Your lists: none yet" => Some("Daftar Anda: belum ada"),
		"Your lists ({})" => Some("Daftar Anda ({})"),
		"{} added to {}." => Some("{} ditambahkan ke {}."),
		"{} removed from {}." => Some("{} dihapus dari {}."),
		"{} added to favorites." => Some("{} ditambahkan ke favorit."),
		"{} removed from favorites." => Some("{} dihapus dari favorit."),
		"1 chat" => Some("1 chat"),
		"{} chats" => Some("{} chat"),
		"Viewed by {} {}" => Some("Dilihat oleh {} {}"),
		"{}'s status updates muted." => Some("Pembaruan status {} dibisukan."),
		"{}'s status updates unmuted." => Some("Pembaruan status {} dibunyikan."),
		"1 view" => Some("1 dilihat"),
		"{} views" => Some("{} dilihat"),
		"photos" => Some("foto"),
		"videos" => Some("video"),
		"media files" => Some("file media"),
		"Products (first {})" => Some("Produk ({} pertama)"),
		"Products ({})" => Some("Produk ({})"),
		" to " => Some(" sampai "),
		", {} on sale" => Some(", {} diskon"),
		", was {}" => Some(", sebelumnya {}"),
		"{} copied: {}" => Some("{} disalin: {}"),
		"Number" => Some("Nomor"),
		"Code" => Some("Kode"),
		"1 option" => Some("1 pilihan"),
		"{} options" => Some("{} pilihan"),
		", phone number {}" => Some(", nomor telepon {}"),
		"Votes: {}" => Some("Suara: {}"),
		"No one has voted in this poll yet." => Some("Belum ada yang memilih di polling ini."),
		"1 vote" => Some("1 suara"),
		"{} votes" => Some("{} suara"),
		"No one has reacted to this post yet." => Some("Belum ada yang bereaksi ke postingan ini."),
		"1 reaction" => Some("1 reaksi"),
		"{} reactions" => Some("{} reaksi"),
		"No admin invites are waiting to be accepted." => Some("Tidak ada undangan admin yang menunggu diterima."),
		"1 invite waiting to be accepted" => Some("1 undangan menunggu diterima"),
		"{} invites waiting to be accepted" => Some("{} undangan menunggu diterima"),
		"no admins besides the owner" => Some("tidak ada admin selain pemilik"),
		"1 admin" => Some("1 admin"),
		"{} admins" => Some("{} admin"),
		"{} shown, {}" => Some("{} ditampilkan, {}"),
		"Loading {} channels{}..." => Some("Memuat saluran {}{}..."),
		"Post {} status. Selected file: {}. Type optional caption, or press Ctrl+Enter to post." => Some("Posting status {}. File yang dipilih: {}. Ketik keterangan opsional, atau tekan Ctrl+Enter untuk memposting."),
		"photo" => Some("foto"),
		"&Viewed by {} {}" => Some("&Dilihat oleh {} {}"),
		"&Add \"{}\" to Dictionary" => Some("&Tambahkan \"{}\" ke Kamus"),
		"&Ignore \"{}\"" => Some("&Abaikan \"{}\""),
		"No chats selected." => Some("Belum ada chat yang dipilih."),
		"1 chat selected." => Some("1 chat dipilih."),
		"{} chats selected." => Some("{} chat dipilih."),
		_ => None,
	}
}
