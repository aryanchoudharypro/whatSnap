//! `--sign`, run as administrator: signs the Share menu package with a
//! certificate made here, and makes Windows trust that certificate.
//!
//! The private key is an ephemeral one that is never written to disk: it
//! exists only while this process signs the package, so once it exits nothing
//! can sign another package with this certificate. The certificate (public
//! part only) goes into the machine's Trusted People store, which is where
//! Windows looks for package signers and the step that needs administrator
//! rights. Certificates from earlier runs are replaced.
//!
//! Signing follows Microsoft's "How to programmatically sign an app package"
//! (SignerSignEx2 with an APPX_SIP_CLIENT_DATA).

use std::ffi::c_void;

use windows::{
	Win32::{
		Foundation::{FILETIME, HANDLE, SYSTEMTIME},
		Security::Cryptography::*,
		System::{SystemInformation::GetSystemTime, Time::FileTimeToSystemTime},
	},
	core::{BOOL, HSTRING, IUnknown, Interface, PCSTR, PCWSTR, PSTR},
};

use crate::package::{PUBLISHER, local_dir, signed_package, signed_source, unsigned_package};

/// SIGNER_SIGN_EX2_PARAMS, which the windows crate doesn't define.
#[repr(C)]
struct SignerSignEx2Params {
	flags: u32,
	subject_info: *const SIGNER_SUBJECT_INFO,
	signing_cert: *const SIGNER_CERT,
	signature_info: *const SIGNER_SIGNATURE_INFO,
	provider_info: *const SIGNER_PROVIDER_INFO,
	timestamp_flags: u32,
	timestamp_algorithm_oid: PCSTR,
	timestamp_url: PCWSTR,
	crypt_attrs: *const CRYPT_ATTRIBUTES,
	sip_data: *const c_void,
	signer_context: *mut *mut SIGNER_CONTEXT,
	crypto_policy: *const c_void,
	reserved: *const c_void,
}

/// APPX_SIP_CLIENT_DATA: what the package SIP needs to sign an .msix.
#[repr(C)]
struct AppxSipClientData {
	signer_params: *const SignerSignEx2Params,
	appx_sip_state: *mut c_void,
}

const ENCODING: CERT_QUERY_ENCODING_TYPE = CERT_QUERY_ENCODING_TYPE(X509_ASN_ENCODING.0 | PKCS_7_ASN_ENCODING.0);

/// Encodes a CryptoAPI structure, asking for its size first.
unsafe fn encode(struct_type: PCSTR, info: *const c_void) -> windows::core::Result<Vec<u8>> {
	let mut size = 0u32;
	unsafe {
		CryptEncodeObjectEx(X509_ASN_ENCODING, struct_type, info, CRYPT_ENCODE_OBJECT_FLAGS(0), None, None, &mut size)?;
		let mut out = vec![0u8; size as usize];
		CryptEncodeObjectEx(
			X509_ASN_ENCODING,
			struct_type,
			info,
			CRYPT_ENCODE_OBJECT_FLAGS(0),
			None,
			Some(out.as_mut_ptr().cast()),
			&mut size,
		)?;
		out.truncate(size as usize);
		Ok(out)
	}
}

/// An ephemeral 2048-bit RSA key in the software key storage provider.
unsafe fn create_key() -> windows::core::Result<NCRYPT_KEY_HANDLE> {
	unsafe {
		let mut provider = NCRYPT_PROV_HANDLE::default();
		NCryptOpenStorageProvider(&mut provider, MS_KEY_STORAGE_PROVIDER, 0)?;
		let mut key = NCRYPT_KEY_HANDLE::default();
		let created = NCryptCreatePersistedKey(
			provider,
			&mut key,
			windows::core::w!("RSA"),
			PCWSTR::null(),
			CERT_KEY_SPEC(0),
			NCRYPT_FLAGS(0),
		);
		let _ = NCryptFreeObject(NCRYPT_HANDLE(provider.0));
		created?;
		let length = 2048u32.to_le_bytes();
		let finished = NCryptSetProperty(NCRYPT_HANDLE(key.0), NCRYPT_LENGTH_PROPERTY, &length, NCRYPT_FLAGS(0))
			.and_then(|_| NCryptFinalizeKey(key, NCRYPT_FLAGS(0)));
		if let Err(e) = finished {
			let _ = NCryptFreeObject(NCRYPT_HANDLE(key.0));
			return Err(e);
		}
		Ok(key)
	}
}

