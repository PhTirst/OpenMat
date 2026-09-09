use std::{borrow::Cow, fmt};

use crate::sys;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    Argument,
    Type,
    Dimension,
    Range,
    Allocation,
    Cancelled,
    Unsupported,
    Plugin,
    Abi,
    State,
    Callback,
    Encoding,
    NotFound,
    Unknown(u32),
}

impl ErrorKind {
    pub const fn status(self) -> u32 {
        match self {
            Self::Argument => sys::OEX_ERROR_ARGUMENT,
            Self::Type => sys::OEX_ERROR_TYPE,
            Self::Dimension => sys::OEX_ERROR_DIMENSION,
            Self::Range => sys::OEX_ERROR_RANGE,
            Self::Allocation => sys::OEX_ERROR_ALLOCATION,
            Self::Cancelled => sys::OEX_ERROR_CANCELLED,
            Self::Unsupported => sys::OEX_ERROR_UNSUPPORTED,
            Self::Plugin => sys::OEX_ERROR_PLUGIN,
            Self::Abi => sys::OEX_ERROR_ABI,
            Self::State => sys::OEX_ERROR_STATE,
            Self::Callback => sys::OEX_ERROR_CALLBACK,
            Self::Encoding => sys::OEX_ERROR_ENCODING,
            Self::NotFound => sys::OEX_ERROR_NOT_FOUND,
            Self::Unknown(status) => {
                if status == 0 {
                    sys::OEX_ERROR_PLUGIN
                } else {
                    status
                }
            }
        }
    }

    pub const fn from_status(status: u32) -> Self {
        match status {
            1 => Self::Argument,
            2 => Self::Type,
            3 => Self::Dimension,
            4 => Self::Range,
            5 => Self::Allocation,
            6 => Self::Cancelled,
            7 => Self::Unsupported,
            8 => Self::Plugin,
            9 => Self::Abi,
            10 => Self::State,
            11 => Self::Callback,
            12 => Self::Encoding,
            13 => Self::NotFound,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    identifier: Option<String>,
    message: Cow<'static, str>,
}

impl Error {
    pub fn new(identifier: impl Into<String>, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind: ErrorKind::Plugin,
            identifier: Some(identifier.into()),
            message: message.into(),
        }
    }

    pub fn with_kind(kind: ErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind,
            identifier: None,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
    pub fn status(&self) -> sys::OexStatus {
        self.kind.status()
    }
    pub fn identifier(&self) -> Option<&str> {
        self.identifier.as_deref()
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(identifier) = &self.identifier {
            write!(f, "{identifier}: ")?;
        }
        write!(f, "{} (OEX status {})", self.message, self.status())
    }
}

impl std::error::Error for Error {}

pub(crate) fn check(status: u32, operation: &'static str) -> Result<()> {
    if status == sys::OEX_OK {
        Ok(())
    } else {
        Err(Error::with_kind(ErrorKind::from_status(status), operation))
    }
}

pub(crate) fn length(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::with_kind(ErrorKind::Range, "length exceeds usize"))
}

pub(crate) fn count(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::with_kind(ErrorKind::Range, "count exceeds u32"))
}
