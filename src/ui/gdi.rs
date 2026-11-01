use windows::Win32::{
	Foundation::{COLORREF, RECT, SIZE},
	Graphics::Gdi::{
		AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
		CreateDIBSection, CreateFontIndirectW, CreateSolidBrush, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_END_ELLIPSIS,
		DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FillRect,
		GetTextExtentPoint32W, GetTextFaceW, HBITMAP, HDC, HFONT, HGDIOBJ, LOGFONTW, SelectObject, SetTextColor,
	},
};

pub(crate) fn colorref(r: u8, g: u8, b: u8) -> COLORREF {
	COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub(crate) struct Bitmap {
	handle: HBITMAP,
	pub width: i32,
	pub height: i32,
}

impl Drop for Bitmap {
	fn drop(&mut self) {
		unsafe {
			let _ = DeleteObject(HGDIOBJ(self.handle.0));
		}
	}
}

impl Bitmap {
	pub(crate) fn from_rgba(width: i32, height: i32, rgba: &[u8]) -> Option<Self> {
		if width <= 0 || height <= 0 || rgba.len() < (width * height * 4) as usize {
			return None;
		}
		let info = BITMAPINFO {
			bmiHeader: BITMAPINFOHEADER {
				biSize: size_of::<BITMAPINFOHEADER>() as u32,
				biWidth: width,
				biHeight: -height,
				biPlanes: 1,
				biBitCount: 32,
				biCompression: BI_RGB.0,
				..Default::default()
			},
			..Default::default()
		};
		let mut bits = std::ptr::null_mut();
		let handle = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
		if bits.is_null() {
			unsafe {
				let _ = DeleteObject(HGDIOBJ(handle.0));
			}
			return None;
		}
		let out = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), (width * height * 4) as usize) };
		for (dst, src) in out.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
			let a = src[3] as u32;
			dst[0] = ((src[2] as u32 * a + 127) / 255) as u8;
			dst[1] = ((src[1] as u32 * a + 127) / 255) as u8;
			dst[2] = ((src[0] as u32 * a + 127) / 255) as u8;
			dst[3] = a as u8;
		}
		Some(Self { handle, width, height })
	}

	pub(crate) fn draw(&self, dc: HDC, x: i32, y: i32) {
		unsafe {
			let mem = CreateCompatibleDC(Some(dc));
			let old = SelectObject(mem, HGDIOBJ(self.handle.0));
			let blend = BLENDFUNCTION {
				BlendOp: AC_SRC_OVER as u8,
				BlendFlags: 0,
				SourceConstantAlpha: 255,
				AlphaFormat: AC_SRC_ALPHA as u8,
			};
			let _ = AlphaBlend(dc, x, y, self.width, self.height, mem, 0, 0, self.width, self.height, blend);
			SelectObject(mem, old);
			let _ = DeleteDC(mem);
		}
	}
}

pub(crate) fn round_rect_coverage(x: i32, y: i32, w: i32, h: i32, radius: f32) -> f32 {
	let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
	let cx = px.clamp(radius, w as f32 - radius);
	let cy = py.clamp(radius, h as f32 - radius);
	let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
	(radius + 0.5 - dist).clamp(0.0, 1.0)
}

pub(crate) fn round_rect(w: i32, h: i32, radius: f32, colour: (u8, u8, u8)) -> Option<Bitmap> {
	Bitmap::from_rgba(w, h, &round_rect_rgba(w, h, radius, colour, 255))
}

pub(crate) fn round_rect_rgba(w: i32, h: i32, radius: f32, (r, g, b): (u8, u8, u8), alpha: u8) -> Vec<u8> {
	let mut rgba = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
	for y in 0..h {
		for x in 0..w {
			let i = ((y * w + x) * 4) as usize;
			rgba[i] = r;
			rgba[i + 1] = g;
			rgba[i + 2] = b;
			rgba[i + 3] = (round_rect_coverage(x, y, w, h, radius) * alpha as f32) as u8;
		}
	}
	rgba
}

pub(crate) fn circle_crop(width: u32, height: u32, rgba: &[u8], size: i32) -> Option<Bitmap> {
	if width == 0 || height == 0 || size <= 0 {
		return None;
	}
	let side = width.min(height) as f32;
	let (x0, y0) = ((width as f32 - side) / 2.0, (height as f32 - side) / 2.0);
	let step = side / size as f32;
	let mut out = vec![0u8; (size * size * 4) as usize];
	for oy in 0..size {
		for ox in 0..size {
			let sx0 = (x0 + ox as f32 * step) as u32;
			let sy0 = (y0 + oy as f32 * step) as u32;
			let sx1 = ((x0 + (ox + 1) as f32 * step) as u32).clamp(sx0 + 1, width);
			let sy1 = ((y0 + (oy + 1) as f32 * step) as u32).clamp(sy0 + 1, height);
			let mut sum = [0u32; 3];
			let mut n = 0u32;
			for sy in sy0..sy1 {
				for sx in sx0..sx1 {
					let i = ((sy * width + sx) * 4) as usize;
					sum[0] += rgba[i] as u32;
					sum[1] += rgba[i + 1] as u32;
					sum[2] += rgba[i + 2] as u32;
					n += 1;
				}
			}
			let i = ((oy * size + ox) * 4) as usize;
			let n = n.max(1);
			out[i] = (sum[0] / n) as u8;
			out[i + 1] = (sum[1] / n) as u8;
			out[i + 2] = (sum[2] / n) as u8;
			out[i + 3] = (round_rect_coverage(ox, oy, size, size, size as f32 / 2.0) * 255.0) as u8;
		}
	}
	Bitmap::from_rgba(size, size, &out)
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
	text.encode_utf16().collect()
}

