//! The small text helpers both ASCII languages need.
//!
//! TESmart's text protocols are two unrelated languages that happen to share a transport: the
//! network side-channel in [`ascii`](crate::wire::ascii) and the matrix command language in
//! [`matrix`](crate::wire::matrix). This module holds only what both of them use, so neither has
//! to depend on the other for a string utility.

use core::fmt::{self, Write as _};

use crate::wire::WireBytes;

/// Renders an ASCII command into a single wire unit.
///
/// # Panics
///
/// Panics if the rendered command exceeds [`MAX_WIRE_LEN`](crate::wire::MAX_WIRE_LEN). The
/// longest command in any family is the matrix `MT8019MA:255.255.255.255;`, at 25 bytes.
pub(crate) fn render(args: fmt::Arguments<'_>) -> WireBytes {
    let mut bytes = WireBytes::new();
    bytes.write_fmt(args).expect("ASCII command longer than MAX_WIRE_LEN");
    bytes
}

/// Splits `prefix` off the front of `text`, ignoring case.
///
/// Nothing promises the case of what a switch sends, and the keywords are ASCII throughout.
pub(crate) fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, rest) = text.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_are_matched_without_regard_to_case() {
        assert_eq!(strip_prefix_ignore_case("LINK:rest", "link:"), Some("rest"));
        assert_eq!(strip_prefix_ignore_case("mt00SW", "MT00"), Some("SW"));
        assert_eq!(strip_prefix_ignore_case("IP?", "PT"), None);
        // A prefix longer than the text is not a match, and must not panic.
        assert_eq!(strip_prefix_ignore_case("IP", "IP?"), None);
    }

    #[test]
    fn rendering_produces_the_bytes_it_was_given() {
        let bytes = render(format_args!("IP:{}.{}.{}.{};", 10, 0, 2, 12));
        assert_eq!(bytes.as_str(), Some("IP:10.0.2.12;"));
    }
}
