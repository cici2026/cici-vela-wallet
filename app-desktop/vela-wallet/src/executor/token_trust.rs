//! The only place the `token_trust` machine touches the outside world.
//!
//! Seven operations: the incoming-transfer scan (`eth_blockNumber`,
//! `eth_getLogs`, `eth_getBlockByNumber`), a batched ERC-20 metadata read, and
//! the custom-token ledger.
//!
//! **Ported from** `src/services/transfer-monitor.ts` and
//! `src/services/token-metadata.ts` @ `c513c4c6` (FR-006). Every *decision* —
//! which contracts are trusted, whether a log is admissible, whether a
//! simulation delta may be believed, when a token may be written — is
//! `token_trust.rs`'s 1,937 lines and is not re-derived here.
//!
//! ## The range cap is the pool's word, not a string match
//!
//! `eth_getLogs` fails two ways that must not be confused: the endpoint is
//! broken, or the endpoint is fine and the span was too wide. Only the second
//! is worth retrying with a narrower window, and only the first is worth
//! failing over — the next endpoint usually has the same cap, and banning a
//! healthy endpoint over it is how a pool loses its best RPC.
//!
//! The classification already exists: `rpc_pool` parses the wording and answers
//! `PoolError::RangeCap { max_span }`. This file maps that one error onto
//! `TrustLogsOutcome::RangeCapped` and everything else onto `Failed`. It does
//! not read an error message.
//!
//! ## What is not wired, and why it is not a silent gap
//!
//! Three of this machine's inputs are *events*, not operations:
//! `HeldChainsSnapshot` (which chains this account uses),
//! `HeldTokensSnapshot` (the ERC-20s it holds) and `RegistryTokensSnapshot`
//! (a chain's canonical stablecoins). All three are facts a background worker
//! learns, and pushing an event into a resident from a worker is the same
//! capability `Event::ChainAssetsArrived` needs and this cut does not build.
//!
//! Unfed, the core degrades exactly as it says it does: an empty held-chains
//! list means the poll falls back to `DEFAULT_MONITOR_CHAINS`, and a cold
//! registry means the trusted set is the customs plus the native sentinels —
//! "everything unverified, the safe direction". Nothing here fills that gap
//! with a guess.

use gpui::App;
use serde_json::{Value, json};

use vela_core::app::token_trust::{
    Event, TokenTrust, TrustLogsOutcome, TrustMetaEntry, TrustOperation, TrustRawLog,
    TrustShellResult, TrustTokenMeta,
};

use crate::executor::pool::{self, PoolError};
use crate::executor::{abi, custom_tokens};
use crate::resident::{Answer, Machine};
use crate::session;

/// One routed call, returning the `result` member.
fn rpc(chain_id: u32, method: &str, params: Value) -> Result<Value, PoolError> {
    pool::call(chain_id, method, params)
        .map(|body| body.get("result").cloned().unwrap_or(Value::Null))
}