/// Today plus `years`, in the form the certificate wants.
fn years_from_now(years: u16) -> SYSTEMTIME {
	let mut time = unsafe { GetSystemTime() };
	time.wYear += years;
	// 29 February doesn't exist in most years.
	if time.wMonth == 2 && time.wDay == 29 {
		time.wDay = 28;
	}
	time
}

/// A yesterday's date, so a clock that's a little behind doesn't make the
/// certificate not yet valid.
fn yesterday() -> SYSTEMTIME {
	let now = unsafe { GetSystemTime() };
	let mut file_time = FILETIME::default();
	unsafe {
		let _ = windows::Win32::System::Time::SystemTimeToFileTime(&now, &mut file_time);
	}
	let ticks = ((file_time.dwHighDateTime as u64) << 32 | file_time.dwLowDateTime as u64) - 864_000_000_000;
	let earlier = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
	let mut out = now;
	unsafe {
		let _ = FileTimeToSystemTime(&earlier, &mut out);
	}
	out
}

/// Makes the self-signed code-signing certificate, holding `key` (which it
/// then owns and frees).
unsafe fn create_certificate(key: NCRYPT_KEY_HANDLE) -> windows::core::Result<*mut CERT_CONTEXT> {
	unsafe {
		let subject = HSTRING::from(PUBLISHER);
		let mut size = 0u32;
		CertStrToNameW(X509_ASN_ENCODING, &subject, CERT_X500_NAME_STR, None, None, &mut size, None)?;
		let mut name = vec![0u8; size as usize];
		CertStrToNameW(
			X509_ASN_ENCODING,
			&subject,
			CERT_X500_NAME_STR,
			None,
			Some(name.as_mut_ptr()),
			&mut size,
			None,
		)?;
		let name_blob = CRYPT_INTEGER_BLOB { cbData: size, pbData: name.as_mut_ptr() };

		// Package signing wants the code-signing usage, and an end-entity
		// (not CA) certificate.
		let mut usage_oid = [PSTR(szOID_PKIX_KP_CODE_SIGNING.0 as *mut u8)];
		let usage = CTL_USAGE { cUsageIdentifier: 1, rgpszUsageIdentifier: usage_oid.as_mut_ptr() };
		let mut usage_bytes = encode(X509_ENHANCED_KEY_USAGE, (&usage as *const CTL_USAGE).cast())?;
		let constraints =
			CERT_BASIC_CONSTRAINTS2_INFO { fCA: BOOL(0), fPathLenConstraint: BOOL(0), dwPathLenConstraint: 0 };
		let mut constraints_bytes =
			encode(X509_BASIC_CONSTRAINTS2, (&constraints as *const CERT_BASIC_CONSTRAINTS2_INFO).cast())?;
		let mut extensions = [
			CERT_EXTENSION {
				pszObjId: PSTR(szOID_ENHANCED_KEY_USAGE.0 as *mut u8),
				fCritical: BOOL(0),
				Value: CRYPT_INTEGER_BLOB { cbData: usage_bytes.len() as u32, pbData: usage_bytes.as_mut_ptr() },
			},
			CERT_EXTENSION {
				pszObjId: PSTR(szOID_BASIC_CONSTRAINTS2.0 as *mut u8),
				fCritical: BOOL(1),
				Value: CRYPT_INTEGER_BLOB {
					cbData: constraints_bytes.len() as u32,
					pbData: constraints_bytes.as_mut_ptr(),
				},
			},
		];
		let extension_list =
			CERT_EXTENSIONS { cExtension: extensions.len() as u32, rgExtension: extensions.as_mut_ptr() };
		let algorithm = CRYPT_ALGORITHM_IDENTIFIER {
			pszObjId: PSTR(szOID_RSA_SHA256RSA.0 as *mut u8),
			Parameters: CRYPT_INTEGER_BLOB::default(),
		};
		let start = yesterday();
		// Long-lived: the package is registered once and stays, and there's
		// no key left to renew it with.
		let end = years_from_now(30);

		let cert = CertCreateSelfSignCertificate(
			Some(HCRYPTPROV_OR_NCRYPT_KEY_HANDLE(key.0)),
			&name_blob,
			CERT_CREATE_SELFSIGN_NO_KEY_INFO,
			None,
			Some(&algorithm),
			Some(&start),
			Some(&end),
			Some(&extension_list),
		);
		if cert.is_null() {
			let error = windows::core::Error::from_thread();
			let _ = NCryptFreeObject(NCRYPT_HANDLE(key.0));
			return Err(error);
		}
		// Lets the signer find the key through the certificate; the
		// certificate frees the key when it's freed.
		let handle = key.0;
		if let Err(e) = CertSetCertificateContextProperty(
			cert,
			CERT_NCRYPT_KEY_HANDLE_TRANSFER_PROP_ID,
			0,
			Some((&handle as *const usize).cast()),
		) {
			let _ = NCryptFreeObject(NCRYPT_HANDLE(key.0));
			let _ = CertFreeCertificateContext(Some(cert));
			return Err(e);
		}
		Ok(cert)
	}
}

