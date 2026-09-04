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

use crate::contacts::fixtures::ContactDetailModel;
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

/// One group's members — DC4.
///
/// `None` when the index names no group: the rail can change under an open
/// view, and drawing whichever group slid into that slot would put somebody
/// else's members under this group's name — with a 群发转账 button above them.
#[must_use]
pub fn group_members(
    view: &ContactsView,
    index: usize,
) -> Option<(SharedString, Vec<ContactRowModel>)> {
    let group = view.groups.get(index)?;
    Some((
        SharedString::from(group.name.clone()),
        group.members.iter().map(row).collect(),
    ))
}

/// One contact, in detail — DC2.
///
/// **The panel used to draw a FIXTURE while its delete and copy acted on the
/// real contact.** So somebody clicking their cousin saw Alice's name, Alice's
/// avatar and Alice's address, and the delete button removed the cousin. A
/// mismatch is worse than a mock: a mock is honestly a picture, and this was a
/// picture with a live weapon attached.
///
/// `None` when the index names nobody — the roster can change under an open
/// panel, and drawing the row that took its place would silently swap who the
/// delete button is pointed at.
#[must_use]
pub fn detail(
    view: &ContactsView,
    index: usize,
    feed: &vela_core::app::activity_feed::FeedView,
    wallet: &crate::wallet::WalletStrings,
    hidden: bool,
) -> Option<ContactDetailModel> {
    let contact = rows(view).into_iter().nth(index)?;
    let address = contact.address_full.to_string();
    let lower = address.to_lowercase();
    Some(ContactDetailModel {
        name: contact.name.clone(),
        // The ADDRESS, not the name: two contacts a person named the same must
        // not draw the same avatar, and the avatar is how somebody checks they
        // are looking at the right one.
        seed: contact.address_full.clone(),
        chips: view
            .groups
            .iter()
            .filter(|group| {
                group
                    .members
                    .iter()
                    .any(|member| member.address.to_lowercase() == lower)
            })
            .map(|group| SharedString::from(group.name.clone()))
            .collect(),
        address_full: contact.address_full,
        // What this person and I have actually exchanged, from the same feed
        // the home draws. Matched on the counterparty, which is the only thing
        // that makes a row "theirs".
        activity: feed
            .rows
            .iter()
            .filter_map(|row| match row {
                vela_core::app::activity_feed::FeedRow::Item { item }
                    if item
                        .counterparty
                        .as_ref()
                        .is_some_and(|other| other.to_lowercase() == lower) =>
                {
                    Some(crate::wallet::live::activity_row(
                        feed, item, wallet, hidden,
                    ))
                }
                _ => None,
            })
            .collect(),
    })
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
    view.contacts.iter().map(row).collect()
}

/// One contact as a row. The name precedence is `display_name`'s, and the
/// section is derived from the name that will actually be drawn — deriving it
/// from a different string is how a row files under a letter it does not show.
fn row(contact: &Contact) -> ContactRowModel {
    let name = display_name(contact);
    ContactRowModel {
        section: section_of(&name),
        name,
        address_display: shorten(&contact.address),
        address_full: SharedString::from(contact.address.clone()),
    }
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

    /// The detail is about the contact that was opened, and its avatar is
    /// seeded by the address rather than the name.
    #[test]
    fn the_detail_is_about_the_contact_that_was_opened() {
        use vela_core::app::activity_feed::{
            ActivityFeed, Event as FeedEvent, FeedDirection, FeedItem, FeedRow, FeedView,
        };
        use vela_core::app::contacts::ContactGroupView;

        let mut book = view(vec![
            contact(
                "0xAAA0000000000000000000000000000000000001",
                Some("Alice"),
                None,
            ),
            contact(
                "0xBBB0000000000000000000000000000000000002",
                Some("Cousin"),
                None,
            ),
        ]);
        book.groups = vec![ContactGroupView {
            id: "g1".to_owned(),
            name: "Family".to_owned(),
            color: None,
            members: vec![contact(
                "0xBBB0000000000000000000000000000000000002",
                Some("Cousin"),
                None,
            )],
        }];

        let mut host = crate::core_host::CoreHost::<ActivityFeed>::new();
        let _ = host.dispatch(FeedEvent::AccountSwitched {
            address: "0xme".to_owned(),
        });
        let feed = FeedView {
            rows: vec![FeedRow::Item {
                item: FeedItem {
                    id: "t1".to_owned(),
                    direction: FeedDirection::Out,
                    // Cousin's, in a different case — the match must not care.
                    counterparty: Some("0xbbb0000000000000000000000000000000000002".to_owned()),
                    alias: None,
                    value: Some("2".to_owned()),
                    symbol: "xDAI".to_owned(),
                    decimals: Some(18),
                    usd_value: 2.0,
                    chain_id: 100,
                    timestamp: 1_788_500_000.0,
                    day_start_ms: 0.0,
                    tx_hash: None,
                    batch: None,
                },
            }],
            ..host.view()
        };
        let wallet = crate::wallet::WalletStrings::resolve(&crate::loc::Loc::from_env());

        // Row 1 is the cousin, and the panel must be about the cousin.
        let cousin =
            detail(&book, 1, &feed, &wallet, false).unwrap_or_else(|| unreachable!("row 1 exists"));
        assert_eq!(cousin.name, "Cousin");
        assert_eq!(
            cousin.address_full,
            "0xBBB0000000000000000000000000000000000002"
        );
        // Seeded by the ADDRESS: two contacts named the same must not share an
        // avatar, and the avatar is how somebody checks they have the right one.
        assert_eq!(cousin.seed, cousin.address_full);
        assert_eq!(cousin.chips, vec![SharedString::from("Family")]);
        // Their own history, matched case-insensitively.
        assert_eq!(cousin.activity.len(), 1);

        // Alice is in no group and has nothing with me.
        let alice =
            detail(&book, 0, &feed, &wallet, false).unwrap_or_else(|| unreachable!("row 0 exists"));
        assert_eq!(alice.name, "Alice");
        assert!(alice.chips.is_empty());
        assert!(alice.activity.is_empty());

        // The roster moved: no panel rather than the wrong one.
        assert!(detail(&book, 9, &feed, &wallet, false).is_none());

        // And a group's members are that group's.
        let (name, members) =
            group_members(&book, 0).unwrap_or_else(|| unreachable!("group 0 exists"));
        assert_eq!(name, "Family");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].name, "Cousin");
        assert!(group_members(&book, 5).is_none());
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
