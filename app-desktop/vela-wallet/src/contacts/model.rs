//! The display models the contacts screen renders.
//!
//! The twin of `settings/model.rs`, and here for the same reason: `page.rs` read
//! `contacts_fixtures::sections()` and rendered `&'static str` fields directly,
//! so a live builder had nothing to plug into.

use gpui::SharedString;

/// One row of the A–Z roster.
#[derive(Clone, Debug, PartialEq)]
pub struct ContactRowModel {
    /// What the row shows as the name: the person's own label if they gave one,
    /// else a resolved identity, else the shortened address. The core carries
    /// all three and refuses to choose — display precedence is a render rule.
    pub name: SharedString,
    pub address_display: SharedString,
    /// The full address. The identicon is derived from it, so it must be the
    /// canonical lowercase form or two spellings of one person draw two
    /// different faces.
    pub address_full: SharedString,
    pub section: SharedString,
}

/// `0x1234…abcd`, the shortening the mocks draw.
///
/// Kept here rather than in `live.rs` because the fixture adapter needs it too:
/// a live row and a mock row must shorten identically, or the seam shows.
#[must_use]
pub fn shorten(address: &str) -> SharedString {
    if address.len() <= 14 {
        return SharedString::from(address.to_owned());
    }
    SharedString::from(format!(
        "{}…{}",
        &address[..6],
        &address[address.len() - 4..]
    ))
}

/// The A–Z section a name sorts under. Anything that is not an ASCII letter
/// files under `#`, which is what the mocks draw for a numeric or symbol name.
#[must_use]
pub fn section_of(name: &str) -> SharedString {
    name.chars()
        .next()
        .filter(char::is_ascii_alphabetic)
        .map(|c| SharedString::from(c.to_uppercase().to_string()))
        .unwrap_or_else(|| SharedString::from("#"))
}
