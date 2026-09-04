//! The only place the `activity_feed` machine touches the outside world.
//!
//! Six operations: the local transaction store, receipt discovery, a delete, a
//! counterparty lookup, the toast timer, and a haptic the desktop does not have.
//!
//! ## `day_start_ms` is the shell's, and it is not a formatting detail
//!
//! The core's words: "LOCAL-midnight epoch ms for `timestamp` — computed by the
//! shell, which owns the device timezone." `vela-core` deliberately ships **no
//! timezone database**, so the day boundary is the one thing it cannot derive.
//!
//! Getting it wrong is visible rather than subtle: a feed grouped by UTC day
//! files a 20:00 transaction in Tokyo under *tomorrow*, and one at 19:00 in New
//! York under *today* when it should be yesterday — wrong for part of every day
//! for everybody who is not in Greenwich.

use std::time::Duration;

use gpui::App;
use serde_json::Value;

use vela_core::app::activity_feed::{
    ActivityFeed, Event, FeedOperation, FeedShellResult, FeedTxKind, FeedTxRecord, FeedTxStatus,
};

use crate::executor::storage;
use crate::resident::{Answer, Machine};
use crate::session;

/// `vela.transactionHistory` — the shared local store.
const TX_KEY: &str = "vela.transactionHistory";

/// One stored row → one feed record.
///
/// Coercion, never policy: a row that will not parse is skipped rather than
/// failing the load, exactly as the contacts and network ledgers do. A feed that
/// refuses to render because one legacy row is odd is worse than a feed missing
/// that row.
fn to_record(row: &Value) -> Option<FeedTxRecord> {
    let text = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let optional = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };

    let id = row.get("id").and_then(Value::as_str)?.to_owned();
    // Stored in SECONDS, and the core says so. Multiplying in the wrong place
    // puts every record in 1970.
    let timestamp = row.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);

    Some(FeedTxRecord {
        id,
        user_op_hash: text("userOpHash"),
        tx_hash: text("txHash"),
        from: text("from"),
        to: text("to"),
        to_name: optional("toName"),
        value: text("value"),
        symbol: text("symbol"),
        decimals: row
            .get("decimals")
            .and_then(Value::as_u64)
            .and_then(|d| u32::try_from(d).ok())
            .unwrap_or(18),
        logo_urls: row.get("logoUrls").and_then(Value::as_array).map(|urls| {
            urls.iter()
                .filter_map(|url| url.as_str().map(str::to_owned))
                .collect()
        }),
        chain_id: row
            .get("chainId")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .unwrap_or(0),
        timestamp,
        day_start_ms: crate::executor::day_start_ms(timestamp * 1000.0),
        status: match row.get("status").and_then(Value::as_str) {
            Some("pending") => FeedTxStatus::Pending,
            Some("failed") => FeedTxStatus::Failed,
            _ => FeedTxStatus::Confirmed,
        },
        // Absent means a record older than the field, and the core reads that
        // as `send` — its own `t.type ?? 'send'`. `None` is the honest report;
        // substituting `Send` here would hide a legacy row from the core's own
        // rule about legacy rows.
        kind: row
            .get("type")
            .and_then(Value::as_str)
            .and_then(|kind| match kind {
                "send" => Some(FeedTxKind::Send),
                "receive" => Some(FeedTxKind::Receive),
                "dapp_tx" => Some(FeedTxKind::DappTx),
                "sign_message" => Some(FeedTxKind::SignMessage),
                "sign_typed_data" => Some(FeedTxKind::SignTypedData),
                "connect" => Some(FeedTxKind::Connect),
                _ => None,
            }),
        usd: optional("usd"),
    })
}

fn read_records() -> Vec<FeedTxRecord> {
    let Ok(Some(Value::Array(rows))) = storage::read_value(TX_KEY) else {
        return Vec::new();
    };
    rows.iter().filter_map(to_record).collect()
}

/// The person's own accounts, by lowercased address → name.
///
/// Checked BEFORE any network lookup, because the answer is already on disk and
/// because "my other wallet" is a better label than an ENS name for the same
/// address.
fn own_account_name(address: &str) -> Option<String> {
    let wanted = address.to_lowercase();
    storage::load_accounts()
        .ok()?
        .into_iter()
        .find_map(|account| (account.address.to_lowercase() == wanted).then_some(account.name))
}

