fn translate_duration_phrase(phrase: &str) -> String {
	if let Some(n) = phrase.strip_suffix(" days") {
		return format!("{n} hari");
	}
	if let Some(n) = phrase.strip_suffix(" hours") {
		return format!("{n} jam");
	}
	if let Some(n) = phrase.strip_suffix(" seconds") {
		return format!("{n} detik");
	}
	phrase.to_string()
}

fn translate_quoted_suffix<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
	let (left, right) = text.split_once(marker)?;
	let inner = right.strip_suffix('"')?;
	Some((left, inner))
}

fn translate_participant(who: &str) -> &str {
	if who == "You" { "Anda" } else { who }
}

pub fn translate_system_message(text: &str) -> String {
	if !crate::i18n::is_indonesian() {
		return text.to_string();
	}
	match text {
		"You received a view once message. For added privacy, you can only open it on your phone." => {
			return "Anda menerima pesan sekali lihat. Demi privasi, pesan ini hanya bisa dibuka di ponsel Anda."
				.to_string();
		}
		"You sent a view once message from another device." => {
			return "Anda mengirim pesan sekali lihat dari perangkat lain.".to_string();
		}
		"You created the group" => return "Anda membuat grup ini".to_string(),
		"You joined using this group's invite link" => {
			return "Anda bergabung melalui tautan undangan grup ini".to_string();
		}
		"You joined the group" => return "Anda bergabung ke grup ini".to_string(),
		"You scheduled a message" => return "Anda menjadwalkan pesan".to_string(),
		"Group created" => return "Grup dibuat".to_string(),
		"Options were removed from a poll" => return "Opsi dihapus dari polling".to_string(),
		"An option added to a poll was removed" => return "Opsi yang ditambahkan ke polling telah dihapus".to_string(),
		_ => {}
	}
	if let Some(rest) = text.strip_prefix("Your security code with ")
		&& let Some(who) = rest.strip_suffix(" changed.")
	{
		return format!("Kode keamanan Anda dengan {who} telah berubah.");
	}
	if let Some(who) = text.strip_suffix(" deleted this group's icon") {
		return format!("{} menghapus ikon grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" changed this group's icon") {
		return format!("{} mengubah ikon grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" turned off disappearing messages.") {
		return format!("{} menonaktifkan pesan sementara.", translate_participant(who));
	}
	if let Some((who, tail)) =
		text.split_once(" turned on disappearing messages. New messages will disappear from this chat ")
		&& let Some(dur) = tail.strip_suffix(" after they're sent.")
	{
		let dur_id = translate_duration_phrase(dur);
		return format!(
			"{} mengaktifkan pesan sementara. Pesan baru akan hilang dari chat ini {dur_id} setelah dikirim.",
			translate_participant(who)
		);
	}
	if let Some(who) = text.strip_suffix(" pinned a message") {
		return format!("{} menyematkan pesan", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" turned on advanced chat privacy") {
		return format!("{} mengaktifkan privasi chat lanjutan", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" turned off advanced chat privacy") {
		return format!("{} menonaktifkan privasi chat lanjutan", translate_participant(who));
	}
	if let Some((who, name)) = translate_quoted_suffix(text, " added this group to the community \"") {
		return format!("{} menambahkan grup ini ke komunitas \"{name}\"", translate_participant(who));
	}
	if let Some(name) = text.strip_prefix("This group was added to the community \"").and_then(|s| s.strip_suffix('"'))
	{
		return format!("Grup ini ditambahkan ke komunitas \"{name}\"");
	}
	if let Some(name) = text.strip_prefix("The community \"").and_then(|s| s.strip_suffix("\" was deactivated")) {
		return format!("Komunitas \"{name}\" telah dinonaktifkan");
	}
	if let Some((who, name)) = translate_quoted_suffix(text, " removed this group from the community \"") {
		return format!("{} mengeluarkan grup ini dari komunitas \"{name}\"", translate_participant(who));
	}
	if let Some(name) =
		text.strip_prefix("This group was removed from the community \"").and_then(|s| s.strip_suffix('"'))
	{
		return format!("Grup ini dikeluarkan dari komunitas \"{name}\"");
	}
	if let Some(who) = text.strip_suffix(" created the group") {
		return format!("{} membuat grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" created this group") {
		return format!("{} membuat grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" added you") {
		return format!("{} menambahkan Anda", translate_participant(who));
	}
	if let Some((who, name)) = translate_quoted_suffix(text, " changed the group name to \"") {
		return format!("{} mengubah nama grup menjadi \"{name}\"", translate_participant(who));
	}
	if let Some(name) = text.strip_prefix("Group name changed to \"").and_then(|s| s.strip_suffix('"')) {
		return format!("Nama grup diubah menjadi \"{name}\"");
	}
	if let Some(who) = text.strip_suffix(" deleted the group description") {
		return format!("{} menghapus deskripsi grup", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" changed the group description") {
		return format!("{} mengubah deskripsi grup", translate_participant(who));
	}
	if let Some(who) =
		text.strip_suffix(" changed this group's settings to allow only admins to send messages to this group")
	{
		return format!(
			"{} mengubah setelan grup agar hanya admin yang dapat mengirim pesan",
			translate_participant(who)
		);
	}
	if let Some(who) =
		text.strip_suffix(" changed this group's settings to allow all members to send messages to this group")
	{
		return format!("{} mengubah setelan grup agar semua anggota dapat mengirim pesan", translate_participant(who));
	}
	if let Some(who) =
		text.strip_suffix(" changed this group's settings to allow only admins to edit this group's info")
	{
		return format!(
			"{} mengubah setelan grup agar hanya admin yang dapat mengedit info grup",
			translate_participant(who)
		);
	}
	if let Some(who) =
		text.strip_suffix(" changed this group's settings to allow all members to edit this group's info")
	{
		return format!(
			"{} mengubah setelan grup agar semua anggota dapat mengedit info grup",
			translate_participant(who)
		);
	}
	if let Some(who) = text.strip_suffix(" turned on admin approval to join this group") {
		return format!("{} mengaktifkan persetujuan admin untuk bergabung ke grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" turned off admin approval to join this group") {
		return format!("{} menonaktifkan persetujuan admin untuk bergabung ke grup ini", translate_participant(who));
	}
	if let Some(who) =
		text.strip_suffix(" changed this group's settings to allow only admins to add others to this group")
	{
		return format!(
			"{} mengubah setelan grup agar hanya admin yang dapat menambahkan anggota",
			translate_participant(who)
		);
	}
	if let Some(who) =
		text.strip_suffix(" changed this group's settings to allow all members to add others to this group")
	{
		return format!(
			"{} mengubah setelan grup agar semua anggota dapat menambahkan anggota",
			translate_participant(who)
		);
	}
	if let Some(who) = text.strip_suffix(" deleted this group") {
		return format!("{} menghapus grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" reset this group's invite link") {
		return format!("{} menyetel ulang tautan undangan grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" joined using this group's invite link") {
		return format!("{} bergabung melalui tautan undangan grup ini", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" joined") {
		return format!("{} bergabung", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" left") {
		return format!("{} keluar", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" is now a group admin") {
		return format!("{} kini menjadi admin grup", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" changed their phone number") {
		return format!("{} mengubah nomor teleponnya", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" is no longer a group admin") {
		return format!("{} tidak lagi menjadi admin grup", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" removed options from a poll") {
		return format!("{} menghapus opsi dari polling", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" removed an option that was added to a poll") {
		return format!("{} menghapus opsi yang ditambahkan ke polling", translate_participant(who));
	}
	if let Some(who) = text.strip_suffix(" shared recent message history") {
		return format!("{} membagikan riwayat pesan terbaru", translate_participant(who));
	}
	if let Some((who, target)) = text.split_once(" shared recent message history with ") {
		return format!("{} membagikan riwayat pesan terbaru dengan {}", translate_participant(who), target);
	}
	if let Some(target) = text.strip_prefix("Recent message history was shared with ") {
		return format!("Riwayat pesan terbaru dibagikan dengan {target}");
	}
	if let Some((who, target)) = text.split_once(" added ") {
		return format!("{} menambahkan {}", translate_participant(who), translate_participant(target));
	}
	if let Some((who, rest)) = text.split_once(" removed ") {
		if let Some(target) = rest.strip_suffix(" as group admin") {
			return format!(
				"{} mencopot {} sebagai admin grup",
				translate_participant(who),
				translate_participant(target)
			);
		}
		return format!("{} mengeluarkan {}", translate_participant(who), translate_participant(rest));
	}
	if let Some((who, rest)) = text.split_once(" made ")
		&& let Some(target) = rest.strip_suffix(" a group admin")
	{
		return format!(
			"{} menjadikan {} sebagai admin grup",
			translate_participant(who),
			translate_participant(target)
		);
	}
	text.to_string()
}

pub fn translate_call_kind_text(text: &str) -> String {
	if !crate::i18n::is_indonesian() {
		return text.to_string();
	}
	let mut out = text.to_string();
	let replacements = [
		("Group video call", "Panggilan video grup"),
		("Group voice call", "Panggilan suara grup"),
		("Video call", "Panggilan video"),
		("Voice call", "Panggilan suara"),
		("Missed group video call", "Panggilan video grup tak terjawab"),
		("Missed group voice call", "Panggilan suara grup tak terjawab"),
		("Missed video call", "Panggilan video tak terjawab"),
		("Missed voice call", "Panggilan suara tak terjawab"),
		("Upcoming group video call", "Panggilan video grup mendatang"),
		("Upcoming group voice call", "Panggilan suara grup mendatang"),
		("Upcoming video call", "Panggilan video mendatang"),
		("Upcoming voice call", "Panggilan suara mendatang"),
		(", answered on another device", ", dijawab di perangkat lain"),
		(", declined on another device", ", ditolak di perangkat lain"),
		(", you declined", ", Anda menolak"),
		(", cancelled", ", dibatalkan"),
		(", declined", ", ditolak"),
		(", unavailable", ", tidak tersedia"),
		(", failed", ", gagal"),
		(", no answer", ", tidak dijawab"),
		(", abandoned", ", terputus"),
	];
	for (en, id) in replacements {
		if out.contains(en) {
			out = out.replace(en, id);
		}
	}
	for (en, id) in [
		("hours", "jam"),
		("hour", "jam"),
		("minutes", "menit"),
		("minute", "menit"),
		("seconds", "detik"),
		("second", "detik"),
	] {
		out = out.replace(&format!(" {en}"), &format!(" {id}"));
	}
	out
}

pub fn translate_rich_message_text(text: &str) -> String {
	if !crate::i18n::is_indonesian() {
		return text.to_string();
	}
	if text == "[Event]" {
		return "Acara".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Event] ") {
		return format!("Acara: {}", translate_event_body(rest));
	}
	if text == "[Event cancelled]" {
		return "Acara dibatalkan".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Event cancelled] ") {
		return format!("Acara dibatalkan: {rest}");
	}
	if text == "[Scheduled Call]" {
		return "Panggilan terjadwal".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Scheduled Call] ") {
		return format!("Panggilan terjadwal: {rest}");
	}
	if text == "[Product]" {
		return "Produk".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Product] ") {
		return format!("Produk: {rest}");
	}
	if text == "[Order]" {
		return "Pesanan".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Order] ") {
		let rest = rest.replace(", out of stock", ", stok habis").replace("Subtotal: ", "Subtotal: ");
		return format!("Pesanan: {rest}");
	}
	if text == "[Poll results]" {
		return "Hasil polling".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Poll results] ") {
		return format!("Hasil polling: {rest}");
	}
	if text == "[Sticker Pack]" {
		return "Paket stiker".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Sticker Pack] ") {
		return format!("Paket stiker: {rest}");
	}
	if text == "[Asked for your phone number]" {
		return "Meminta nomor telepon Anda".to_string();
	}
	if text == "[Payment] Open WhatsApp on your phone to see this payment." {
		return "Pembayaran: buka WhatsApp di ponsel Anda untuk melihat pembayaran ini.".to_string();
	}
	if text == "[Mentioned you in a status update]" {
		return "Menyebut Anda dalam pembaruan status".to_string();
	}
	if text == "[Channel Invite]" {
		return "Undangan saluran".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Channel Invite] ") {
		return format!("Undangan saluran: {rest}");
	}
	if text == "[Group Invite]" {
		return "Undangan grup".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Group Invite] ") {
		return format!("Undangan grup: {rest}");
	}
	if text == "[Channel Admin Invite]" {
		return "Undangan admin saluran".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Channel Admin Invite] ") {
		return format!("Undangan admin saluran: {rest}");
	}
	if text == "[Reaction]" {
		return "Reaksi".to_string();
	}
	if text == "[Protocol Message]" {
		return "Pesan sistem".to_string();
	}
	if text == "[Unsupported Message]" {
		return "Pesan belum didukung".to_string();
	}
	if text == "[Audio]" {
		return "Audio".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Audio] ") {
		return format!("Audio {rest}");
	}
	if text == "[Voice Message]" {
		return "Pesan suara".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Voice Message] ") {
		return format!("Pesan suara {rest}");
	}
	if text == "[Video Note]" {
		return "Catatan video".to_string();
	}
	if let Some(rest) = text.strip_prefix("[Video Note] ") {
		return format!("Catatan video {rest}");
	}
	if text == "[View once] Opened" {
		return "Sekali lihat (sudah dibuka)".to_string();
	}
	if let Some(rest) = text.strip_prefix("[View once] ") {
		return format!("Sekali lihat: {}", translate_rich_message_text(rest));
	}
	if text.starts_with("[Poll] ") || text.starts_with("[Quiz] ") {
		return translate_poll_or_quiz_body(text);
	}
	text.to_string()
}

fn translate_poll_or_quiz_body(text: &str) -> String {
	let mut out = text.to_string();
	out = out.replace(" You got it right: the answer is ", " Jawaban Anda benar: ");
	if let Some(pos) = out.find("Options: ") {
		let (head, tail) = out.split_at(pos);
		let mut tail = tail.replacen("Options: ", "Opsi: ", 1);
		tail = tail.replace(". No votes yet.", ". Belum ada suara.");
		tail = tail.replace(". No votes yet", ". Belum ada suara");
		tail = tail.replace(" people voted", " orang memilih");
		tail = tail.replace(" votes in all", " total suara");
		tail = tail.replace(" votes", " suara").replace(" vote", " suara");
		return format!("{head}{tail}");
	}
	out
}

fn translate_event_body(text: &str) -> String {
	let mut out = text.to_string();
	out = out.replace("No responses yet.", "Belum ada tanggapan.");
	out = out.replace(" You said maybe.", " Anda menjawab mungkin.");
	out = out.replace(" You can't go.", " Anda tidak bisa hadir.");
	out = out.replace(" going", " hadir");
	out = out.replace(" maybe, ", " mungkin, ");
	out = out.replace(" can't go.", " tidak bisa hadir.");
	out = out.replace(" (including ", " (termasuk ").replace(" guests)", " tamu)");
	out
}

pub fn translate_backend_message(msg: &str) -> String {
	if !crate::i18n::is_indonesian() {
		return msg.to_string();
	}
	let exact = match msg {
		"Disconnected: Logged out from primary device." | "Logged out from primary device." => {
			"Terputus: keluar dari perangkat utama."
		}
		"Disconnected: Connection lost. Attempting to reconnect..." => {
			"Terputus: koneksi terputus. Mencoba menghubungkan kembali..."
		}
		"Failed to send QR image" => "Gagal mengirim gambar QR",
		"Can't reach WhatsApp. Check your internet connection." => {
			"Tidak dapat terhubung ke WhatsApp. Periksa koneksi internet Anda."
		}
		"Couldn't complete the secure connection to WhatsApp." => "Tidak dapat menyelesaikan koneksi aman ke WhatsApp.",
		"Couldn't connect to WhatsApp." => "Tidak dapat terhubung ke WhatsApp.",
		"WhatsApp has rejected this version of whatSnap as outdated. Please update whatSnap to reconnect." => {
			"WhatsApp menolak versi whatSnap ini karena sudah kedaluwarsa. Perbarui whatSnap untuk menghubungkan kembali."
		}
		"This version of whatSnap is outdated. Please update it." => {
			"Versi whatSnap ini sudah kedaluwarsa. Silakan perbarui."
		}
		"Couldn't refresh WhatsApp's connection token." => "Tidak dapat menyegarkan token koneksi WhatsApp.",
		"Not responding." => "Tidak merespons.",
		"Calls aren't available yet." => "Fitur panggilan belum tersedia.",
		"That call is no longer available." => "Panggilan tersebut sudah tidak tersedia.",
		"That call has already ended." => "Panggilan tersebut sudah berakhir.",
		"Wait for the call to connect before adding someone." => {
			"Tunggu hingga panggilan terhubung sebelum menambahkan peserta."
		}
		"You can't add yourself to the call." => "Anda tidak dapat menambahkan diri sendiri ke panggilan.",
		"You must configure a Secret Code for Chat Lock on your primary phone before you can lock or unlock chats from this device." => {
			"Atur Kode Rahasia Kunci Chat di ponsel utama Anda terlebih dahulu sebelum mengunci atau membuka chat dari perangkat ini."
		}
		"Only this channel's admins can post in it." => "Hanya admin saluran ini yang dapat memposting.",
		"View once messages can't be forwarded." => "Pesan sekali lihat tidak dapat diteruskan.",
		"This kind of message can't be forwarded." => "Jenis pesan ini tidak dapat diteruskan.",
		"You can only forward to channels you run." => "Anda hanya dapat meneruskan ke saluran yang Anda kelola.",
		"Message not found for forwarding" => "Pesan tidak ditemukan untuk diteruskan",
		"Failed to parse message protobuf for forwarding" => "Gagal membaca data pesan untuk diteruskan",
		"Message not found for retry" => "Pesan tidak ditemukan untuk dikirim ulang",
		"The original file is no longer available to retry." => "File asli tidak lagi tersedia untuk dikirim ulang.",
		"You've already opened this view once message." => "Anda sudah membuka pesan sekali lihat ini.",
		"You've already played this view once voice message." => "Anda sudah memutar pesan suara sekali lihat ini.",
		"View once media opens outside the viewer." => "Media sekali lihat dibuka di luar penampil.",
		"Not on WhatsApp or query failed" => "Tidak terdaftar di WhatsApp atau pemeriksaan gagal",
		"Couldn't load the catalogue. This business may not have one." => {
			"Tidak dapat memuat katalog. Akun bisnis ini mungkin belum memilikinya."
		}
		"Couldn't load the order's details." => "Tidak dapat memuat detail pesanan.",
		"This message's buttons can't be opened here." => "Tombol pesan ini tidak dapat dibuka di sini.",
		"This message has no buttons." => "Pesan ini tidak memiliki tombol.",
		"Couldn't load the pinned messages." => "Tidak dapat memuat pesan yang disematkan.",
		"Channel updated." => "Saluran diperbarui.",
		"Channel picture removed." => "Foto saluran dihapus.",
		"Channel picture changed." => "Foto saluran diubah.",
		"You're now an admin of this channel." => "Anda kini menjadi admin saluran ini.",
		"Admin invite sent." => "Undangan admin terkirim.",
		"Admin invite cancelled." => "Undangan admin dibatalkan.",
		"Admin removed." => "Admin dihapus.",
		"Ownership handed over. You're now an admin." => "Kepemilikan diserahkan. Anda kini menjadi admin.",
		"Post pinned." => "Postingan disematkan.",
		"Post unpinned." => "Sematan postingan dilepas.",
		"There's no one else in this group to call." => "Tidak ada orang lain di grup ini untuk ditelepon.",
		"Choose a date and a time for the message." => "Pilih tanggal dan waktu untuk pesan.",
		"Messages can't be scheduled in this chat." => "Pesan tidak dapat dijadwalkan di chat ini.",
		"Phone could not re-upload the expired media" => "Ponsel tidak dapat mengunggah ulang media yang kedaluwarsa",
		"Phone could not re-upload the media" => "Ponsel tidak dapat mengunggah ulang media",
		"Timeout waiting for phone to re-upload media" => "Waktu habis menunggu ponsel mengunggah ulang media",
		"a poll can have at most twelve options" => "polling maksimal memiliki dua belas pilihan",
		"a poll needs at least two options" => "polling membutuhkan setidaknya dua pilihan",
		"a poll option is too long" => "pilihan polling terlalu panjang",
		"poll options must all be different" => "pilihan polling harus berbeda satu sama lain",
		"the poll needs a question" => "polling membutuhkan pertanyaan",
		"the poll question is too long" => "pertanyaan polling terlalu panjang",
		"the quiz's correct answer isn't one of its options" => {
			"jawaban benar kuis bukan termasuk salah satu pilihannya"
		}
		"WhatsApp hasn't turned on pinning channel posts for your account yet." => {
			"WhatsApp belum mengaktifkan fitur sematkan postingan saluran untuk akun Anda."
		}
		"Can't pin this post: it hasn't finished loading from the channel." => {
			"Belum dapat menyematkan postingan ini karena masih dimuat dari saluran."
		}
		"This post hasn't finished loading from the channel." => "Postingan ini belum selesai dimuat dari saluran.",
		"This poll hasn't finished loading from the channel." => "Polling ini belum selesai dimuat dari saluran.",
		"Can't react to this post: it hasn't finished loading from the channel." => {
			"Belum dapat bereaksi ke postingan ini karena masih dimuat dari saluran."
		}
		"Couldn't find this channel." => "Saluran ini tidak ditemukan.",
		"Couldn't join this group: its community isn't known. Open the community's groups list and try again." => {
			"Tidak dapat bergabung ke grup ini karena komunitasnya belum diketahui. Buka daftar grup komunitas lalu coba lagi."
		}
		"This group needs an admin to approve new members. Your request to join was sent." => {
			"Grup ini memerlukan persetujuan admin untuk anggota baru. Permintaan bergabung Anda telah dikirim."
		}
		"Only group admins can see this group's invite link." => {
			"Hanya admin grup yang dapat melihat tautan undangan grup ini."
		}
		"Some participants have privacy settings that prevent being added directly. An invite link was sent to them instead." => {
			"Beberapa peserta memiliki setelan privasi yang membatasi penambahan langsung. Tautan undangan telah dikirim kepada mereka."
		}
		"This group invite has expired." => "Undangan grup ini sudah kedaluwarsa.",
		"This invite link was reset by a group admin and no longer works." => {
			"Tautan undangan ini telah disetel ulang oleh admin grup dan tidak berlaku lagi."
		}
		"This invite link isn't valid." => "Tautan undangan ini tidak valid.",
		"The pairing code expired. Ask for a new one to try again." => {
			"Kode penautan sudah kedaluwarsa. Minta kode baru untuk mencoba lagi."
		}
		"This event can't be opened here." => "Acara ini tidak dapat dibuka di sini.",
		"This event was cancelled." => "Acara ini dibatalkan.",
		"Events can't be posted in channels." => "Acara tidak dapat diposting di saluran.",
		"This poll can't be opened here." => "Polling ini tidak dapat dibuka di sini.",
		"You've already answered this quiz." => "Anda sudah menjawab kuis ini.",
		"Can't vote in this poll: it hasn't finished loading from the channel." => {
			"Belum dapat memilih di polling ini karena masih dimuat dari saluran."
		}
		"You have reached the maximum number of messages you can schedule." => {
			"Anda telah mencapai batas maksimum pesan terjadwal."
		}
		"That scheduled message is already gone." => "Pesan terjadwal tersebut sudah tidak ada.",
		"Couldn't delete scheduled message." => "Tidak dapat menghapus pesan terjadwal.",
		"Couldn't delete scheduled message. Connect to the internet and try again." => {
			"Tidak dapat menghapus pesan terjadwal. Hubungkan ke internet lalu coba lagi."
		}
		"Couldn't recover this message's text to send it now." => {
			"Tidak dapat memulihkan teks pesan ini untuk dikirim sekarang."
		}
		"That status update isn't here any more." => "Pembaruan status tersebut sudah tidak tersedia.",
		"That isn't a sticker that can be added to favorites." => "Stiker ini tidak dapat ditambahkan ke favorit.",
		"Couldn't load this sticker pack." => "Tidak dapat memuat paket stiker ini.",
		"WhatsApp error 479: Failed to delete for everyone. The message key mismatch or the revoke time limit expired." => {
			"Gagal menghapus untuk semua orang: kunci pesan tidak cocok atau batas waktu penarikan telah habis."
		}
		"not logged in" => "belum masuk",
		"invalid jid" => "JID tidak valid",
		"user info not found" => "info pengguna tidak ditemukan",
		_ => "",
	};
	if !exact.is_empty() {
		return exact.to_string();
	}

	if let Some(rest) = msg.strip_prefix("Participant ")
		&& let Some(num) = rest.strip_suffix(" is not on WhatsApp")
	{
		return format!("Peserta {num} tidak terdaftar di WhatsApp");
	}
	if let Some(rest) = msg.strip_prefix("Admin invites sent to ")
		&& let Some(count) = rest.strip_suffix(" people.")
	{
		return format!("Undangan admin dikirim ke {count} orang.");
	}
	if let Some(name) = msg.strip_suffix(" is already in the call.") {
		return format!("{name} sudah berada dalam panggilan.");
	}
	if let Some(rest) = msg.strip_prefix("Couldn't add ") {
		if let Some(name) = rest.strip_suffix(": the call is full.") {
			return format!("Tidak dapat menambahkan {name}: panggilan sudah penuh.");
		}
		if let Some(name) = rest.strip_suffix(": they have no device that can take a call.") {
			return format!("Tidak dapat menambahkan {name}: tidak ada perangkat yang bisa menerima panggilan.");
		}
		if let Some(name) = rest.strip_suffix(
			": WhatsApp hasn't sent the call details needed to invite someone yet. Try again in a moment.",
		) {
			return format!(
				"Tidak dapat menambahkan {name}: WhatsApp belum mengirim detail panggilan. Coba sebentar lagi."
			);
		}
		return format!("Tidak dapat menambahkan {rest}");
	}
	if let Some(rest) = msg.strip_prefix("This message can only be forwarded to ")
		&& let Some(limit) = rest.strip_suffix(" chat(s) at a time.")
	{
		return format!("Pesan ini hanya dapat diteruskan ke maksimal {limit} chat sekaligus.");
	}
	if let Some((feature, _)) = msg.split_once(" is on in this chat, so it can't be exported.") {
		return format!("{feature} aktif di chat ini sehingga chat tidak dapat diekspor.");
	}
	if let Some((feature, _)) = msg.split_once(" is on in this chat, so Meta AI can't be mentioned here.") {
		return format!("{feature} aktif di chat ini sehingga Meta AI tidak dapat disebut di sini.");
	}

	let prefixes = [
		("Failed to answer call: ", "Gagal menjawab panggilan: "),
		("Failed to delete chat: ", "Gagal menghapus chat: "),
		("Failed to clear chat: ", "Gagal membersihkan chat: "),
		("Failed to leave group: ", "Gagal keluar dari grup: "),
		("Left the group, but failed to delete the chat: ", "Berhasil keluar dari grup, tetapi gagal menghapus chat: "),
		("Failed to post status: ", "Gagal memposting status: "),
		("Logout failed: ", "Gagal keluar akun: "),
		("Could not start pairing: ", "Tidak dapat memulai penautan: "),
		("Pairing failed: ", "Penautan gagal: "),
		("Export failed while finalizing archive: ", "Ekspor gagal saat menyelesaikan arsip: "),
		("Export failed: ", "Ekspor gagal: "),
		("Couldn't change link previews: ", "Tidak dapat mengubah pratinjau tautan: "),
		("Couldn't join the call: ", "Tidak dapat bergabung ke panggilan: "),
		("Couldn't create the call link: ", "Tidak dapat membuat tautan panggilan: "),
		("Couldn't load the groups in common: ", "Tidak dapat memuat grup yang sama: "),
		("Couldn't use the picture: ", "Tidak dapat menggunakan foto tersebut: "),
		("Couldn't create the channel: ", "Tidak dapat membuat saluran: "),
		("Couldn't change the channel: ", "Tidak dapat mengubah saluran: "),
		("Couldn't delete the channel: ", "Tidak dapat menghapus saluran: "),
		("Couldn't load the channel's followers: ", "Tidak dapat memuat pengikut saluran: "),
		("Couldn't change the channel's admins: ", "Tidak dapat mengubah admin saluran: "),
		("Couldn't send an admin invite to ", "Tidak dapat mengirim undangan admin ke "),
		("Couldn't accept the invite: ", "Tidak dapat menerima undangan: "),
		("Couldn't cancel the invite: ", "Tidak dapat membatalkan undangan: "),
		("Couldn't load who reacted: ", "Tidak dapat memuat daftar reaksi: "),
		("Couldn't load who voted: ", "Tidak dapat memuat daftar pemilih: "),
		("Couldn't load the pending admin invites: ", "Tidak dapat memuat undangan admin tertunda: "),
		("Couldn't load similar channels: ", "Tidak dapat memuat saluran serupa: "),
		("Couldn't load this channel's posts: ", "Tidak dapat memuat postingan saluran ini: "),
		("Couldn't load channels: ", "Tidak dapat memuat saluran: "),
		("Couldn't react: ", "Tidak dapat memberi reaksi: "),
		("Couldn't change this channel's mute setting: ", "Tidak dapat mengubah setelan bisukan saluran ini: "),
		("Couldn't load this channel: ", "Tidak dapat memuat saluran ini: "),
		("Couldn't load the community's groups: ", "Tidak dapat memuat grup komunitas: "),
		("Couldn't join this group: ", "Tidak dapat bergabung ke grup ini: "),
		("Couldn't delete the contact: ", "Tidak dapat menghapus kontak: "),
		("Couldn't read the group's settings: ", "Tidak dapat membaca setelan grup: "),
		("Couldn't change a group setting: ", "Tidak dapat mengubah setelan grup: "),
		("Couldn't report to the admins: ", "Tidak dapat melapor ke admin: "),
		("Couldn't load reported messages: ", "Tidak dapat memuat pesan yang dilaporkan: "),
		("Couldn't load member changes: ", "Tidak dapat memuat riwayat perubahan anggota: "),
		("Couldn't load the group: ", "Tidak dapat memuat grup: "),
		("Couldn't send the message history: ", "Tidak dapat mengirim riwayat pesan: "),
		("Couldn't get the invite link: ", "Tidak dapat mengambil tautan undangan: "),
		("Couldn't update participant(s): ", "Tidak dapat memperbarui peserta: "),
		("This invite is no longer valid: ", "Undangan ini tidak lagi valid: "),
		("Couldn't open this invite link: ", "Tidak dapat membuka tautan undangan ini: "),
		("Couldn't forward the media: ", "Tidak dapat meneruskan media: "),
		("Couldn't send the report: ", "Tidak dapat mengirim laporan: "),
		("Couldn't change disappearing messages: ", "Tidak dapat mengubah pesan sementara: "),
		("Couldn't answer the event: ", "Tidak dapat menjawab acara: "),
		("Couldn't send your answer: ", "Tidak dapat mengirim jawaban Anda: "),
		("Can't create the event: ", "Tidak dapat membuat acara: "),
		("Couldn't load the join requests: ", "Tidak dapat memuat permintaan bergabung: "),
		("Couldn't answer the join requests: ", "Tidak dapat menjawab permintaan bergabung: "),
		("Couldn't create the community: ", "Tidak dapat membuat komunitas: "),
		(
			"The community was created, but its description couldn't be set: ",
			"Komunitas berhasil dibuat, tetapi deskripsinya tidak dapat diatur: ",
		),
		("Couldn't load your groups: ", "Tidak dapat memuat grup Anda: "),
		("Couldn't deactivate the community: ", "Tidak dapat menonaktifkan komunitas: "),
		("Couldn't change your tag: ", "Tidak dapat mengubah label Anda: "),
		("Couldn't load the group's members to mention: ", "Tidak dapat memuat daftar anggota untuk disebut: "),
		("Couldn't pin: ", "Tidak dapat menyematkan: "),
		("Couldn't keep the message: ", "Tidak dapat menyimpan pesan: "),
		("Couldn't change advanced chat privacy: ", "Tidak dapat mengubah privasi chat lanjutan: "),
		("Couldn't vote: ", "Tidak dapat memilih: "),
		("Couldn't send the vote: ", "Tidak dapat mengirim pilihan: "),
		("Couldn't save the excluded contacts: ", "Tidak dapat menyimpan kontak yang dikecualikan: "),
		("Couldn't use that photo: ", "Tidak dapat menggunakan foto tersebut: "),
		("Couldn't change your profile photo: ", "Tidak dapat mengubah foto profil Anda: "),
		("Couldn't change the default message timer: ", "Tidak dapat mengubah pengatur waktu pesan bawaan: "),
		("Couldn't react to the status: ", "Tidak dapat bereaksi terhadap status: "),
		("Couldn't like the status: ", "Tidak dapat menyukai status: "),
		("Couldn't delete the status: ", "Tidak dapat menghapus status: "),
		("Couldn't change muted status updates: ", "Tidak dapat mengubah status yang dibisukan: "),
		("Couldn't change who sees your status: ", "Tidak dapat mengubah siapa yang dapat melihat status Anda: "),
		("Couldn't send the sticker: ", "Tidak dapat mengirim stiker: "),
		("Couldn't change your favorite stickers: ", "Tidak dapat mengubah stiker favorit Anda: "),
		("Couldn't load WhatsApp's sticker packs: ", "Tidak dapat memuat paket stiker WhatsApp: "),
		("unknown participant action: ", "aksi peserta tidak dikenal: "),
		("invalid participant number(s), skipped: ", "nomor peserta tidak valid, dilewati: "),
	];
	for (en, id) in prefixes {
		if let Some(rest) = msg.strip_prefix(en) {
			return format!("{id}{rest}");
		}
	}
	if let Some(rest) = msg.strip_prefix("Couldn't ") {
		return format!("Tidak dapat memproses: {rest}");
	}
	msg.to_string()
}
