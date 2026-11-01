use windows::{
	Win32::{
		Foundation::{HWND, LPARAM, POINT, RECT, TRUE},
		Graphics::{
			Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
			Gdi::{
				BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACKNESS, CAPTUREBLT, CreateCompatibleBitmap,
				CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors,
				GetDC, GetMonitorInfoW, HALFTONE, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, PatBlt, ReleaseDC,
				SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
			},
		},
		Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
		UI::WindowsAndMessaging::{
			CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetCursorInfo,
			GetIconInfo, GetSystemMetrics, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextW, ICONINFO,
			IsIconic, IsWindow, IsWindowVisible, SM_CXCURSOR, SM_CXSCREEN, SM_CYCURSOR, SM_CYSCREEN, WS_EX_APPWINDOW,
			WS_EX_TOOLWINDOW,
		},
	},
	core::BOOL,
};

const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
const MONITORINFOF_PRIMARY: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareTarget {
	Monitor { left: i32, top: i32, width: i32, height: i32 },
	Window(isize),
}

#[derive(Debug, Clone)]
pub struct ShareChoice {
	pub target: ShareTarget,
	pub label: String,
	pub window_title: Option<String>,
}

pub fn share_choices() -> Vec<ShareChoice> {
	let mut monitors: Vec<(bool, ShareTarget)> = Vec::new();
	unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
		let monitors = unsafe { &mut *(data.0 as *mut Vec<(bool, ShareTarget)>) };
		let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
		if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
			let r = info.rcMonitor;
			monitors.push((
				info.dwFlags & MONITORINFOF_PRIMARY != 0,
				ShareTarget::Monitor { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top },
			));
		}
		TRUE
	}
	unsafe {
		let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut monitors as *mut _ as isize));
	}
	if monitors.is_empty() {
		let (width, height) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
		monitors.push((true, ShareTarget::Monitor { left: 0, top: 0, width, height }));
	}
	monitors.sort_by_key(|(primary, _)| !primary);

	let mut choices: Vec<ShareChoice> = Vec::new();
	let several = monitors.len() > 1;
	for (i, (_, target)) in monitors.into_iter().enumerate() {
		let label = match target {
			ShareTarget::Monitor { width, height, .. } if several => format!("Screen {} ({width} x {height})", i + 1),
			_ => "Entire screen".to_string(),
		};
		choices.push(ShareChoice { target, label, window_title: None });
	}

	let mut windows: Vec<HWND> = Vec::new();
	unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> BOOL {
		let windows = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
		if is_shareable_window(hwnd) {
			windows.push(hwnd);
		}
		TRUE
	}
	unsafe {
		let _ = EnumWindows(Some(collect_window), LPARAM(&mut windows as *mut _ as isize));
	}
	for hwnd in windows {
		let title = window_title(hwnd);
		choices.push(ShareChoice {
			target: ShareTarget::Window(hwnd.0 as isize),
			label: format!("Window: {title}"),
			window_title: Some(title),
		});
	}
	choices
}

fn window_title(hwnd: HWND) -> String {
	let mut buf = [0u16; 256];
	let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
	String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

fn is_shareable_window(hwnd: HWND) -> bool {
	unsafe {
		if !IsWindowVisible(hwnd).as_bool() || window_title(hwnd).trim().is_empty() {
			return false;
		}
		let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
		let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
		if style & WS_EX_TOOLWINDOW.0 != 0 || (owned && style & WS_EX_APPWINDOW.0 == 0) {
			return false;
		}
		let mut cloaked = 0u32;
		let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, size_of::<u32>() as u32);
		cloaked == 0
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
	pub(crate) x: i32,
	pub(crate) y: i32,
	pub(crate) width: i32,
	pub(crate) height: i32,
}

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
	nv12[..width * height].iter().step_by(7).all(|&luma| luma <= 20)
}

struct WindowSurface {
	dc: HDC,
	bitmap: HBITMAP,
	previous: HGDIOBJ,
	width: i32,
	height: i32,
}