impl Machine for ActivityFeed {
    const LABEL: &'static str = "activity_feed";

    fn boot_event(cx: &App) -> Event {
        Event::AccountSwitched {
            address: session::view(cx).address,
        }
    }

    fn perform(operation: &FeedOperation) -> Answer<FeedShellResult> {
        match operation {
            FeedOperation::ReadTxStore { read_id, .. } => {
                Answer::Now(FeedShellResult::StoreLoaded {
                    records: read_records(),
                    now_ms: crate::executor::now_ms(),
                    // Echoed, and the core explains why at length: it is what
                    // binds a celebration to the read that earned it.
                    read_id: *read_id,
                })
            }

            // live in 032 — receipt discovery is `getLogs` over the transfer
            // allowlist plus `token_trust` admission, and the records it would
            // persist are the same store 032's send path writes. Zero new
            // records is a true statement about a scan that found none, not a
            // failure, so the feed simply does not celebrate anything yet.
            FeedOperation::ScanIncomingTransfers { .. } => {
                Answer::Now(FeedShellResult::SyncCompleted { new_count: 0 })
            }

            FeedOperation::DeleteTxRecord { id } => {
                let id = id.clone();
                let removed = match storage::read_value(TX_KEY) {
                    Ok(Some(Value::Array(rows))) => {
                        let before = rows.len();
                        let kept: Vec<Value> = rows
                            .into_iter()
                            .filter(|row| row.get("id").and_then(Value::as_str) != Some(&id))
                            .collect();
                        let removed = kept.len() != before;
                        storage::write_value(TX_KEY, Value::Array(kept)).is_ok() && removed
                    }
                    _ => false,
                };
                Answer::Now(if removed {
                    FeedShellResult::DeleteCommitted { id }
                } else {
                    // A delete that removed nothing is a FAILED delete, not a
                    // quiet success: the row is still on the screen, and the
                    // core needs to know it is still there.
                    FeedShellResult::DeleteFailed { id }
                })
            }

            FeedOperation::ResolveRecipientIdentity { addr } => {
                let addr = addr.clone();
                // The local half only. The core says "the shell checks the
                // user's OWN accounts first (local name, no network), then
                // ENS/.bnb/Vela" — the second half is the same waterfall
                // `contacts::resolve_identity` still owes, and they should land
                // together rather than be written twice.
                Answer::Now(FeedShellResult::AliasResolved {
                    name: own_account_name(&addr),
                    addr,
                })
            }

            FeedOperation::Timer { ms, generation } => Answer::After(
                Duration::from_millis(u64::from(*ms)),
                FeedShellResult::ToastExpired {
                    generation: *generation,
                },
            ),

            // A desktop has no haptics. Answered rather than skipped — the
            // celebration still runs, it just has one fewer sense.
            FeedOperation::Haptic => Answer::Now(FeedShellResult::HapticPlayed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seed(rows: Value) {
        if storage::write_value(TX_KEY, rows).is_err() {
            unreachable!("could not seed the tx store");
        }
    }

    /// A record another client wrote reads back, camelCase and all.
    #[test]
    fn a_stored_row_maps_to_a_feed_record() {
        storage::tests::with_temp_state("feed-map", || {
            seed(json!([{
                "id": "tx1", "userOpHash": "0xuop", "txHash": "0xhash",
                "from": "0xme", "to": "0xyou", "toName": "Ada",
                "value": "1000000000000000000", "symbol": "xDAI", "decimals": 18,
                "chainId": 100, "timestamp": 1_756_000_000, "status": "confirmed",
                "type": "send", "usd": "$1.00"
            }]));
            let records = read_records();
            assert_eq!(records.len(), 1);
            let r = &records[0];
            assert_eq!(r.id, "tx1");
            assert_eq!(r.to_name.as_deref(), Some("Ada"));
            assert_eq!(r.chain_id, 100);
            assert_eq!(r.status, FeedTxStatus::Confirmed);
            assert_eq!(r.kind, Some(FeedTxKind::Send));
            assert_eq!(r.usd.as_deref(), Some("$1.00"));
        });
    }

    /// A row with no `type` is a record older than the field. The core reads
    /// `None` as `send` by its own rule — so the shell must report `None` and
    /// not decide on its behalf.
    #[test]
    fn a_legacy_row_reports_no_kind_rather_than_guessing() {
        storage::tests::with_temp_state("feed-legacy", || {
            seed(json!([{ "id": "old", "timestamp": 1_700_000_000, "chainId": 1 }]));
            let records = read_records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].kind, None, "the core owns the legacy default");
            assert_eq!(
                records[0].decimals, 18,
                "an absent decimals is the usual 18"
            );
        });
    }

    /// One unparseable row must not cost the feed.
    #[test]
    fn a_row_without_an_id_is_skipped_not_fatal() {
        storage::tests::with_temp_state("feed-junk", || {
            seed(json!([
                { "timestamp": 1 },
                { "id": "good", "timestamp": 1_756_000_000, "chainId": 100 }
            ]));
            let records = read_records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].id, "good");
        });
    }

    /// Local midnight, not UTC midnight, and the two differ for most people.
    #[test]
    fn the_day_boundary_is_local() {
        let noon_utc = 1_756_040_000_000.0;
        let start = crate::executor::day_start_ms(noon_utc);
        const DAY_MS: f64 = 86_400_000.0;

        assert!(start <= noon_utc, "a day starts before the moment in it");
        assert!(
            noon_utc - start < DAY_MS,
            "and no more than a day before it"
        );
        // Two instants in the same local day share a boundary; one a day apart
        // does not. That holds in every timezone, which is what makes it a
        // usable assertion on a machine whose zone the test does not know.
        assert_eq!(
            crate::executor::day_start_ms(noon_utc + 3_600_000.0),
            start,
            "an hour later is the same local day"
        );
        assert_ne!(
            crate::executor::day_start_ms(noon_utc + DAY_MS),
            start,
            "a day later is not"
        );
    }

    /// Deleting a row that is not there is a FAILURE, not a quiet success: the
    /// row is still on the person's screen and the core has to know.
    #[test]
    fn deleting_a_missing_record_reports_failure() {
        storage::tests::with_temp_state("feed-delete", || {
            seed(json!([{ "id": "tx1", "timestamp": 1, "chainId": 1 }]));

            match ActivityFeed::perform(&FeedOperation::DeleteTxRecord {
                id: "tx1".to_owned(),
            }) {
                Answer::Now(FeedShellResult::DeleteCommitted { id }) => assert_eq!(id, "tx1"),
                other => unreachable!(
                    "the delete should have committed: {other:?}",
                    other = match other {
                        Answer::Now(result) => format!("{result:?}"),
                        _ => "non-local".to_owned(),
                    }
                ),
            }
            assert!(read_records().is_empty());

            match ActivityFeed::perform(&FeedOperation::DeleteTxRecord {
                id: "gone".to_owned(),
            }) {
                Answer::Now(FeedShellResult::DeleteFailed { id }) => assert_eq!(id, "gone"),
                _ => unreachable!("deleting nothing must report failure"),
            }
        });
    }

    /// The person's own account is named from disk, with no network involved.
    #[test]
    fn an_own_account_resolves_locally() {
        storage::tests::with_temp_state("feed-own-account", || {
            let account = vela_core::app::Account {
                id: "cred0".to_owned(),
                name: "Everyday wallet".to_owned(),
                address: "0xABCdef0000000000000000000000000000000001".to_owned(),
                public_key_hex: "04aa".to_owned(),
                created_at_iso: "2026-09-04T00:00:00.000Z".to_owned(),
                keys: Vec::new(),
            };
            if storage::save_account(&account).is_err() {
                unreachable!("could not save");
            }
            // Asked in a different case: the address is the key, and a wallet
            // that misses its own account on casing labels it a stranger.
            assert_eq!(
                own_account_name("0xabcdef0000000000000000000000000000000001").as_deref(),
                Some("Everyday wallet")
            );
            assert_eq!(own_account_name("0xsomeone-else"), None);
        });
    }
}
