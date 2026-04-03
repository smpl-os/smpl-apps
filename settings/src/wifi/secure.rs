//! Secure in-memory string that is **zero-wiped when dropped**.
//!
//! # Why this exists
//!
//! Rust's standard [`String`] frees its heap allocation on drop, but the
//! allocator does *not* overwrite the bytes.  Until the allocator hands that
//! page back to the OS (or another allocation overwrites it), the password
//! bytes remain readable in:
//!
//! * `/proc/<pid>/mem` (readable by root)
//! * A kernel crash dump or hibernation image
//! * A cold-boot DRAM image
//! * A debugger / memory profiler attached at the right moment
//!
//! [`SecretString`] wraps [`zeroize::Zeroizing<String>`] which uses
//! `write_volatile` plus a [`compiler_fence`][core::sync::atomic::compiler_fence]
//! to prevent the wipe from being optimised away as a dead-store.
//!
//! # Intentional limitations
//!
//! * **[`Clone`] is deliberately not implemented.** Every clone would create
//!   another cleartext copy in heap memory, widening the attack surface.
//!   If you *must* duplicate a secret, use [`SecretString::new`] explicitly so
//!   the duplication is visible in code review.
//!
//! * **Slint [`SharedString`] fields cannot be zeroed.** When a password is
//!   read from a Slint `TextInput`, callers should immediately convert it to a
//!   `SecretString` and then call `ui.set_wifi_password_input("".into())` to
//!   request that Slint release its copy.  Slint's reference-counted internals
//!   may still hold a copy briefly, but this reduces the window.
//!
//! * **The `qrcode` crate internally copies the URI bytes** when building the
//!   QR matrix.  This is outside our control; the copy is short-lived and the
//!   encoded form is pixel data, not plaintext.

use std::fmt;
use zeroize::Zeroizing;

// ── SecretString ──────────────────────────────────────────────────────────────

/// A heap-allocated UTF-8 string that is zero-wiped the moment it is dropped.
///
/// The inner [`Zeroizing<String>`] calls `String::zeroize()` on drop, which
/// overwrites every byte of the heap allocation with `0x00` using
/// `core::ptr::write_volatile` and a `compiler_fence`, thwarting dead-store
/// elimination by the optimiser.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Allocate a new `SecretString` copied from `s`.
    #[inline]
    pub fn new(s: &str) -> Self {
        Self(Zeroizing::new(s.to_string()))
    }

    /// View the contents as a plain `&str`.
    ///
    /// The returned reference is borrowed from `self`; it is valid only as
    /// long as `self` is alive.  Do **not** copy it into a plain `String`.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// `true` if the string has no characters.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of UTF-8 bytes.
    #[inline]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

// ── Constructors ──────────────────────────────────────────────────────────────

impl From<&str> for SecretString {
    /// Copy the string slice into a new zeroing allocation.
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for SecretString {
    /// Move `s` directly into the zeroing wrapper — no extra copy.
    ///
    /// The single heap allocation that backed `s` is now owned by
    /// `Zeroizing<String>`.  When the `SecretString` is dropped, `Zeroizing`
    /// calls `String::zeroize()` which overwrites every byte with `0x00`
    /// using `core::ptr::write_volatile` + a `compiler_fence`, defeating
    /// dead-store elimination.
    fn from(s: String) -> Self {
        Self(Zeroizing::new(s))
    }
}

// ── Safety traits ─────────────────────────────────────────────────────────────

// SAFETY: `Zeroizing<String>` is `Send` (String is Send).
unsafe impl Send for SecretString {}
// SAFETY: `Zeroizing<String>` is `Sync` (String is Sync).
unsafe impl Sync for SecretString {}

// ── Display / Debug (never expose value) ─────────────────────────────────────

/// Always prints `[REDACTED]`; never the actual value.
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Always prints `[REDACTED]`; never the actual value.
impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_new() {
        let s = SecretString::new("hunter2");
        assert_eq!(s.as_str(), "hunter2");
        assert!(!s.is_empty());
    }

    #[test]
    #[ignore = "reads freed memory; unreliable when allocator reuses pages"]
    fn secret_zeroed_on_drop() {
        // Collect the allocation address before moving into SecretString.
        let s = "hunter2".to_string();
        let ptr = s.as_ptr();
        let len = s.len();

        // Move into SecretString — same allocation, no copy.
        let secret = SecretString::from(s);
        assert_eq!(secret.as_str(), "hunter2");

        // Drop the SecretString; Zeroizing::drop calls write_volatile on each byte.
        drop(secret);

        // Read the (now freed) backing bytes.  The allocator usually does not
        // immediately reuse the page, so we can verify the zeroing.
        // SAFETY: We are reading memory we no longer own; valid only in tests
        //         where the allocator is unlikely to have reused the region yet.
        unsafe {
            let bytes = std::slice::from_raw_parts(ptr, len);
            assert!(
                bytes.iter().all(|&b| b == 0),
                "password bytes were not zeroed on SecretString::drop"
            );
        }
    }

    #[test]
    fn debug_display_redacts() {
        let s = SecretString::new("sup3r_s3cr3t");
        assert_eq!(format!("{:?}", s), "[REDACTED]");
        assert_eq!(format!("{}", s), "[REDACTED]");
    }

    #[test]
    fn empty_check() {
        assert!(SecretString::new("").is_empty());
        assert!(!SecretString::new("x").is_empty());
    }
}