impl Drop for WindowSurface {
	fn drop(&mut self) {
		unsafe {
			SelectObject(self.dc, self.previous);
			let _ = DeleteObject(self.bitmap.into());
			let _ = DeleteDC(self.dc);
		}
	}
}

pub(crate) struct ScreenCapture {
	target: ShareTarget,
	screen_dc: HDC,
	mem_dc: HDC,
	bitmap: HBITMAP,
	previous: HGDIOBJ,
	pixels: *mut u8,
	width: i32,
	height: i32,
	surface: Option<WindowSurface>,
}

impl ScreenCapture {
	pub(crate) fn new(target: ShareTarget, width: u32, height: u32) -> crate::error::Result<Self> {
		if let ShareTarget::Window(hwnd) = target
			&& !window_alive(hwnd)
		{
			crate::error::bail!("the window to share is gone");
		}
		unsafe {
			let screen_dc = GetDC(None);
			if screen_dc.is_invalid() {
				crate::error::bail!("couldn't open the screen for capture");
			}
			let mem_dc = CreateCompatibleDC(Some(screen_dc));
			if mem_dc.is_invalid() {
				ReleaseDC(None, screen_dc);
				crate::error::bail!("couldn't create a capture surface");
			}
			let info = BITMAPINFO {
				bmiHeader: BITMAPINFOHEADER {
					biSize: size_of::<BITMAPINFOHEADER>() as u32,
					biWidth: width as i32,
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
					crate::error::bail!("couldn't allocate the capture bitmap");
				}
			};
			let previous = SelectObject(mem_dc, bitmap.into());
			SetStretchBltMode(mem_dc, HALFTONE);
			let _ = SetBrushOrgEx(mem_dc, 0, 0, None);
			Ok(Self {
				target,
				screen_dc,
				mem_dc,
				bitmap,
				previous,
				pixels: pixels as *mut u8,
				width: width as i32,
				height: height as i32,
				surface: None,
			})
		}
	}

	pub(crate) fn target_gone(&self) -> bool {
		matches!(self.target, ShareTarget::Window(hwnd) if !window_alive(hwnd))
	}

	pub(crate) fn grab_nv12(&mut self, out: &mut [u8]) -> crate::error::Result<()> {
		let (origin, size, placement) = match self.target {
			ShareTarget::Monitor { left, top, width, height } => {
				let placement = fit(width, height, self.width, self.height);
				let copied = unsafe {
					StretchBlt(
						self.mem_dc,
						placement.x,
						placement.y,
						placement.width,
						placement.height,
						Some(self.screen_dc),
						left,
						top,
						width,
						height,
						SRCCOPY | CAPTUREBLT,
					)
				};
				if !copied.as_bool() {
					crate::error::bail!("the screen couldn't be read");
				}
				((left, top), (width, height), placement)
			}
			ShareTarget::Window(hwnd) => self.grab_window(HWND(hwnd as *mut _))?,
		};
		self.draw_pointer(placement, origin, size);
		let bgra = unsafe { std::slice::from_raw_parts(self.pixels, (self.width * self.height * 4) as usize) };
		bgra_to_nv12(bgra, self.width as usize, self.height as usize, out);
		Ok(())
	}

