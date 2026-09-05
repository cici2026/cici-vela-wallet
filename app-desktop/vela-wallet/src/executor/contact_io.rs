//! Reading and writing an address-book backup.
//!
//! **Ported from** `src/services/contact-io.ts` @ `c513c4c6` (FR-006), the
//! serialize and parse halves only. The import POLICY — existing-wins, the
//! counts, which groups get created — is the core's (`Event::ImportParsed`),
//! and none of it is re-decided here.
//!
//! ## The format is the point
//!
//! A backup written on the phone has to open on the desktop and vice versa, so
//! `version`, `exportedAt`, `contacts` and `groups` keep their spelling, and the
//! CSV keeps its column order. Inventing a desktop format would make export a
//! feature that only talks to itself.
//!
//! ## The CSV heuristics are not tidiness
//!
//! A foreign file rarely spells the column `address` — `wallet`, `Public
//! Address` and `Recipient` are all common — and the version this ports from
//! records what happened when an unrecognised header fell back to column 0: if
//! column 0 held the NAME, every row failed the address test, every row was
//! dropped silently, and the import reported "0 added, 0 already existed".
//! Nothing imported, nothing explained, nothing to try differently. So when the
//! header does not say where the address is, **the data does**.

use serde_json::{Map, Value, json};

use vela_core::app::contacts::{Contact, ContactGroupView, ContactImportEntry, ContactImportGroup};

/// The backup document's version. Not ours to bump alone: every client reads
/// these bytes.
const BACKUP_VERSION: u64 = 1;

/// What a file yielded, before the core rules on any of it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub contacts: Vec<ContactImportEntry>,
    pub groups: Vec<ContactImportGroup>,
}

/// A CSV that plainly held contact rows and yielded no address at all.
///
/// Distinct from an empty parse on purpose: a file we cannot read must SAY so
/// rather than succeed with zero of everything, which is indistinguishable from
/// an empty address book.
#[derive(Debug, PartialEq, Eq)]
pub struct Unreadable;

// ---------------------------------------------------------------------------
// Serialize
// ---------------------------------------------------------------------------

