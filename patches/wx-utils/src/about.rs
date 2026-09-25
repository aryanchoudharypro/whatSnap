use std::ffi::CString;

use wxdragon::{ffi, prelude::*};

pub struct AboutBoxBuilder<'a> {
	parent: &'a Frame,
	name: Option<String>,
	version: Option<String>,
	description: Option<String>,
	copyright: Option<String>,
	website: Option<String>,
	licence: Option<String>,
	developers: Vec<String>,
	translators: Vec<String>,
	artists: Vec<String>,
	doc_writers: Vec<String>,
}

impl<'a> AboutBoxBuilder<'a> {
	#[must_use]
	pub const fn new(parent: &'a Frame) -> Self {
		Self {
			parent,
			name: None,
			version: None,
			description: None,
			copyright: None,
			website: None,
			licence: None,
			developers: Vec::new(),
			translators: Vec::new(),
			artists: Vec::new(),
			doc_writers: Vec::new(),
		}
	}

	#[must_use]
	pub fn name(mut self, name: impl Into<String>) -> Self {
		self.name = Some(name.into());
		self
	}

	#[must_use]
	pub fn version(mut self, version: impl Into<String>) -> Self {
		self.version = Some(version.into());
		self
	}

	#[must_use]
	pub fn description(mut self, desc: impl Into<String>) -> Self {
		self.description = Some(desc.into());
		self
	}

	#[must_use]
	pub fn copyright(mut self, copyright: impl Into<String>) -> Self {
		self.copyright = Some(copyright.into());
		self
	}

	#[must_use]
	pub fn website(mut self, url: impl Into<String>) -> Self {
		self.website = Some(url.into());
		self
	}

	/// Sets the licence text shown in the dialog.
	///
	/// Platforms that offer a native about box do not have a place for this, so setting it makes
	/// wxWidgets fall back to its own dialog. That is already the case once developers or a
	/// website are set, so it costs nothing alongside those.
	#[must_use]
	pub fn licence(mut self, licence: impl Into<String>) -> Self {
		self.licence = Some(licence.into());
		self
	}

	#[must_use]
	pub fn add_developer(mut self, dev: impl Into<String>) -> Self {
		self.developers.push(dev.into());
		self
	}

	/// Credits someone who translated the application, listed under their own heading in the
	/// dialog rather than mixed in with the developers.
	#[must_use]
	pub fn add_translator(mut self, translator: impl Into<String>) -> Self {
		self.translators.push(translator.into());
		self
	}

	#[must_use]
	pub fn add_artist(mut self, artist: impl Into<String>) -> Self {
		self.artists.push(artist.into());
		self
	}

	/// Credits someone who wrote the documentation.
	#[must_use]
	pub fn add_doc_writer(mut self, writer: impl Into<String>) -> Self {
		self.doc_writers.push(writer.into());
		self
	}

	pub fn show(self) {
		unsafe {
			let info = ffi::wxd_AboutDialogInfo_Create();
			if info.is_null() {
				return;
			}
			// A quick internal macro to handle the CString conversion and FFI call
			macro_rules! apply_str {
				($val:expr, $f:path) => {
					if let Some(s) = $val {
						if let Ok(cs) = CString::new(s) {
							$f(info, cs.as_ptr());
						}
					}
				};
			}
			apply_str!(self.name, ffi::wxd_AboutDialogInfo_SetName);
			apply_str!(self.version, ffi::wxd_AboutDialogInfo_SetVersion);
			apply_str!(self.description, ffi::wxd_AboutDialogInfo_SetDescription);
			apply_str!(self.copyright, ffi::wxd_AboutDialogInfo_SetCopyright);
			apply_str!(self.website, ffi::wxd_AboutDialogInfo_SetWebSite);
			apply_str!(self.licence, ffi::wxd_AboutDialogInfo_SetLicence);
			macro_rules! apply_each {
				($vals:expr, $f:path) => {
					for value in $vals {
						if let Ok(cs) = CString::new(value) {
							$f(info, cs.as_ptr());
						}
					}
				};
			}
			apply_each!(self.developers, ffi::wxd_AboutDialogInfo_AddDeveloper);
			apply_each!(self.translators, ffi::wxd_AboutDialogInfo_AddTranslator);
			apply_each!(self.artists, ffi::wxd_AboutDialogInfo_AddArtist);
			apply_each!(self.doc_writers, ffi::wxd_AboutDialogInfo_AddDocWriter);
			ffi::wxd_AboutBox(info, self.parent.handle_ptr());
			ffi::wxd_AboutDialogInfo_Destroy(info);
		}
	}
}
