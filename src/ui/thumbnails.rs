//! The small pictures of photos, videos and stickers the message list draws:
//! asked of the backend only for the rows on screen (see thumbnails.go),
//! decoded to the size they're drawn at, and kept for a few dozen messages.

use std::{
	cell::RefCell,
	collections::{HashMap, VecDeque},
	sync::mpsc,
	time::{Duration, Instant},
};

use wxdragon::bitmap::Bitmap;

/// Decoded pictures kept at once, each at most about 200 x 150 x 4 bytes.
const MAX_BITMAPS: usize = 64;
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Known file paths kept before the ones not on screen are forgotten (they
/// are asked for again if scrolled back to), so paging through a big chat's
/// media all session doesn't keep one path per message seen.
const MAX_PATHS: usize = 2048;

pub(crate) enum Thumb<'a> {
	Picture(&'a Bitmap),
	/// Nothing to show (view once, no preview): the placeholder stays.
	Nothing,
	Waiting,
}

#[derive(Default)]
struct Cache {
	paths: HashMap<String, Option<String>>,
	/// Each with the box it was made for; another box makes it again.
	bitmaps: HashMap<String, ((i32, i32), Bitmap)>,
	order: VecDeque<String>,
	asked: HashMap<String, Instant>,
	pending: Vec<String>,
	flush_scheduled: bool,
	tx: Option<mpsc::Sender<String>>,
	on_change: Option<Box<dyn Fn()>>,
}

thread_local! {
	static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

pub(crate) fn init(tx: mpsc::Sender<String>, on_change: impl Fn() + 'static) {
	CACHE.with(|c| {
		let mut c = c.borrow_mut();
		c.tx = Some(tx);
		c.on_change = Some(Box::new(on_change));
	});
}

/// The file of message `id`'s picture: Some(None) when it has none, None
/// while it's not known yet (it's asked for).
pub(crate) fn path_of(id: &str) -> Option<Option<String>> {
	CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		let known = c.paths.get(id).cloned();
		if known.is_none() {
			ask(&mut c, id);
		}
		known
	})
}

/// Runs `draw` with message `id`'s picture, fitted in `max_w` x `max_h`.
pub(crate) fn with_thumb<R>(id: &str, max_w: i32, max_h: i32, draw: impl FnOnce(Thumb) -> R) -> R {
	CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		let path = match c.paths.get(id) {
			Some(Some(path)) => path.clone(),
			Some(None) => return draw(Thumb::Nothing),
			None => {
				ask(&mut c, id);
				return draw(Thumb::Waiting);
			}
		};
		if c.bitmaps.get(id).is_none_or(|(size, _)| *size != (max_w, max_h)) {
			let decoded =
				crate::media::image_decode::decode_rgba_in_box(&path, max_w.max(1) as u32, max_h.max(1) as u32)
					.ok()
					.and_then(|(w, h, rgba)| Bitmap::from_rgba(&rgba, w, h));
			let Some(bitmap) = decoded else {
				c.paths.insert(id.to_string(), None);
				return draw(Thumb::Nothing);
			};
			c.order.retain(|k| k != id);
			while c.bitmaps.len() >= MAX_BITMAPS
				&& let Some(old) = c.order.pop_front()
			{
				c.bitmaps.remove(&old);
			}
			c.order.push_back(id.to_string());
			c.bitmaps.insert(id.to_string(), ((max_w, max_h), bitmap));
		}
		draw(c.bitmaps.get(id).map_or(Thumb::Nothing, |(_, bitmap)| Thumb::Picture(bitmap)))
	})
}

fn ask(c: &mut Cache, id: &str) {
	if id.is_empty() || c.asked.get(id).is_some_and(|t| t.elapsed() < RETRY_AFTER) {
		return;
	}
	if c.asked.len() >= MAX_PATHS {
		c.asked.retain(|_, t| t.elapsed() < RETRY_AFTER);
	}
	c.asked.insert(id.to_string(), Instant::now());
	c.pending.push(id.to_string());
	if c.flush_scheduled {
		return;
	}
	c.flush_scheduled = true;
	wxdragon::call_after(Box::new(|| {
		CACHE.with(|cache| {
			let mut c = cache.borrow_mut();
			c.flush_scheduled = false;
			// The newest rows are at the bottom of the screen, where the
			// user usually is: those first.
			let mut ids = std::mem::take(&mut c.pending);
			ids.reverse();
			if let Some(tx) = &c.tx
				&& !ids.is_empty()
			{
				let payload = serde_json::json!({ "action": "get_thumbnails", "data": { "ids": ids } });
				let _ = tx.send(payload.to_string());
			}
		});
	}));
}

