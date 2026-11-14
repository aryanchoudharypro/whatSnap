use windows::{
	Win32::{
		Foundation::GENERIC_READ,
		Graphics::Imaging::{
			CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapSource, IWICImagingFactory,
			WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
			WICDecodeMetadataCacheOnDemand,
		},
		System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
	},
	core::{HSTRING, Interface},
};

/// The size `width` x `height` scales to so that neither side is over
/// `max_edge`, keeping its shape. Never scales up.
pub(crate) fn fit_within(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
	let longest = width.max(height);
	if longest <= max_edge || longest == 0 {
		return (width, height);
	}
	let scale = |side: u32| ((side as u64 * max_edge as u64 + longest as u64 / 2) / longest as u64).max(1) as u32;
	(scale(width), scale(height))
}

/// The size `width` x `height` scales to so that it fits in a `max_width` x
/// `max_height` box, keeping its shape. Never scales up.
pub(crate) fn fit_in_box(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
	if width == 0 || height == 0 || (width <= max_width && height <= max_height) {
		return (width, height);
	}
	let (w, h, mw, mh) = (width as u64, height as u64, max_width as u64, max_height as u64);
	if w * mh >= h * mw {
		(max_width.max(1), ((h * mw + w / 2) / w).max(1) as u32)
	} else {
		(((w * mh + h / 2) / h).max(1) as u32, max_height.max(1))
	}
}

/// Read the source proportions without allocating or decoding its pixels.
pub(crate) fn dimensions(path: &str) -> crate::error::Result<(u32, u32)> {
	unsafe {
		let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
		let decoder = factory.CreateDecoderFromFilename(
			&HSTRING::from(path),
			None,
			GENERIC_READ,
			WICDecodeMetadataCacheOnDemand,
		)?;
		let frame = decoder.GetFrame(0)?;
		let (mut width, mut height) = (0, 0);
		frame.GetSize(&mut width, &mut height)?;
		Ok((width, height))
	}
}

pub(crate) fn decode_rgba(path: &str, max_edge: u32) -> crate::error::Result<(u32, u32, Vec<u8>)> {
	decode_rgba_with(path, |w, h| fit_within(w, h, max_edge))
}

pub(crate) fn decode_rgba_in_box(
	path: &str,
	max_width: u32,
	max_height: u32,
) -> crate::error::Result<(u32, u32, Vec<u8>)> {
	decode_rgba_with(path, |w, h| fit_in_box(w, h, max_width, max_height))
}

fn decode_rgba_with(path: &str, fit: impl Fn(u32, u32) -> (u32, u32)) -> crate::error::Result<(u32, u32, Vec<u8>)> {
	unsafe {
		let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
		let decoder = factory.CreateDecoderFromFilename(
			&HSTRING::from(path),
			None,
			GENERIC_READ,
			WICDecodeMetadataCacheOnDemand,
		)?;
		let frame = decoder.GetFrame(0)?;
		let (mut width, mut height) = (0u32, 0u32);
		frame.GetSize(&mut width, &mut height)?;
		let (out_w, out_h) = fit(width, height);
		let mut source: IWICBitmapSource = frame.cast()?;
		if (out_w, out_h) != (width, height) {
			let scaler = factory.CreateBitmapScaler()?;
			scaler.Initialize(&source, out_w, out_h, WICBitmapInterpolationModeFant)?;
			source = scaler.cast()?;
		}
		let converter = factory.CreateFormatConverter()?;
		converter.Initialize(
			&source,
			&GUID_WICPixelFormat32bppRGBA,
			WICBitmapDitherTypeNone,
			None,
			0.0,
			WICBitmapPaletteTypeCustom,
		)?;
		let stride = out_w * 4;
		let mut rgba = vec![0u8; stride as usize * out_h as usize];
		converter.CopyPixels(std::ptr::null(), stride, &mut rgba)?;
		Ok((out_w, out_h, rgba))
	}
}

#[cfg(test)]
mod tests {
	use super::fit_within;

	#[test]
	fn shrinks_the_longest_side_and_keeps_the_shape() {
		assert_eq!(fit_within(4000, 3000, 1000), (1000, 750));
		assert_eq!(fit_within(1080, 1920, 960), (540, 960));
	}

	#[test]
	fn never_scales_up() {
		assert_eq!(fit_within(640, 480, 1000), (640, 480));
		assert_eq!(fit_within(0, 0, 1000), (0, 0));
	}

	#[test]
	fn fits_a_box_by_the_side_that_runs_out_first() {
		use super::fit_in_box;
		assert_eq!(fit_in_box(400, 300, 160, 120), (160, 120));
		assert_eq!(fit_in_box(300, 600, 160, 120), (60, 120));
		assert_eq!(fit_in_box(1000, 100, 160, 120), (160, 16));
		assert_eq!(fit_in_box(80, 60, 160, 120), (80, 60));
	}

	#[test]
	fn a_thin_strip_keeps_at_least_one_pixel() {
		assert_eq!(fit_within(10000, 1, 100), (100, 1));
	}
}