/// Signs the package at `path` in place.
unsafe fn sign_file(path: &std::path::Path, cert: *const CERT_CONTEXT) -> windows::core::Result<()> {
	unsafe {
		let path = HSTRING::from(path.as_os_str());
		let mut file_info = SIGNER_FILE_INFO {
			cbSize: std::mem::size_of::<SIGNER_FILE_INFO>() as u32,
			pwszFileName: PCWSTR(path.as_ptr()),
			hFile: HANDLE::default(),
		};
		let mut index = 0u32;
		let subject_info = SIGNER_SUBJECT_INFO {
			cbSize: std::mem::size_of::<SIGNER_SUBJECT_INFO>() as u32,
			pdwIndex: &mut index,
			dwSubjectChoice: SIGNER_SUBJECT_FILE,
			Anonymous: SIGNER_SUBJECT_INFO_0 { pSignerFileInfo: &mut file_info },
		};
		let mut store_info = SIGNER_CERT_STORE_INFO {
			cbSize: std::mem::size_of::<SIGNER_CERT_STORE_INFO>() as u32,
			pSigningCert: cert,
			dwCertPolicy: SIGNER_CERT_POLICY_CHAIN_NO_ROOT,
			hCertStore: HCERTSTORE::default(),
		};
		let signer_cert = SIGNER_CERT {
			cbSize: std::mem::size_of::<SIGNER_CERT>() as u32,
			dwCertChoice: SIGNER_CERT_STORE,
			Anonymous: SIGNER_CERT_0 { pCertStoreInfo: &mut store_info },
			..Default::default()
		};
		// Must match the hash the package's block map uses: makeappx's
		// default, SHA-256.
		let signature_info = SIGNER_SIGNATURE_INFO {
			cbSize: std::mem::size_of::<SIGNER_SIGNATURE_INFO>() as u32,
			algidHash: CALG_SHA_256,
			dwAttrChoice: SIGNER_NO_ATTR,
			..Default::default()
		};
		let mut context: *mut SIGNER_CONTEXT = std::ptr::null_mut();
		let mut params = SignerSignEx2Params {
			flags: 0,
			subject_info: &subject_info,
			signing_cert: &signer_cert,
			signature_info: &signature_info,
			provider_info: std::ptr::null(),
			timestamp_flags: 0,
			timestamp_algorithm_oid: PCSTR::null(),
			timestamp_url: PCWSTR::null(),
			crypt_attrs: std::ptr::null(),
			sip_data: std::ptr::null(),
			signer_context: &mut context,
			crypto_policy: std::ptr::null(),
			reserved: std::ptr::null(),
		};
		let mut sip_data = AppxSipClientData { signer_params: &params, appx_sip_state: std::ptr::null_mut() };
		params.sip_data = (&mut sip_data as *mut AppxSipClientData).cast();

		let result = SignerSignEx2(
			SIGNER_SIGN_FLAGS(params.flags),
			params.subject_info,
			params.signing_cert,
			params.signature_info,
			None,
			None,
			PCSTR::null(),
			PCWSTR::null(),
			None,
			Some(params.sip_data),
			params.signer_context,
			None,
			None,
		);
		// The SIP hands back a COM object it's done with.
		if !sip_data.appx_sip_state.is_null() {
			drop(IUnknown::from_raw(sip_data.appx_sip_state));
		}
		if !context.is_null() {
			let _ = SignerFreeSignerContext(context);
		}
		result
	}
}

