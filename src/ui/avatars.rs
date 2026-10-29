//! Profile pictures for the rows the app draws: asked of the backend only for
//! the chats actually drawn, decoded small, and kept for a few hundred chats
//! at most. The backend keeps the files on disk (see avatar_previews.go), so
//! after the first run they come straight back.

use std::{
	cell::RefCell,
	collections::{HashMap, VecDeque},
	sync::mpsc,
	time::{Duration, Instant},
};

use super::gdi::Bitmap;

/// Decoded pictures kept at once. Each is size x size x 4 bytes (~6 KB at
/// 40 px), so this is about 1.5 MB at most.
const MAX_BITMAPS: usize = 256;
/// A request with no answer (offline, say) is asked again after this.
const RETRY_AFTER: Duration = Duration::from_secs(60);

pub(crate) enum Avatar<'a> {
	Picture(&'a Bitmap),
	/// No picture, or one we may not see: the default silhouette.
	Nothing,
	/// Not known yet; it's been asked for.
	Waiting,
}

#[derive(Default)]
struct Cache {
	/// jid -> its file, or None for "has no picture".
	paths: HashMap<String, Option<String>>,
	bitmaps: HashMap<String, Bitmap>,
	order: VecDeque<String>,
	/// The size the bitmaps were made at; a change (DPI, font) drops them.
	size: i32,
	asked: HashMap<String, Instant>,
	pending: Vec<String>,
	flush_scheduled: bool,
	tx: Option<mpsc::Sender<String>>,
	on_change: Vec<Box<dyn Fn()>>,
}

thread_local! {
	static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Sets where requests go. Call once at startup.
pub(crate) fn init(tx: mpsc::Sender<String>) {
	CACHE.with(|c| c.borrow_mut().tx = Some(tx));
}

/// Runs `callback` whenever a picture arrives or changes, to redraw.
pub(crate) fn on_change(callback: impl Fn() + 'static) {
	CACHE.with(|c| c.borrow_mut().on_change.push(Box::new(callback)));
}

/// Runs `draw` with `jid`'s picture at `size` pixels, asking for it if it's
/// not known yet.
pub(crate) fn with_avatar<R>(jid: &str, size: i32, draw: impl FnOnce(Avatar) -> R) -> R {
	CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		if c.size != size {
			c.size = size;
			c.bitmaps.clear();
			c.order.clear();
		}
		let path = match c.paths.get(jid) {
			Some(Some(path)) => path.clone(),
			Some(None) => return draw(Avatar::Nothing),
			None => {
				ask(&mut c, jid);
				return draw(Avatar::Waiting);
			}
		};
		if !c.bitmaps.contains_key(jid) {
			let decoded = crate::image_decode::decode_rgba(&path, (size * 2) as u32)
				.ok()
				.and_then(|(w, h, rgba)| super::gdi::circle_crop(w, h, &rgba, size));
			let Some(bitmap) = decoded else {
				// An unreadable file: draw the silhouette, don't retry each paint.
				c.paths.insert(jid.to_string(), None);
				return draw(Avatar::Nothing);
			};
			c.order.retain(|j| j != jid);
			while c.bitmaps.len() >= MAX_BITMAPS
				&& let Some(old) = c.order.pop_front()
			{
				c.bitmaps.remove(&old);
			}
			c.order.push_back(jid.to_string());
			c.bitmaps.insert(jid.to_string(), bitmap);
		}
		draw(c.bitmaps.get(jid).map_or(Avatar::Nothing, Avatar::Picture))
	})
}

fn ask(c: &mut Cache, jid: &str) {
	if jid.is_empty() || c.asked.get(jid).is_some_and(|t| t.elapsed() < RETRY_AFTER) {
		return;
	}
	c.asked.insert(jid.to_string(), Instant::now());
	c.pending.push(jid.to_string());
	if c.flush_scheduled {
		return;
	}
	c.flush_scheduled = true;
	// After this paint, so a screenful of rows goes as one request.
	wxdragon::call_after(Box::new(|| {
		CACHE.with(|cache| {
			let mut c = cache.borrow_mut();
			c.flush_scheduled = false;
			let jids = std::mem::take(&mut c.pending);
			if let Some(tx) = &c.tx
				&& !jids.is_empty()
			{
				let payload = serde_json::json!({ "action": "get_avatar_previews", "data": { "jids": jids } });
				let _ = tx.send(payload.to_string());
			}
		});
	}));
}

fn changed() {
	// Taken out while they run, so a redraw that asks for a picture can
	// borrow the cache.
	let callbacks = CACHE.with(|c| std::mem::take(&mut c.borrow_mut().on_change));
	for callback in &callbacks {
		callback();
	}
	CACHE.with(|c| {
		let mut c = c.borrow_mut();
		let added = std::mem::replace(&mut c.on_change, callbacks);
		c.on_change.extend(added);
	});
}

/// The backend's `avatar_previews`: {"items": [{"jid", "path"}]}.
pub(crate) fn on_previews(data: &serde_json::Value) {
	let Some(items) = data.get("items").and_then(|i| i.as_array()) else { return };
	CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		for item in items {
			let Some(jid) = item.get("jid").and_then(|j| j.as_str()) else { continue };
			let path = item.get("path").and_then(|p| p.as_str()).filter(|p| !p.is_empty()).map(str::to_string);
			c.asked.remove(jid);
			c.bitmaps.remove(jid);
			c.paths.insert(jid.to_string(), path);
		}
	});
	changed();
}

/// The backend's `avatar_updated`: someone changed or removed their picture.
/// Forgetting it makes the next paint ask again.
pub(crate) fn on_updated(jid: &str) {
	CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		c.paths.remove(jid);
		c.bitmaps.remove(jid);
		c.asked.remove(jid);
	});
	changed();
}
