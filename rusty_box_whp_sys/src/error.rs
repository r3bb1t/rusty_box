//! The one error type every entry point across the WHP seam returns.

use core::fmt;

/// What went wrong, coarsely enough that a caller can branch on it without
/// knowing a single `HRESULT`.
///
/// Non-exhaustive at birth: this enum crosses a crate boundary, so gaining a
/// variant must stay a minor change (REPLAN v4 decision 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WhpErrorKind {
    /// The platform is not there at all: a non-Windows build, or a Windows one
    /// where `winhvplatform.dll` reports no hypervisor. Distinguished from
    /// every other kind because it is the one a caller is expected to handle by
    /// choosing a different engine rather than by failing.
    Unsupported,
    /// The hypervisor rejected the call. Carries the raw `HRESULT` so the exact
    /// refusal survives to a log.
    Platform,
    /// A handle the platform returned failed its own validity test, or a
    /// buffer it filled had a size the API contract forbids.
    Contract,
    /// The host allocation backing a guest-physical range could not be made.
    HostMemory,
}

/// A failed Windows Hypervisor Platform call.
///
/// Keeps a private field so adding context later is not a breaking change.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WhpError {
    kind: WhpErrorKind,
    /// The `HRESULT` for [`WhpErrorKind::Platform`], zero otherwise.
    hresult: i32,
    /// The API this came out of, for a message a reader can act on.
    call: &'static str,
}

impl WhpError {
    /// The refusal reported by a platform call.
    pub(crate) const fn platform(call: &'static str, hresult: i32) -> Self {
        Self { kind: WhpErrorKind::Platform, hresult, call }
    }

    /// This build, or this machine, has no Windows Hypervisor Platform.
    pub(crate) const fn unsupported(call: &'static str) -> Self {
        Self { kind: WhpErrorKind::Unsupported, hresult: 0, call }
    }

    /// The platform answered, but with something its own contract forbids.
    pub const fn contract(call: &'static str) -> Self {
        Self { kind: WhpErrorKind::Contract, hresult: 0, call }
    }

    /// Host memory for a guest-physical range could not be obtained.
    pub const fn host_memory(call: &'static str) -> Self {
        Self { kind: WhpErrorKind::HostMemory, hresult: 0, call }
    }

    /// Which of the four kinds this is.
    #[must_use]
    pub const fn kind(&self) -> WhpErrorKind {
        self.kind
    }

    /// The `HRESULT` behind a [`WhpErrorKind::Platform`] error; zero for every
    /// other kind.
    #[must_use]
    pub const fn hresult(&self) -> i32 {
        self.hresult
    }

    /// The platform API that refused.
    #[must_use]
    pub const fn call(&self) -> &'static str {
        self.call
    }
}

impl fmt::Display for WhpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            WhpErrorKind::Unsupported => {
                write!(f, "{}: no Windows Hypervisor Platform on this build or host", self.call)
            }
            WhpErrorKind::Platform => {
                write!(f, "{} failed: HRESULT {:#010x}", self.call, self.hresult as u32)
            }
            WhpErrorKind::Contract => {
                write!(f, "{} answered outside its own contract", self.call)
            }
            WhpErrorKind::HostMemory => {
                write!(f, "{}: host memory for the guest-physical range could not be allocated", self.call)
            }
        }
    }
}

impl fmt::Debug for WhpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for WhpError {}

/// Every fallible verb across the WHP seam returns this.
pub type WhpResult<T> = Result<T, WhpError>;