/// The JSON backup, pretty-printed as the other clients write it.
#[must_use]
pub fn to_json(contacts: &[Contact], groups: &[ContactGroupView], exported_at: &str) -> String {
    let backup = json!({
        "version": BACKUP_VERSION,
        "exportedAt": exported_at,
        "contacts": contacts.iter().map(exported_contact).collect::<Vec<_>>(),
        "groups": groups
            .iter()
            .map(|group| {
                let mut object = Map::new();
                object.insert("name".to_owned(), json!(group.name));
                if let Some(color) = group.color.as_ref().filter(|c| !c.is_empty()) {
                    object.insert("color".to_owned(), json!(color));
                }
                object.insert(
                    "members".to_owned(),
                    json!(
                        group
                            .members
                            .iter()
                            .map(|member| member.address.clone())
                            .collect::<Vec<_>>()
                    ),
                );
                Value::Object(object)
            })
            .collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&backup).unwrap_or_else(|_| "{}".to_owned())
}

/// An absent field is OMITTED, not written as null or "". A backup that says
/// `"name": ""` re-imports a contact whose name is the empty string.
fn exported_contact(contact: &Contact) -> Value {
    let mut object = Map::new();
    object.insert("address".to_owned(), json!(contact.address));
    if let Some(name) = contact.name.as_ref().filter(|n| !n.is_empty()) {
        object.insert("name".to_owned(), json!(name));
    }
    if let Some(note) = contact.note.as_ref().filter(|n| !n.is_empty()) {
        object.insert("note".to_owned(), json!(note));
    }
    if contact.favorite {
        object.insert("favorite".to_owned(), json!(true));
    }
    Value::Object(object)
}

/// The CSV backup: `address,name,note,favorite,groups`, groups `;`-joined.
#[must_use]
pub fn to_csv(contacts: &[Contact], groups: &[ContactGroupView]) -> String {
    let mut lines = vec!["address,name,note,favorite,groups".to_owned()];
    for contact in contacts {
        let memberships: Vec<String> = groups
            .iter()
            .filter(|group| {
                group
                    .members
                    .iter()
                    .any(|member| member.address == contact.address)
            })
            .map(|group| group.name.clone())
            .collect();
        lines.push(
            [
                contact.address.clone(),
                contact.name.clone().unwrap_or_default(),
                contact.note.clone().unwrap_or_default(),
                if contact.favorite { "true" } else { "" }.to_owned(),
                memberships.join(";"),
            ]
            .iter()
            .map(|cell| csv_cell(cell))
            .collect::<Vec<_>>()
            .join(","),
        );
    }
    lines.join("\n")
}

/// Quote only when the cell needs it — a comma, a quote or a newline.
fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// JSON or CSV, detected by extension then by shape.
pub fn parse(content: &str, filename: Option<&str>) -> Result<Parsed, Unreadable> {
    // A BOM in front of `{` is still JSON, and a BOM in front of a header is
    // still a header.
    let trimmed = content.trim_start_matches('\u{feff}').trim();
    let looks_json = filename.is_some_and(|name| name.to_lowercase().ends_with(".json"))
        || trimmed.starts_with('{');
    if looks_json {
        Ok(parse_json(trimmed))
    } else {
        parse_csv(trimmed)
    }
}

fn parse_json(text: &str) -> Parsed {
    // Unparseable JSON yields NOTHING rather than an error: the core reports
    // "0 added" and the person tries another file, which is the same outcome as
    // an empty backup and needs no second failure mode.
    let Ok(data) = serde_json::from_str::<Value>(text) else {
        return Parsed::default();
    };
    let contacts = data
        .get("contacts")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(imported_contact).collect())
        .unwrap_or_default();
    let groups = data
        .get("groups")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(ContactImportGroup {
                        name: item.get("name").and_then(Value::as_str)?.to_owned(),
                        color: item.get("color").and_then(Value::as_str).map(str::to_owned),
                        members: item
                            .get("members")
                            .and_then(Value::as_array)
                            .map(|members| {
                                members
                                    .iter()
                                    .filter_map(|m| m.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Parsed { contacts, groups }
}

fn imported_contact(value: &Value) -> Option<ContactImportEntry> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    Some(ContactImportEntry {
        address: value.get("address").and_then(Value::as_str)?.to_owned(),
        name: text("name"),
        note: text("note"),
        // `true` or the string "true" — a CSV round-tripped through a
        // spreadsheet comes back as the second.
        favorite: match value.get("favorite") {
            Some(Value::Bool(true)) => Some(true),
            Some(Value::String(text)) if text == "true" => Some(true),
            _ => None,
        },
    })
}

/// Split one CSV line, honouring quotes and doubled quotes.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    out.push(cur);
    out
}

/// Is this an EVM address? The one question the CSV heuristics turn on.
fn is_address(value: &str) -> bool {
    let stripped = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    stripped.is_some_and(|body| body.len() == 40 && body.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Which column holds the address: the header's word if it says so, else the
/// first column that actually contains one.
fn address_column(header: Option<&[String]>, rows: &[Vec<String>]) -> Option<usize> {
    if let Some(header) = header {
        if let Some(index) = header.iter().position(|h| h.to_lowercase() == "address") {
            return Some(index);
        }
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    (0..width).find(|i| {
        rows.iter()
            .any(|row| row.get(*i).is_some_and(|cell| is_address(cell)))
    })
}

struct Columns {
    address: usize,
    name: Option<usize>,
    note: Option<usize>,
    favorite: Option<usize>,
    groups: Option<usize>,
}

fn index_columns(header: Option<&[String]>, address: usize, named_address: bool) -> Columns {
    let find = |header: &[String], word: &str| header.iter().position(|h| h.to_lowercase() == word);
    // The first column that is NOT the address one — the de-facto label.
    let first_other = if address == 0 { 1 } else { 0 };

    match header {
        // The file speaks our vocabulary: take every column it names and infer
        // nothing beyond them.
        Some(header) if named_address => Columns {
            address,
            name: find(header, "name"),
            note: find(header, "note"),
            favorite: find(header, "favorite"),
            groups: find(header, "groups"),
        },
        // A foreign header (`label,wallet`): its words told us nothing, so keep
        // only what is unambiguous.
        Some(header) => Columns {
            address,
            name: find(header, "name").or(Some(first_other)),
            note: None,
            favorite: None,
            groups: None,
        },
        // Headerless in our own export order — but only when the address sits
        // where that order puts it. Anywhere else and the file has told us
        // nothing about the rest.
        None if address == 0 => Columns {
            address,
            name: Some(1),
            note: Some(2),
            favorite: Some(3),
            groups: Some(4),
        },
        None => Columns {
            address,
            name: Some(first_other),
            note: None,
            favorite: None,
            groups: None,
        },
    }
}

fn parse_csv(text: &str) -> Result<Parsed, Unreadable> {
    let lines: Vec<&str> = text
        .split(['\n', '\r'])
        .filter(|line| !line.trim().is_empty())
        .collect();
    let Some(first_line) = lines.first() else {
        return Ok(Parsed::default());
    };
    let first: Vec<String> = split_csv_line(first_line)
        .into_iter()
        .map(|cell| cell.trim().to_owned())
        .collect();
    // A first row containing an address is DATA, not a header.
    let has_header = !first.iter().any(|cell| is_address(cell));
    let rows: Vec<Vec<String>> = lines
        .iter()
        .skip(usize::from(has_header))
        .map(|line| {
            split_csv_line(line)
                .into_iter()
                .map(|cell| cell.trim().to_owned())
                .collect()
        })
        .collect();

    let header = has_header.then_some(first.as_slice());
    let named_address = header.is_some_and(|h| h.iter().any(|c| c.to_lowercase() == "address"));
    let columns = index_columns(
        header,
        address_column(header, &rows).unwrap_or(0),
        named_address,
    );

    let mut contacts = Vec::new();
    let mut group_map: Vec<(String, Vec<String>)> = Vec::new();
    let mut attempted = 0u32;
    let mut valid = 0u32;
    for cells in &rows {
        let cell = |index: Option<usize>| {
            index
                .and_then(|i| cells.get(i))
                .map(String::as_str)
                .filter(|s| !s.is_empty())
        };
        let Some(address) = cell(Some(columns.address)) else {
            // Nothing where the address goes is structure — a blank line or a
            // separator — not a contact anyone tried to import.
            continue;
        };
        let address = address.to_owned();
        attempted += 1;

        // A malformed row is carried through, NOT dropped: "is this an address"
        // is the core's question and it counts the answer. Swallowing bad rows
        // here made `invalid` structurally zero on this path.
        contacts.push(ContactImportEntry {
            address: address.clone(),
            name: cell(columns.name).map(str::to_owned),
            note: cell(columns.note).map(str::to_owned),
            favorite: cell(columns.favorite)
                .filter(|value| matches!(value.to_lowercase().as_str(), "true" | "1" | "yes"))
                .map(|_| true),
        });
        if !is_address(&address) {
            continue;
        }
        valid += 1;

        if let Some(names) = cell(columns.groups) {
            for name in names.split(';').map(str::trim).filter(|n| !n.is_empty()) {
                let lower = address.to_lowercase();
                match group_map.iter_mut().find(|(existing, _)| existing == name) {
                    Some((_, members)) => members.push(lower),
                    None => group_map.push((name.to_owned(), vec![lower])),
                }
            }
        }
    }

    // Rows that plainly meant to be contacts, and not one address among them.
    if attempted > 0 && valid == 0 {
        return Err(Unreadable);
    }
    Ok(Parsed {
        contacts,
        groups: group_map
            .into_iter()
            .map(|(name, members)| ContactImportGroup {
                name,
                color: None,
                members,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vela_core::app::contacts::{ContactKind, ContactSource};

    fn contact(address: &str, name: Option<&str>, favorite: bool) -> Contact {
        Contact {
            address: address.to_owned(),
            name: name.map(str::to_owned),
            resolved_name: None,
            resolved_source: None,
            kind: ContactKind::Eoa,
            favorite,
            note: None,
            tx_count: 0,
            last_used_ms: 0.0,
            first_seen_ms: 0.0,
            source: ContactSource::Manual,
        }
    }

    const ALICE: &str = "0xAaAa000000000000000000000000000000000001";
    const BOB: &str = "0xbBbB000000000000000000000000000000000002";

    /// A backup written here reads back here, whole.
    #[test]
    fn a_json_backup_round_trips() {
        let contacts = vec![
            contact(ALICE, Some("Alice"), true),
            contact(BOB, None, false),
        ];
        let groups = vec![ContactGroupView {
            id: "g1".to_owned(),
            name: "Family".to_owned(),
            color: Some("#ff0000".to_owned()),
            members: vec![contact(ALICE, Some("Alice"), true)],
        }];

        let text = to_json(&contacts, &groups, "2026-09-05T00:00:00.000Z");
        // The bytes every other client reads.
        assert!(text.contains("\"version\": 1"));
        assert!(text.contains("\"exportedAt\""));
        // An absent name is OMITTED — `"name": ""` re-imports an empty name.
        assert!(!text.contains("\"name\": \"\""));

        let parsed = parse(&text, Some("book.json"))
            .unwrap_or_else(|_| unreachable!("our own file is readable"));
        assert_eq!(parsed.contacts.len(), 2);
        assert_eq!(parsed.contacts[0].address, ALICE);
        assert_eq!(parsed.contacts[0].name.as_deref(), Some("Alice"));
        assert_eq!(parsed.contacts[0].favorite, Some(true));
        assert_eq!(parsed.contacts[1].name, None);
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].name, "Family");
        assert_eq!(parsed.groups[0].color.as_deref(), Some("#ff0000"));
        assert_eq!(parsed.groups[0].members, vec![ALICE.to_owned()]);
    }

    /// The CSV round trip, including a cell that needs quoting.
    #[test]
    fn a_csv_backup_round_trips_and_quotes_what_it_must() {
        let mut alice = contact(ALICE, Some("Alice, the one"), true);
        alice.note = Some("said \"hi\"".to_owned());
        let groups = vec![ContactGroupView {
            id: "g1".to_owned(),
            name: "Family".to_owned(),
            color: None,
            members: vec![alice.clone()],
        }];

        let text = to_csv(&[alice], &groups);
        assert!(text.starts_with("address,name,note,favorite,groups"));
        assert!(text.contains("\"Alice, the one\""), "{text}");
        assert!(text.contains("\"said \"\"hi\"\"\""), "{text}");

        let parsed =
            parse(&text, Some("book.csv")).unwrap_or_else(|_| unreachable!("our own file"));
        assert_eq!(parsed.contacts.len(), 1);
        assert_eq!(parsed.contacts[0].name.as_deref(), Some("Alice, the one"));
        assert_eq!(parsed.contacts[0].note.as_deref(), Some("said \"hi\""));
        assert_eq!(parsed.contacts[0].favorite, Some(true));
        assert_eq!(parsed.groups[0].members, vec![ALICE.to_lowercase()]);
    }

    /// A foreign header that does not say "address" — the data says where it is.
    ///
    /// Falling back to column 0 here is what made an import report "0 added, 0
    /// already existed": nothing imported, nothing explained.
    #[test]
    fn a_foreign_header_is_read_from_the_data_not_from_column_zero() {
        let csv = format!("label,wallet\nAlice,{ALICE}\nBob,{BOB}\n");
        let parsed =
            parse(&csv, Some("theirs.csv")).unwrap_or_else(|_| unreachable!("it has addresses"));
        assert_eq!(parsed.contacts.len(), 2);
        assert_eq!(parsed.contacts[0].address, ALICE);
        // The label beside it is the one unambiguous extra.
        assert_eq!(parsed.contacts[0].name.as_deref(), Some("Alice"));
    }

    /// A malformed row is CARRIED, so the core can count it as invalid.
    #[test]
    fn a_bad_address_reaches_the_core_rather_than_being_swallowed() {
        let csv = format!("address,name\n{ALICE},Alice\nnot-an-address,Nobody\n");
        let parsed = parse(&csv, Some("book.csv")).unwrap_or_else(|_| unreachable!("one is valid"));
        assert_eq!(
            parsed.contacts.len(),
            2,
            "the bad row is the core's to judge"
        );
        assert_eq!(parsed.contacts[1].address, "not-an-address");
    }

    /// A file that plainly held contacts and yielded no address SAYS so.
    #[test]
    fn an_unreadable_csv_is_an_error_not_an_empty_success() {
        let csv = "name,email\nAlice,a@example.com\nBob,b@example.com\n";
        assert_eq!(parse(csv, Some("theirs.csv")), Err(Unreadable));

        // An empty file is not unreadable — it is empty.
        assert_eq!(parse("", Some("empty.csv")), Ok(Parsed::default()));
        // And unparseable JSON yields nothing rather than a second failure mode.
        assert_eq!(parse("{ not json", Some("x.json")), Ok(Parsed::default()));
    }

    /// A headerless file in our own order, and one whose address is elsewhere.
    #[test]
    fn a_headerless_file_is_positional_only_when_the_address_is_where_it_should_be() {
        let ours = format!("{ALICE},Alice,a note,true,Family\n");
        let parsed = parse(&ours, None).unwrap_or_else(|_| unreachable!("valid"));
        assert_eq!(parsed.contacts[0].name.as_deref(), Some("Alice"));
        assert_eq!(parsed.contacts[0].note.as_deref(), Some("a note"));
        assert_eq!(parsed.contacts[0].favorite, Some(true));
        assert_eq!(parsed.groups[0].name, "Family");

        // Address in column 1: the file has told us nothing about columns 2+,
        // so only the label beside it is taken.
        let theirs = format!("Alice,{ALICE},something,else\n");
        let parsed = parse(&theirs, None).unwrap_or_else(|_| unreachable!("valid"));
        assert_eq!(parsed.contacts[0].address, ALICE);
        assert_eq!(parsed.contacts[0].name.as_deref(), Some("Alice"));
        assert_eq!(parsed.contacts[0].note, None, "column 2 means nothing here");
    }
}
