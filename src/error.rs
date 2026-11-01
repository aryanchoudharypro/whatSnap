use std::fmt;

pub(crate) struct Error(String);

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl Error {
	pub(crate) fn msg(message: impl fmt::Display) -> Self {
		Error(message.to_string())
	}
}

impl<E: std::error::Error> From<E> for Error {
	fn from(e: E) -> Self {
		Error(e.to_string())
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl fmt::Debug for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

macro_rules! err {
	($($arg:tt)*) => {
		$crate::error::Error::msg(format!($($arg)*))
	};
}

macro_rules! bail {
	($($arg:tt)*) => {
		return Err($crate::error::err!($($arg)*))
	};
}

pub(crate) use bail;
pub(crate) use err;
