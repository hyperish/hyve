//! Complete-input ASCII specialization of winnow 1.0.5's take_till0/1
//! (src/token/mod.rs at 21b3ab2535a675768a170a851efda83253b9eb21).
//! Upstream copyright and MIT terms are in LICENSE-MIT alongside this module.
//! Our config grammars operate on complete UTF-8 input and accept only ASCII
//! tokens. Stopping at the first non-ASCII byte preserves UTF-8 boundaries.

/// Returns a borrowed run while advancing only on success. The upstream
/// partial-stream and generic token/error branches are unnecessary here.
pub(super) fn take_ascii1<'a>(
    input: &mut &'a str,
    accepts: impl Fn(u8) -> bool,
) -> Option<&'a str> {
    let offset = input
        .bytes()
        .position(|byte| !byte.is_ascii() || !accepts(byte))
        .unwrap_or(input.len());
    if offset == 0 {
        return None;
    }
    let (taken, rest) = input.split_at(offset);
    *input = rest;
    Some(taken)
}