/// Puts the certificate in the machine's Trusted People store, replacing any
/// earlier whatSnap ones.
unsafe fn trust(cert: *const CERT_CONTEXT) -> windows::core::Result<()> {
	unsafe {
		let store_name = HSTRING::from("TrustedPeople");
		let store = CertOpenStore(
			CERT_STORE_PROV_SYSTEM_W,
			CERT_QUERY_ENCODING_TYPE(0),
			None,
			CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE),
			Some(store_name.as_ptr().cast()),
		)?;
		let subject = HSTRING::from(PUBLISHER.trim_start_matches("CN="));
		loop {
			let old = CertFindCertificateInStore(
				store,
				ENCODING,
				0,
				CERT_FIND_SUBJECT_STR,
				Some(subject.as_ptr().cast()),
				None,
			);
			if old.is_null() {
				break;
			}
			// Deleting frees the context it's given.
			if CertDeleteCertificateFromStore(old).is_err() {
				break;
			}
		}
		let encoded = std::slice::from_raw_parts((*cert).pbCertEncoded, (*cert).cbCertEncoded as usize);
		let added = CertAddEncodedCertificateToStore(
			Some(store),
			X509_ASN_ENCODING,
			encoded,
			CERT_STORE_ADD_REPLACE_EXISTING,
			None,
		);
		let _ = CertCloseStore(Some(store), 0);
		added
	}
}

/// Copies the installed package to `share\local`, signs the copy, and trusts
/// the certificate that signed it.
pub fn sign_package() -> Result<(), String> {
	let unsigned = std::fs::read(unsigned_package()).map_err(|e| format!("reading the package: {e}"))?;
	std::fs::create_dir_all(local_dir()).map_err(|e| format!("making {}: {e}", local_dir().display()))?;
	let pending = local_dir().join("signing.msix");
	std::fs::write(&pending, &unsigned).map_err(|e| format!("copying the package: {e}"))?;
	unsafe {
		let key = create_key().map_err(|e| format!("making a key: {e}"))?;
		let cert = create_certificate(key).map_err(|e| format!("making the certificate: {e}"))?;
		let result = sign_file(&pending, cert)
			.map_err(|e| format!("signing: {e}"))
			.and_then(|_| trust(cert).map_err(|e| format!("trusting the certificate: {e}")));
		// Frees the key too: nothing can sign with it after this.
		let _ = CertFreeCertificateContext(Some(cert));
		result?;
	}
	std::fs::rename(&pending, signed_package()).map_err(|e| format!("saving the signed package: {e}"))?;
	std::fs::write(signed_source(), &unsigned).map_err(|e| format!("saving the package copy: {e}"))?;
	Ok(())
}