	#[allow(clippy::type_complexity)]
	fn grab_window(&mut self, hwnd: HWND) -> crate::error::Result<((i32, i32), (i32, i32), Placement)> {
		unsafe {
			let mut rect = RECT::default();
			if GetWindowRect(hwnd, &mut rect).is_err() || IsIconic(hwnd).as_bool() {
				crate::error::bail!("the window can't be read right now");
			}
			let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
			if width <= 0 || height <= 0 {
				crate::error::bail!("the window can't be read right now");
			}
			if self.surface.as_ref().is_none_or(|surface| surface.width != width || surface.height != height) {
				self.surface = None;
				let dc = CreateCompatibleDC(Some(self.screen_dc));
				let bitmap = CreateCompatibleBitmap(self.screen_dc, width, height);
				if dc.is_invalid() || bitmap.is_invalid() {
					if !dc.is_invalid() {
						let _ = DeleteDC(dc);
					}
					if !bitmap.is_invalid() {
						let _ = DeleteObject(bitmap.into());
					}
					crate::error::bail!("couldn't allocate a surface for the window");
				}
				let previous = SelectObject(dc, bitmap.into());
				self.surface = Some(WindowSurface { dc, bitmap, previous, width, height });
			}
			let surface = self.surface.as_ref().expect("surface was just made");
			if !PrintWindow(hwnd, surface.dc, PW_RENDERFULLCONTENT).as_bool() {
				crate::error::bail!("the window couldn't be read");
			}
			let placement = fit(width, height, self.width, self.height);
			let _ = PatBlt(self.mem_dc, 0, 0, self.width, self.height, BLACKNESS);
			let copied = StretchBlt(
				self.mem_dc,
				placement.x,
				placement.y,
				placement.width,
				placement.height,
				Some(surface.dc),
				0,
				0,
				width,
				height,
				SRCCOPY,
			);
			if !copied.as_bool() {
				crate::error::bail!("the window couldn't be read");
			}
			Ok(((rect.left, rect.top), (width, height), placement))
		}
	}

	fn draw_pointer(&self, placement: Placement, origin: (i32, i32), size: (i32, i32)) {
		unsafe {
			let mut cursor = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
			if GetCursorInfo(&mut cursor).is_err() || (cursor.flags.0 & CURSOR_SHOWING.0) == 0 {
				return;
			}
			let (source_w, source_h) = size;
			let at = (cursor.ptScreenPos.x - origin.0, cursor.ptScreenPos.y - origin.1);
			if at.0 < 0 || at.1 < 0 || at.0 >= source_w || at.1 >= source_h {
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
			let x = placement.x + scale(at.0 - hotspot.x, source_w, placement.width);
			let y = placement.y + scale(at.1 - hotspot.y, source_h, placement.height);
			let width = scale(GetSystemMetrics(SM_CXCURSOR), source_w, placement.width).max(8);
			let height = scale(GetSystemMetrics(SM_CYCURSOR), source_h, placement.height).max(8);
			let _ = DrawIconEx(self.mem_dc, x, y, cursor.hCursor.into(), width, height, 0, None, DI_NORMAL);
		}
	}
}

fn window_alive(hwnd: isize) -> bool {
	unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}

impl Drop for ScreenCapture {
	fn drop(&mut self) {
		self.surface = None;
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
		assert_eq!(frame(0, 0, 255), [82, 82, 82, 82, 82, 82, 82, 82, 90, 240, 90, 240]);
	}

	#[test]
	fn tells_a_black_frame_from_one_with_a_picture() {
		let mut frame = vec![16u8; 64 * 32 * 3 / 2];
		assert!(is_black(&frame, 64, 32));
		frame[7 * 40] = 200;
		assert!(!is_black(&frame, 64, 32));
	}

	#[test]
	#[ignore]
	fn grabs_a_real_picture_of_the_screen() {
		let primary = ShareTarget::Monitor { left: 0, top: 0, width: 1920, height: 1080 };
		let mut capture = ScreenCapture::new(primary, 1280, 720).unwrap();
		let mut frame = vec![0u8; 1280 * 720 * 3 / 2];
		capture.grab_nv12(&mut frame).unwrap();
		let luma = &frame[..1280 * 720];
		let (darkest, brightest) = (luma.iter().min().unwrap(), luma.iter().max().unwrap());
		assert!(brightest - darkest > 32, "the grab is one flat shade ({darkest}..{brightest})");
	}

	#[test]
	fn colour_is_averaged_over_each_block() {
		let bgra = [0, 0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 0, 0, 0, 0, 0];
		let mut out = [0u8; 6];
		bgra_to_nv12(&bgra, 2, 2, &mut out);
		assert_eq!(out, [16, 235, 235, 16, 128, 128]);
	}
}
