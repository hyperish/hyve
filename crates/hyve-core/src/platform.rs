//! Target-specific clock and randomness.
//!
//! `wasm32-unknown-unknown` has no OS clock or RNG behind `std`; there the clock
//! comes from `Date.now()` and randomness from `crypto.getRandomValues` (via
//! `getrandom`'s `wasm_js` backend).

/// Current UTC time in microseconds since the Unix epoch.
pub(crate) fn now_unix_micros() -> i64 {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        (js_sys::Date::now() * 1000.0) as i64
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_micros() as i64,
            Err(e) => -(e.duration().as_micros() as i64),
        }
    }
}

/// Fills `buf` from the platform CSPRNG.
///
/// # Panics
///
/// Panics if the operating system's random source is unavailable. Generating
/// identifiers without a secure random source would silently weaken them, so
/// this is treated as a fatal environment defect rather than a recoverable error.
pub(crate) fn fill_random(buf: &mut [u8]) {
    if let Err(e) = getrandom::fill(buf) {
        panic!("hyve-core: the platform CSPRNG is unavailable ({e})");
    }
}