/// One raw log, as the core reads it. A log missing the fields that identify it
/// is dropped rather than defaulted: an entry with no transaction hash cannot
/// be de-duped, and a receipt that cannot be de-duped is a row that reappears.
fn to_raw_log(value: &Value) -> Option<TrustRawLog> {
    Some(TrustRawLog {
        address: value.get("address").and_then(Value::as_str)?.to_owned(),
        topics: value
            .get("topics")
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|topic| topic.as_str().map(str::to_owned))
            .collect(),
        data: value
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or("0x")
            .to_owned(),
        transaction_hash: value
            .get("transactionHash")
            .and_then(Value::as_str)?
            .to_owned(),
        // The core reads an absent one as `0x0`, so absent is passed on as
        // absent rather than substituted here.
        block_number: value
            .get("blockNumber")
            .and_then(Value::as_str)
            .map(str::to_owned),
        log_index: value
            .get("logIndex")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// `symbol()` and `decimals()` for many addresses, in one `aggregate3`.
///
/// **Every requested address is answered**, resolved or not: the core records
/// an unresolvable token as a fact worth remembering, and a missing entry would
/// instead read as a question never asked.
///
/// All-or-nothing per token. A symbol with no scale renders an amount at the
/// wrong magnitude, which is worse than an unnamed token.
fn erc20_meta(chain_id: u32, addrs: &[String]) -> Vec<TrustMetaEntry> {
    let mut calls = Vec::with_capacity(addrs.len() * 2);
    for addr in addrs {
        calls.push(abi::Call3 {
            target: addr.clone(),
            call_data: abi::enc_symbol(),
        });
        calls.push(abi::Call3 {
            target: addr.clone(),
            call_data: abi::enc_decimals(),
        });
    }
    let results = if calls.is_empty() {
        Vec::new()
    } else {
        rpc(
            chain_id,
            "eth_call",
            json!([{ "to": abi::MULTICALL3, "data": abi::enc_aggregate3(&calls) }, "latest"]),
        )
        .ok()
        .and_then(|value| value.as_str().map(abi::dec_aggregate3))
        .unwrap_or_default()
    };

    addrs
        .iter()
        .enumerate()
        .map(|(index, addr)| {
            let answered = |at: usize| {
                results
                    .get(at)
                    .filter(|result| result.success)
                    .map(|result| result.data.as_str())
            };
            let symbol = answered(index * 2).and_then(abi::dec_string);
            let decimals = answered(index * 2 + 1).and_then(abi::dec_u8);
            TrustMetaEntry {
                addr: addr.clone(),
                meta: match (symbol, decimals) {
                    (Some(symbol), Some(decimals)) => Some(TrustTokenMeta {
                        symbol,
                        decimals: u32::from(decimals),
                    }),
                    _ => None,
                },
            }
        })
        .collect()
}

impl Machine for TokenTrust {
    const LABEL: &'static str = "token_trust";

    fn boot_event(cx: &App) -> Event {
        // The account, with no held chains yet. An empty list is not a guess:
        // the core reads it as "brand-new wallet" and polls its own default
        // set, which is exactly what is true before any balance has answered.
        Event::HeldChainsSnapshot {
            address: session::view(cx).address,
            chain_ids: Vec::new(),
        }
    }

    fn perform(operation: &TrustOperation) -> Answer<TrustShellResult> {
        match operation {
            TrustOperation::RpcBlockNumber { address, chain_id } => {
                let (address, chain_id) = (address.clone(), *chain_id);
                Answer::Blocking(Box::new(move || TrustShellResult::BlockNumber {
                    address,
                    chain_id,
                    block_hex: rpc(chain_id, "eth_blockNumber", json!([]))
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned)),
                }))
            }

            TrustOperation::RpcGetLogs {
                address,
                chain_id,
                from_block,
                to_block,
                recipient_topic,
                contracts,
            } => {
                let (address, chain_id) = (address.clone(), *chain_id);
                let mut filter = json!({
                    "fromBlock": from_block,
                    // topics[1] is the sender and is deliberately unfiltered:
                    // this asks who RECEIVED, from anyone.
                    "topics": [
                        vela_core::app::token_trust::TRANSFER_TOPIC,
                        Value::Null,
                        recipient_topic,
                    ],
                });
                if let Some(object) = filter.as_object_mut() {
                    object.insert("toBlock".to_owned(), json!(to_block));
                    // The allowlist. An EMPTY list is not "no filter": the core
                    // asks with the contracts it trusts, and dropping the key
                    // would widen the query to every token on the chain —
                    // which is precisely the spam channel the allowlist exists
                    // to close.
                    object.insert("address".to_owned(), json!(contracts));
                }
                Answer::Blocking(Box::new(move || {
                    let outcome = match pool::call(chain_id, "eth_getLogs", json!([filter])) {
                        Ok(body) => TrustLogsOutcome::Ok {
                            logs: body
                                .get("result")
                                .and_then(Value::as_array)
                                .map(|logs| logs.iter().filter_map(to_raw_log).collect())
                                .unwrap_or_default(),
                        },
                        // The pool already parsed the endpoint's wording. A cap
                        // it could not put a number to arrives as 0, which the
                        // core reads as "narrow conservatively".
                        Err(PoolError::RangeCap { max_span }) => TrustLogsOutcome::RangeCapped {
                            cap: if max_span.is_finite() && max_span > 0.0 {
                                max_span as u32
                            } else {
                                0
                            },
                        },
                        Err(PoolError::Failed { .. } | PoolError::Unavailable) => {
                            TrustLogsOutcome::Failed
                        }
                    };
                    TrustShellResult::Logs {
                        address,
                        chain_id,
                        outcome,
                    }
                }))
            }

            TrustOperation::RpcGetBlockByNumber {
                address,
                chain_id,
                block,
            } => {
                let (address, chain_id, block) = (address.clone(), *chain_id, block.clone());
                Answer::Blocking(Box::new(move || {
                    let header = rpc(chain_id, "eth_getBlockByNumber", json!([block, false])).ok();
                    let timestamp_sec = header
                        .as_ref()
                        .and_then(|value| value.get("timestamp"))
                        .and_then(Value::as_str)
                        .and_then(|hex| u64::from_str_radix(hex.trim_start_matches("0x"), 16).ok())
                        // Precision: 2^53 seconds is past any block this
                        // wallet will see, but a garbage word is not, and it
                        // must not become a plausible date.
                        .filter(|seconds| *seconds < (1u64 << 53))
                        .map(|seconds| seconds as f64);
                    TrustShellResult::BlockTimestamp {
                        address,
                        chain_id,
                        block_number: abi::dec_hex_quantity(&block)
                            .and_then(|digits| digits.parse::<f64>().ok())
                            .unwrap_or(0.0),
                        // `None` = the lookup failed; the core falls the
                        // transfer back to "now" itself.
                        timestamp_sec,
                        now_ms: crate::executor::now_ms(),
                    }
                }))
            }

            TrustOperation::MulticallErc20Meta { chain_id, addrs } => {
                let (chain_id, addrs) = (*chain_id, addrs.clone());
                Answer::Blocking(Box::new(move || TrustShellResult::ErcMeta {
                    chain_id,
                    entries: erc20_meta(chain_id, &addrs),
                }))
            }

            TrustOperation::ReadCustomTokens => Answer::Now(TrustShellResult::CustomTokens {
                // `Some(vec![])` is "there are none"; `None` would be "the read
                // failed", which fails the admission closed. A read that
                // returned nothing is the first of those.
                tokens: Some(
                    custom_tokens::read()
                        .iter()
                        .map(custom_tokens::StoredToken::to_trust)
                        .collect(),
                ),
            }),

            TrustOperation::WriteCustomToken { token } => {
                Answer::Now(TrustShellResult::TokenWritten {
                    ok: custom_tokens::save(custom_tokens::StoredToken::from(token)),
                })
            }

            // The balance fetch reads the token list on every run, so there is
            // no separate token cache to drop on the desktop. Answered, because
            // a skipped operation leaves the core waiting forever.
            TrustOperation::InvalidateTokenCache { .. } => {
                Answer::Now(TrustShellResult::CacheInvalidated)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::storage;
    use vela_core::app::token_trust::TrustCustomToken;

    fn perform(operation: TrustOperation) -> TrustShellResult {
        match TokenTrust::perform(&operation) {
            Answer::Now(result) => result,
            _ => unreachable!("this operation is local"),
        }
    }

    fn blocking(operation: TrustOperation) -> TrustShellResult {
        match TokenTrust::perform(&operation) {
            Answer::Blocking(work) => work(),
            _ => unreachable!("this operation is network work"),
        }
    }

    /// A log that cannot be identified is dropped, not defaulted.
    #[test]
    fn a_log_without_an_identity_is_dropped_rather_than_invented() {
        let good = json!({
            "address": "0xaaa",
            "topics": ["0xddf2", "0x00", "0x11"],
            "data": "0x2a",
            "transactionHash": "0xdead",
            "blockNumber": "0x10",
            "logIndex": "0x1",
        });
        let log = to_raw_log(&good).unwrap_or_else(|| unreachable!("a complete log"));
        assert_eq!(log.address, "0xaaa");
        assert_eq!(log.topics.len(), 3);
        assert_eq!(log.block_number.as_deref(), Some("0x10"));

        // No transaction hash: nothing to de-dupe by, so the row would come
        // back on every poll.
        assert_eq!(
            to_raw_log(&json!({ "address": "0xaaa", "topics": [] })),
            None
        );
        // No address: nothing to check against the allowlist.
        assert_eq!(
            to_raw_log(&json!({ "topics": [], "transactionHash": "0xdead" })),
            None
        );
        // An absent block number stays absent — the core reads it as `0x0`,
        // and substituting one here would hide that from its own rule.
        let sparse = json!({
            "address": "0xaaa",
            "topics": [],
            "transactionHash": "0xdead",
        });
        let log = to_raw_log(&sparse).unwrap_or_else(|| unreachable!("identifiable"));
        assert_eq!(log.block_number, None);
        assert_eq!(
            log.data, "0x",
            "an absent data member is empty, not missing"
        );
    }

    /// The ledger this machine shares with `manage_tokens`.
    #[test]
    fn a_token_admitted_here_is_the_same_record_the_panel_writes() {
        storage::tests::with_temp_state("trust-ledger", || {
            let token = TrustCustomToken {
                id: "100_0xaaa".to_owned(),
                chain_id: 100,
                contract_address: "0xaaa".to_owned(),
                symbol: "AAA".to_owned(),
                name: "Triple A".to_owned(),
                decimals: 6,
            };
            assert_eq!(
                perform(TrustOperation::WriteCustomToken {
                    token: token.clone()
                }),
                TrustShellResult::TokenWritten { ok: true }
            );

            // Read back through this machine's own operation…
            match perform(TrustOperation::ReadCustomTokens) {
                TrustShellResult::CustomTokens { tokens } => {
                    let tokens = tokens.unwrap_or_else(|| unreachable!("the read succeeded"));
                    assert_eq!(tokens, vec![token]);
                }
                other => unreachable!("wrong variant: {other:?}"),
            }
            // …and the derived display name the core does not carry is there.
            assert_eq!(custom_tokens::read()[0].network_name, "Gnosis");
        });
    }

    /// An empty ledger is `Some(vec![])`, never `None`.
    ///
    /// `None` means the READ failed and fails the admission closed. Conflating
    /// the two would make a wallet with no custom tokens look like a wallet
    /// whose storage is broken.
    #[test]
    fn no_custom_tokens_is_not_a_failed_read() {
        storage::tests::with_temp_state("trust-empty-ledger", || {
            match perform(TrustOperation::ReadCustomTokens) {
                TrustShellResult::CustomTokens { tokens } => {
                    assert_eq!(tokens, Some(Vec::new()));
                }
                other => unreachable!("wrong variant: {other:?}"),
            }
        });
    }

    /// The cache invalidation is answered even though there is nothing to drop.
    #[test]
    fn the_cache_invalidation_is_answered_rather_than_skipped() {
        assert_eq!(
            perform(TrustOperation::InvalidateTokenCache {
                address: "0xabc".to_owned()
            }),
            TrustShellResult::CacheInvalidated
        );
    }

    /// Every requested address is answered, and a token that answered only half
    /// is `None` rather than half a token.
    #[test]
    #[ignore = "reads real ERC-20s on Gnosis"]
    fn a_batched_metadata_read_answers_every_address_it_was_given() {
        storage::tests::with_temp_state("trust-meta-live", || {
            // USDC on Gnosis, then an address with no contract behind it.
            let addrs = vec![
                "0xDDAfbb505ad214D7b80b1f830fcCc89B60fb7A83".to_owned(),
                "0x0000000000000000000000000000000000000001".to_owned(),
            ];
            let result = blocking(TrustOperation::MulticallErc20Meta {
                chain_id: 100,
                addrs: addrs.clone(),
            });
            match result {
                TrustShellResult::ErcMeta { chain_id, entries } => {
                    assert_eq!(chain_id, 100);
                    for entry in &entries {
                        println!("    {} → {:?}", entry.addr, entry.meta);
                    }
                    assert_eq!(entries.len(), addrs.len(), "every address is answered");
                    let usdc = entries[0]
                        .meta
                        .clone()
                        .unwrap_or_else(|| unreachable!("USDC did not answer"));
                    assert_eq!(usdc.decimals, 6);
                    assert!(!usdc.symbol.is_empty());
                    // Looked up and unresolvable is a fact, not a default.
                    assert_eq!(entries[1].meta, None);
                }
                other => unreachable!("wrong variant: {other:?}"),
            }
        });
    }

    /// The scan's three RPCs, against a real chain.
    #[test]
    #[ignore = "reads Gnosis"]
    fn the_scan_reads_a_block_its_timestamp_and_its_logs() {
        storage::tests::with_temp_state("trust-scan-live", || {
            const GOLDEN: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";
            let block_hex = match blocking(TrustOperation::RpcBlockNumber {
                address: GOLDEN.to_owned(),
                chain_id: 100,
            }) {
                TrustShellResult::BlockNumber { block_hex, .. } => {
                    block_hex.unwrap_or_else(|| unreachable!("no block number"))
                }
                other => unreachable!("wrong variant: {other:?}"),
            };
            let latest = abi::dec_hex_quantity(&block_hex)
                .and_then(|digits| digits.parse::<u64>().ok())
                .unwrap_or_else(|| unreachable!("not a quantity: {block_hex}"));
            println!("    latest block: {latest}");
            assert!(latest > 30_000_000, "Gnosis is well past this height");

            match blocking(TrustOperation::RpcGetBlockByNumber {
                address: GOLDEN.to_owned(),
                chain_id: 100,
                block: block_hex.clone(),
            }) {
                TrustShellResult::BlockTimestamp {
                    block_number,
                    timestamp_sec,
                    now_ms,
                    ..
                } => {
                    let seconds =
                        timestamp_sec.unwrap_or_else(|| unreachable!("no block timestamp"));
                    println!("    block {block_number} at {seconds}");
                    #[allow(clippy::cast_precision_loss, reason = "a block height")]
                    let expected = latest as f64;
                    assert!((block_number - expected).abs() < 1.0);
                    // The header's time is within an hour of this machine's,
                    // which is what says we read a timestamp and not a word.
                    assert!((seconds * 1000.0 - now_ms).abs() < 3_600_000.0);
                }
                other => unreachable!("wrong variant: {other:?}"),
            }

            // The allowlist restricted to one real token: an answer, not a cap
            // and not a failure.
            let recipient_topic =
                format!("0x{:0>64}", GOLDEN.trim_start_matches("0x").to_lowercase());
            match blocking(TrustOperation::RpcGetLogs {
                address: GOLDEN.to_owned(),
                chain_id: 100,
                from_block: format!("0x{:x}", latest.saturating_sub(50)),
                to_block: format!("0x{latest:x}"),
                recipient_topic,
                contracts: vec!["0xDDAfbb505ad214D7b80b1f830fcCc89B60fb7A83".to_owned()],
            }) {
                TrustShellResult::Logs { outcome, .. } => {
                    println!("    logs: {outcome:?}");
                    // Zero logs in fifty blocks is the expected answer and is
                    // still `Ok` — nothing found is not a failure.
                    assert!(
                        matches!(outcome, TrustLogsOutcome::Ok { .. }),
                        "a healthy endpoint over 50 blocks must not fail"
                    );
                }
                other => unreachable!("wrong variant: {other:?}"),
            }
        });
    }
}
