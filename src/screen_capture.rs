//! Grabs the primary display for a call's screen share: GDI copies the desktop
//! (scaled, with the mouse pointer drawn in) into a fixed-size bitmap, which
//! is then converted to the NV12 the H.264 encoder in `call_video_send.rs`
//! takes. GDI rather than Desktop Duplication because it needs no Direct3D
//! device and its cost is small at a screen share's frame rate.

use windows::Win32::{
	Foundation::POINT,
	Graphics::Gdi::{
		BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
		DeleteDC, DeleteObject, GetDC, HALFTONE, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
		SetBrushOrgEx, SetStretchBltMode, StretchBlt,
	},
	UI::WindowsAndMessaging::{
		CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, GetSystemMetrics, ICONINFO,
		SM_CXCURSOR, SM_CXSCREEN, SM_CYCURSOR, SM_CYSCREEN,
	},
};

/// Where the screen lands inside the output frame: scaled to fit, centred,
/// with black bars filling whatever the aspect ratios leave over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
	pub(crate) x: i32,
	pub(crate) y: i32,
	pub(crate) width: i32,
	pub(crate) height: i32,
}

/// Fits a `screen_w` x `screen_h` display into a `frame_w` x `frame_h` frame.
/// Sizes and offsets come out even, since NV12 stores colour once per 2x2
/// block of pixels.
pub(crate) fn fit(screen_w: i32, screen_h: i32, frame_w: i32, frame_h: i32) -> Placement {
	if screen_w <= 0 || screen_h <= 0 {
		return Placement { x: 0, y: 0, width: frame_w, height: frame_h };
	}
	let (width, height) = if screen_w as i64 * frame_h as i64 >= screen_h as i64 * frame_w as i64 {
		(frame_w, (screen_h as i64 * frame_w as i64 / screen_w as i64) as i32)
	} else {
		((screen_w as i64 * frame_h as i64 / screen_h as i64) as i32, frame_h)
	};
	let width = (width & !1).clamp(2, frame_w);
	let height = (height & !1).clamp(2, frame_h);
	Placement { x: ((frame_w - width) / 2) & !1, y: ((frame_h - height) / 2) & !1, width, height }
}