pub(crate) fn fill(dc: HDC, rect: RECT, colour: COLORREF) {
	unsafe {
		let brush = CreateSolidBrush(colour);
		FillRect(dc, &rect, brush);
		let _ = DeleteObject(HGDIOBJ(brush.0));
	}
}

pub(crate) fn text_width(dc: HDC, font: HFONT, text: &str) -> i32 {
	if text.is_empty() {
		return 0;
	}
	unsafe {
		let old = SelectObject(dc, HGDIOBJ(font.0));
		let mut size = SIZE::default();
		let _ = GetTextExtentPoint32W(dc, &wide(text), &mut size);
		SelectObject(dc, old);
		size.cx
	}
}

pub(crate) fn text(dc: HDC, font: HFONT, colour: COLORREF, text: &str, mut rect: RECT, extra: DRAW_TEXT_FORMAT) {
	if text.is_empty() || rect.right <= rect.left {
		return;
	}
	unsafe {
		let old = SelectObject(dc, HGDIOBJ(font.0));
		SetTextColor(dc, colour);
		let mut chars = wide(text);
		DrawTextW(
			dc,
			&mut chars,
			&mut rect,
			DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | DT_LEFT | extra,
		);
		SelectObject(dc, old);
	}
}

pub(crate) fn derived_font(base: &LOGFONTW, scale: f32, weight: i32) -> HFONT {
	let mut lf = *base;
	lf.lfHeight = (lf.lfHeight as f32 * scale).round() as i32;
	lf.lfWidth = 0;
	lf.lfWeight = weight;
	unsafe { CreateFontIndirectW(&lf) }
}

pub(crate) fn icon_font(dc: HDC, base: &LOGFONTW, scale: f32) -> Option<HFONT> {
	for face in ["Segoe Fluent Icons", "Segoe MDL2 Assets"] {
		let mut lf = *base;
		lf.lfHeight = (lf.lfHeight as f32 * scale).round() as i32;
		lf.lfWidth = 0;
		lf.lfWeight = 400;
		lf.lfItalic = 0;
		lf.lfFaceName = [0; 32];
		for (dst, src) in lf.lfFaceName.iter_mut().zip(face.encode_utf16()) {
			*dst = src;
		}
		let font = unsafe { CreateFontIndirectW(&lf) };
		if font.is_invalid() {
			continue;
		}
		let mut got = [0u16; 32];
		unsafe {
			let old = SelectObject(dc, HGDIOBJ(font.0));
			GetTextFaceW(dc, Some(&mut got));
			SelectObject(dc, old);
		}
		let end = got.iter().position(|&c| c == 0).unwrap_or(got.len());
		let got = String::from_utf16_lossy(&got[..end]);
		if got.eq_ignore_ascii_case(face) {
			return Some(font);
		}
		unsafe {
			let _ = DeleteObject(HGDIOBJ(font.0));
		}
	}
	None
}

pub(crate) fn face_installed(face: &str) -> bool {
	unsafe {
		let dc = windows::Win32::Graphics::Gdi::GetDC(None);
		let mut lf = LOGFONTW { lfHeight: -12, lfWeight: 400, ..Default::default() };
		for (dst, src) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
			*dst = src;
		}
		let font = CreateFontIndirectW(&lf);
		let mut got = [0u16; 32];
		let old = SelectObject(dc, HGDIOBJ(font.0));
		GetTextFaceW(dc, Some(&mut got));
		SelectObject(dc, old);
		delete_font(font);
		windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
		let end = got.iter().position(|&c| c == 0).unwrap_or(got.len());
		String::from_utf16_lossy(&got[..end]).eq_ignore_ascii_case(face)
	}
}

pub(crate) fn delete_font(font: HFONT) {
	if !font.is_invalid() {
		unsafe {
			let _ = DeleteObject(HGDIOBJ(font.0));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::round_rect_coverage;

	#[test]
	fn a_circle_covers_its_middle_and_not_its_corners() {
		assert_eq!(round_rect_coverage(20, 20, 40, 40, 20.0), 1.0);
		assert_eq!(round_rect_coverage(0, 0, 40, 40, 20.0), 0.0);
		let edge = round_rect_coverage(0, 20, 40, 40, 20.0);
		assert!(edge > 0.0 && edge <= 1.0);
	}
}
