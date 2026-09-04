//! The contacts screen's display models, built from what the core decided.
//!
//! The sibling of `fixtures.rs`, never its replacement.
//!
//! ## What the core owns, and what this adds
//!
//! The core owns the book itself: saved entries merged with history-derived
//! suggestions, tombstone-suppressed, sorted favourites-first then most-recent.
//! **That order is not re-sorted here.** This adds only the two things the core
//! declines to decide because they are render rules — which of the three names a
//! row shows, and which letter it files under — and then groups the rows the
//! core already ordered.

use gpui::SharedString;

use vela_core::app::contacts::{Contact, ContactsView};

use crate::contacts::model::{ContactRowModel, section_of, shorten};

/// The name a row shows: the person's own label, else a resolved identity, else
/// the shortened address.
///
/// The core carries all three and refuses to pick — `name` "wins over
/// `resolved_name` for display" is the only precedence it states, and it states
/// it as a comment about the shell's job rather than a field it computes.
fn display_name(contact: &Contact) -> SharedString {
    contact
        .name
        .as_deref()
        .filter(|value| !value.is_empty())
        .or(contact
            .resolved_name
            .as_deref()
            .filter(|value| !value.is_empty()))
        .map_or_else(
            || shorten(&contact.address),
            |value| SharedString::from(value.to_owned()),
        )
}

/// The group rail: the person's own groups, with how many people are in each.
///
/// The rail drew `fixtures::GROUPS` for a signed-in person too — 家人 / 工作 /
/// … under somebody's real address book. The same shape the home's asset strip
/// had before phase 17, and found the same way: by reading what the surface
/// actually calls rather than trusting that "contacts is live".
///
/// Returns the id alongside, because the row that opens a group's menu has to
/// name WHICH group to the core, and an index into a list that reorders is not
/// a name.
#[must_use]
pub fn groups(view: &ContactsView) -> Vec<(SharedString, SharedString, u32)> {
    view.groups
        .iter()
        .map(|group| {
            (
                SharedString::from(group.id.clone()),
                SharedString::from(group.name.clone()),
                u32::try_from(group.members.len()).unwrap_or(u32::MAX),
            )
        })
        .collect()
}

/// One row per contact, in the core's order.
#[must_use]
pub fn rows(view: &ContactsView) -> Vec<ContactRowModel> {
    view.contacts
        .iter()
        .map(|contact| {
            let name = display_name(contact);
            ContactRowModel {
                section: section_of(&name),
                name,
                address_display: shorten(&contact.address),
                address_full: SharedString::from(contact.address.clone()),
            }
        })
        .collect()
}

/// The roster, grouped into the A–Z sections the screen draws.
///
/// Grouping only — **not** sorting. Re-sorting here would silently override the
/// core's favourites-first, most-recent-next ordering, which is a product rule
/// it owns and tests. Consecutive rows sharing a letter become one section, so
/// the core's order survives intact.
#[must_use]
pub fn sections(view: &ContactsView) -> Vec<(SharedString, Vec<ContactRowModel>)> {
    let mut out: Vec<(SharedString, Vec<ContactRowModel>)> = Vec::new();
    for row in rows(view) {
        match out.last_mut() {
            Some((letter, rows)) if *letter == row.section => rows.push(row),
            _ => out.push((row.section.clone(), vec![row])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vela_core::app::contacts::{ContactKind, ContactSource};

    fn contact(address: &str, name: Option<&str>, resolved: Option<&str>) -> Contact {
        Contact {
            address: address.to_owned(),
            name: name.map(str::to_owned),
            resolved_name: resolved.map(str::to_owned),
            resolved_source: None,
            kind: ContactKind::Eoa,
            favorite: false,
            note: None,
            tx_count: 0,
            last_used_ms: 0.0,
            first_seen_ms: 0.0,
            source: ContactSource::Manual,
        }
    }

    fn view(contacts: Vec<Contact>) -> ContactsView {
        ContactsView {
            loaded: true,
            contacts,
            groups: Vec::new(),
            last_import: None,
            recipient: None,
        }
    }

    #[test]
    fn a_saved_name_wins_then_a_resolved_one_then_the_address() {
        let rows = rows(&view(vec![
            contact(
                "0xaaaa000000000000000000000000000000000001",
                Some("Ada"),
                Some("ada.eth"),
            ),
            contact(
                "0xbbbb000000000000000000000000000000000002",
                None,
                Some("bob.eth"),
            ),
            contact("0xcccc000000000000000000000000000000000003", None, None),
        ]));
        assert_eq!(rows[0].name, SharedString::from("Ada"));
        assert_eq!(rows[1].name, SharedString::from("bob.eth"));
        assert_eq!(rows[2].name, SharedString::from("0xcccc…0003"));
    }

    /// The rail lists the person's own groups, with the id each row needs.
    #[test]
    fn the_group_rail_is_the_persons_own_and_carries_each_id() {
        use vela_core::app::contacts::ContactGroupView;

        let mut book = view(Vec::new());
        book.groups = vec![
            ContactGroupView {
                id: "g-family".to_owned(),
                name: "Family".to_owned(),
                color: None,
                members: vec![contact("0xaaa", None, None), contact("0xbbb", None, None)],
            },
            ContactGroupView {
                id: "g-empty".to_owned(),
                name: "Work".to_owned(),
                color: None,
                members: Vec::new(),
            },
        ];

        let rows = groups(&book);
        assert_eq!(rows.len(), 2);
        // The ID, not the index: a rail that reorders would otherwise delete
        // whichever group slid into the slot that was clicked.
        assert_eq!(rows[0].0, "g-family");
        assert_eq!(rows[0].1, "Family");
        assert_eq!(rows[0].2, 2);
        // An empty group is still a group — it is a thing the person made, and
        // hiding it would make its delete unreachable.
        assert_eq!(rows[1].0, "g-empty");
        assert_eq!(rows[1].2, 0);

        assert!(groups(&view(Vec::new())).is_empty());
    }

    /// The core sorts favourites first, then most recent. Grouping must not
    /// quietly re-sort that away.
    #[test]
    fn sectioning_groups_without_reordering() {
        let rows = view(vec![
            contact("0x1", Some("Zoe"), None),
            contact("0x2", Some("Zack"), None),
            contact("0x3", Some("Ada"), None),
        ]);
        let sections = sections(&rows);
        assert_eq!(
            sections.len(),
            2,
            "two runs of letters, not two sorted buckets"
        );
        assert_eq!(sections[0].0, SharedString::from("Z"));
        assert_eq!(
            sections[0]
                .1
                .iter()
                .map(|r| r.name.to_string())
                .collect::<Vec<_>>(),
            vec!["Zoe", "Zack"],
            "the core's order inside a letter must survive"
        );
        assert_eq!(sections[1].0, SharedString::from("A"));
    }

    /// A name that is not a letter files under `#`, as the mocks draw.
    #[test]
    fn a_non_alphabetic_name_files_under_hash() {
        let rows = rows(&view(vec![contact(
            "0xdddd000000000000000000000000000000000004",
            Some("42"),
            None,
        )]));
        assert_eq!(rows[0].section, SharedString::from("#"));
    }
}