/// The backend's `thumbnails`: {"items": [{"id", "path"}]}.
pub(crate) fn on_thumbnails(data: &serde_json::Value) {
	let Some(items) = data.get("items").and_then(|i| i.as_array()) else { return };
	let on_change = CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		if c.paths.len() >= MAX_PATHS {
			let Cache { paths, bitmaps, .. } = &mut *c;
			paths.retain(|id, _| bitmaps.contains_key(id));
		}
		for item in items {
			let Some(id) = item.get("id").and_then(|j| j.as_str()) else { continue };
			let path = item.get("path").and_then(|p| p.as_str()).filter(|p| !p.is_empty()).map(str::to_string);
			c.asked.remove(id);
			c.bitmaps.remove(id);
			c.paths.insert(id.to_string(), path);
		}
		c.on_change.take()
	});
	if let Some(callback) = on_change {
		callback();
		CACHE.with(|c| c.borrow_mut().on_change = Some(callback));
	}
}

/// A message's media finished downloading: a sharper picture can be made.
pub(crate) fn forget(id: &str) {
	CACHE.with(|cache| {
		let mut c = cache.borrow_mut();
		c.paths.remove(id);
		c.bitmaps.remove(id);
		c.asked.remove(id);
	});
}

/// Voice notes' loudness bars (0-100), by message id: a few dozen bytes each.
#[derive(Default)]
struct Waveforms {
	bars: HashMap<String, Vec<u8>>,
	order: VecDeque<String>,
	asked: HashMap<String, Instant>,
	pending: Vec<String>,
	flush_scheduled: bool,
}

const MAX_WAVEFORMS: usize = 512;

thread_local! {
	static WAVEFORMS: RefCell<Waveforms> = RefCell::new(Waveforms::default());
}

/// Runs `draw` with voice note `id`'s bars, None until they've come (or
/// when it has none).
pub(crate) fn with_waveform<R>(id: &str, draw: impl FnOnce(Option<&[u8]>) -> R) -> R {
	WAVEFORMS.with(|w| {
		let mut w = w.borrow_mut();
		if !w.bars.contains_key(id) && !id.is_empty() && w.asked.get(id).is_none_or(|t| t.elapsed() >= RETRY_AFTER) {
			w.asked.insert(id.to_string(), Instant::now());
			w.pending.push(id.to_string());
			if !w.flush_scheduled {
				w.flush_scheduled = true;
				wxdragon::call_after(Box::new(|| {
					let ids = WAVEFORMS.with(|w| {
						let mut w = w.borrow_mut();
						w.flush_scheduled = false;
						std::mem::take(&mut w.pending)
					});
					CACHE.with(|c| {
						if let Some(tx) = &c.borrow().tx {
							let payload = serde_json::json!({ "action": "get_waveforms", "data": { "ids": ids } });
							let _ = tx.send(payload.to_string());
						}
					});
				}));
			}
		}
		draw(w.bars.get(id).map(Vec::as_slice).filter(|b| !b.is_empty()))
	})
}

/// The backend's `waveforms`: {"items": [{"id", "bars": [0-100, ...]}]}.
pub(crate) fn on_waveforms(data: &serde_json::Value) {
	let Some(items) = data.get("items").and_then(|i| i.as_array()) else { return };
	WAVEFORMS.with(|w| {
		let mut w = w.borrow_mut();
		for item in items {
			let Some(id) = item.get("id").and_then(|j| j.as_str()) else { continue };
			let bars: Vec<u8> = item
				.get("bars")
				.and_then(|b| b.as_array())
				.map(|b| b.iter().filter_map(|v| v.as_u64()).map(|v| v.min(100) as u8).collect())
				.unwrap_or_default();
			while w.bars.len() >= MAX_WAVEFORMS
				&& let Some(old) = w.order.pop_front()
			{
				w.bars.remove(&old);
			}
			w.asked.remove(id);
			w.order.push_back(id.to_string());
			w.bars.insert(id.to_string(), bars);
		}
	});
	let on_change = CACHE.with(|c| c.borrow_mut().on_change.take());
	if let Some(callback) = on_change {
		callback();
		CACHE.with(|c| c.borrow_mut().on_change = Some(callback));
	}
}