/// Converts top-down 32-bit BGRA pixels to NV12 (BT.601, video range): a full
/// plane of brightness, then one interleaved U/V pair per 2x2 block, averaged
/// over the block so thin text strokes keep their colour.
pub(crate) fn bgra_to_nv12(bgra: &[u8], width: usize, height: usize, out: &mut [u8]) {
	debug_assert!(width % 2 == 0 && height % 2 == 0);
	debug_assert!(bgra.len() >= width * height * 4 && out.len() >= width * height * 3 / 2);
	let (luma, chroma) = out.split_at_mut(width * height);
	for (row, line) in bgra.chunks_exact(width * 4).take(height).enumerate() {
		let luma_row = &mut luma[row * width..(row + 1) * width];
		for (pixel, y) in line.as_chunks::<4>().0.iter().zip(luma_row.iter_mut()) {
			let (b, g, r) = (pixel[0] as i32, pixel[1] as i32, pixel[2] as i32);
			*y = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
		}
	}
	for block_row in 0..height / 2 {
		let top = &bgra[block_row * 2 * width * 4..];
		let bottom = &bgra[(block_row * 2 + 1) * width * 4..];
		let chroma_row = &mut chroma[block_row * width..(block_row + 1) * width];
		for block in 0..width / 2 {
			let at = block * 8;
			let (mut b, mut g, mut r) = (0i32, 0i32, 0i32);
			for pixel in [&top[at..at + 4], &top[at + 4..at + 8], &bottom[at..at + 4], &bottom[at + 4..at + 8]] {
				b += pixel[0] as i32;
				g += pixel[1] as i32;
				r += pixel[2] as i32;
			}
			let (b, g, r) = ((b + 2) >> 2, (g + 2) >> 2, (r + 2) >> 2);
			chroma_row[block * 2] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
			chroma_row[block * 2 + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
		}
	}
}

/// Whether an NV12 frame is black from edge to edge. That's what a capture
/// returns while a screen reader's screen curtain is up (NVDA's Screen
/// Curtain blacks out everything that reads the screen, not just the
/// monitor), and the person sharing has no other way to find out.
pub(crate) fn is_black(nv12: &[u8], width: usize, height: usize) -> bool {
	// Video-range black is 16; a little headroom above it.
	nv12[..width * height].iter().step_by(7).all(|&luma| luma <= 20)
}

/// The GDI objects of one screen-share session. Everything is created and
/// used on the capture thread.
pub(crate) struct ScreenCapture {
	screen_dc: HDC,
	mem_dc: HDC,
	bitmap: HBITMAP,
	previous: HGDIOBJ,
	pixels: *mut u8,
	width: i32,
	height: i32,
}

impl ScreenCapture {
	pub(crate) fn new(width: u32, height: u32) -> anyhow::Result<Self> {
		unsafe {
			let screen_dc = GetDC(None);
			if screen_dc.is_invalid() {
				anyhow::bail!("couldn't open the screen for capture");
			}
			let mem_dc = CreateCompatibleDC(Some(screen_dc));
			if mem_dc.is_invalid() {
				ReleaseDC(None, screen_dc);
				anyhow::bail!("couldn't create a capture surface");
			}
			let info = BITMAPINFO {
				bmiHeader: BITMAPINFOHEADER {
					biSize: size_of::<BITMAPINFOHEADER>() as u32,
					biWidth: width as i32,
					// Negative: rows run top to bottom, as the converter reads them.
					biHeight: -(height as i32),
					biPlanes: 1,
					biBitCount: 32,
					biCompression: BI_RGB.0,
					..Default::default()
				},
				..Default::default()
			};
			let mut pixels: *mut core::ffi::c_void = std::ptr::null_mut();
			let bitmap = match CreateDIBSection(Some(screen_dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0) {
				Ok(bitmap) if !pixels.is_null() => bitmap,
				_ => {
					let _ = DeleteDC(mem_dc);
					ReleaseDC(None, screen_dc);
					anyhow::bail!("couldn't allocate the capture bitmap");
				}
			};
			let previous = SelectObject(mem_dc, bitmap.into());
			// HALFTONE averages the pixels it drops, which keeps small text
			// readable after the screen is scaled down.
			SetStretchBltMode(mem_dc, HALFTONE);
			let _ = SetBrushOrgEx(mem_dc, 0, 0, None);
			Ok(Self {
				screen_dc,
				mem_dc,
				bitmap,
				previous,
				pixels: pixels as *mut u8,
				width: width as i32,
				height: height as i32,
			})
		}
	}

	/// Copies the screen as it is right now into `out` as one NV12 frame.
	pub(crate) fn grab_nv12(&mut self, out: &mut [u8]) -> anyhow::Result<()> {
		let (screen_w, screen_h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
		let placement = fit(screen_w, screen_h, self.width, self.height);
		let copied = unsafe {
			StretchBlt(
				self.mem_dc,
				placement.x,
				placement.y,
				placement.width,
				placement.height,
				Some(self.screen_dc),
				0,
				0,
				screen_w,
				screen_h,
				SRCCOPY | CAPTUREBLT,
			)
		};
		// Fails while the secure desktop is up (a UAC prompt, the lock
		// screen): the caller sends the previous picture again.
		if !copied.as_bool() {
			anyhow::bail!("the screen couldn't be read");
		}
		self.draw_pointer(placement, screen_w, screen_h);
		let bgra = unsafe { std::slice::from_raw_parts(self.pixels, (self.width * self.height * 4) as usize) };
		bgra_to_nv12(bgra, self.width as usize, self.height as usize, out);
		Ok(())
	}

	/// The mouse pointer isn't part of the desktop image, so it's drawn on
	/// top. Purely cosmetic: any failure just leaves the pointer out.
	fn draw_pointer(&self, placement: Placement, screen_w: i32, screen_h: i32) {
		unsafe {
			let mut cursor = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
			if GetCursorInfo(&mut cursor).is_err() || (cursor.flags.0 & CURSOR_SHOWING.0) == 0 {
				return;
			}
			let mut hotspot = POINT::default();
			let mut icon = ICONINFO::default();
			if GetIconInfo(cursor.hCursor.into(), &mut icon).is_ok() {
				hotspot = POINT { x: icon.xHotspot as i32, y: icon.yHotspot as i32 };
				if !icon.hbmMask.is_invalid() {
					let _ = DeleteObject(icon.hbmMask.into());
				}
				if !icon.hbmColor.is_invalid() {
					let _ = DeleteObject(icon.hbmColor.into());
				}
			}
			let scale = |value: i32, from: i32, to: i32| (value as i64 * to as i64 / from.max(1) as i64) as i32;
			let x = placement.x + scale(cursor.ptScreenPos.x - hotspot.x, screen_w, placement.width);
			let y = placement.y + scale(cursor.ptScreenPos.y - hotspot.y, screen_h, placement.height);
			let width = scale(GetSystemMetrics(SM_CXCURSOR), screen_w, placement.width).max(8);
			let height = scale(GetSystemMetrics(SM_CYCURSOR), screen_h, placement.height).max(8);
			let _ = DrawIconEx(self.mem_dc, x, y, cursor.hCursor.into(), width, height, 0, None, DI_NORMAL);
		}
	}
}

impl Drop for ScreenCapture {
	fn drop(&mut self) {
		unsafe {
			SelectObject(self.mem_dc, self.previous);
			let _ = DeleteObject(self.bitmap.into());
			let _ = DeleteDC(self.mem_dc);
			ReleaseDC(None, self.screen_dc);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_matching_screen_fills_the_frame() {
		assert_eq!(fit(1920, 1080, 1280, 720), Placement { x: 0, y: 0, width: 1280, height: 720 });
	}

	#[test]
	fn a_taller_screen_gets_bars_at_the_sides() {
		assert_eq!(fit(1280, 1024, 1280, 720), Placement { x: 190, y: 0, width: 900, height: 720 });
	}

	#[test]
	fn a_wider_screen_gets_bars_above_and_below() {
		assert_eq!(fit(3440, 1440, 1280, 720), Placement { x: 0, y: 92, width: 1280, height: 534 });
	}

	#[test]
	fn an_unknown_screen_size_uses_the_whole_frame() {
		assert_eq!(fit(0, 0, 1280, 720), Placement { x: 0, y: 0, width: 1280, height: 720 });
	}

	#[test]
	fn converts_flat_colours_to_video_range_nv12() {
		let frame = |b: u8, g: u8, r: u8| {
			let bgra: Vec<u8> = [b, g, r, 0].repeat(4 * 2);
			let mut out = vec![0u8; 4 * 2 * 3 / 2];
			bgra_to_nv12(&bgra, 4, 2, &mut out);
			out
		};
		assert_eq!(frame(0, 0, 0), [16, 16, 16, 16, 16, 16, 16, 16, 128, 128, 128, 128]);
		assert_eq!(frame(255, 255, 255), [235, 235, 235, 235, 235, 235, 235, 235, 128, 128, 128, 128]);
		// Pure red: dark, with V (the second of each pair) pushed to its ceiling.
		assert_eq!(frame(0, 0, 255), [82, 82, 82, 82, 82, 82, 82, 82, 90, 240, 90, 240]);
	}

	#[test]
	fn tells_a_black_frame_from_one_with_a_picture() {
		let mut frame = vec![16u8; 64 * 32 * 3 / 2];
		assert!(is_black(&frame, 64, 32));
		// One lit pixel on a sampled position is a picture.
		frame[7 * 40] = 200;
		assert!(!is_black(&frame, 64, 32));
	}

	/// Needs an unlocked desktop with no screen curtain:
	/// `cargo test grabs_a_real -- --ignored`.
	#[test]
	#[ignore]
	fn grabs_a_real_picture_of_the_screen() {
		let mut capture = ScreenCapture::new(1280, 720).unwrap();
		let mut frame = vec![0u8; 1280 * 720 * 3 / 2];
		capture.grab_nv12(&mut frame).unwrap();
		let luma = &frame[..1280 * 720];
		let (darkest, brightest) = (luma.iter().min().unwrap(), luma.iter().max().unwrap());
		assert!(brightest - darkest > 32, "the grab is one flat shade ({darkest}..{brightest})");
	}

	#[test]
	fn colour_is_averaged_over_each_block() {
		// One 2x2 block: two black and two white pixels average to mid grey.
		let bgra = [0, 0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0, 0];
		let mut out = [0u8; 6];
		bgra_to_nv12(&bgra, 2, 2, &mut out);
		assert_eq!(out, [16, 235, 235, 16, 128, 128]);
	}
}
