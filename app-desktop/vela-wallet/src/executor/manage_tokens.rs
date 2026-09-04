//! The only place the `manage_tokens` machine touches the outside world.
//!
//! Five operations: an ERC-20 metadata read, the custom-token ledger, and a
//! cache invalidation.
//!
//! ## Three calls, not a multicall — for now
//!
//! The operation is named `MulticallErc20Meta` because that is what the web does
//! (one `aggregate3` to Multicall3). This asks `symbol()`, `name()` and
//! `decimals()` separately: three round trips instead of one, no Multicall3
//! encoding, and the same answer. The name is the core's word for *what it
//! wants*, not an instruction about how — and folding them into one call later
//! changes nothing it sees.
//!
//! What is NOT deferred is the failure rule. Metadata is all-or-nothing: a token
//! with a symbol and no decimals renders an amount at the wrong magnitude, which
//! is worse than refusing to add it. `None` unless all three answered.

use gpui::App;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use vela_core::app::manage_tokens::{
    Event, ManageTokens, MtokCustomToken, MtokOperation, MtokShellResult, MtokTokenMeta,
};

use crate::executor::{pool, storage};
use crate::resident::{Answer, Machine};

/// `vela.customTokens` — the shared ledger.
const TOKENS_KEY: &str = "vela.customTokens";

/// ERC-20 selectors. `keccak("symbol()")[..4]` and friends, as constants
/// because they are facts rather than computations.
const SEL_SYMBOL: &str = "0x95d89b41";
const SEL_NAME: &str = "0x06fdde03";
const SEL_DECIMALS: &str = "0x313ce567";

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct StoredToken {
    id: String,
    chain_id: u32,
    contract_address: String,
    symbol: String,
    name: String,
    decimals: u8,
    network_name: String,
}

impl From<StoredToken> for MtokCustomToken {
    fn from(s: StoredToken) -> Self {
        Self {
            id: s.id,
            chain_id: s.chain_id,
            contract_address: s.contract_address,
            symbol: s.symbol,
            name: s.name,
            decimals: s.decimals,
            network_name: s.network_name,
        }
    }
}

impl From<&MtokCustomToken> for StoredToken {
    fn from(t: &MtokCustomToken) -> Self {
        Self {
            id: t.id.clone(),
            chain_id: t.chain_id,
            contract_address: t.contract_address.clone(),
            symbol: t.symbol.clone(),
            name: t.name.clone(),
            decimals: t.decimals,
            network_name: t.network_name.clone(),
        }
    }
}

/// The custom-token ledger, as the core's own type.
///
/// `pub` because the balance fetch reads the same list: one ledger, one shape,
/// one place that knows how `vela.customTokens` is spelled on disk.
pub fn read_tokens() -> Vec<MtokCustomToken> {
    let Ok(Some(Value::Array(items))) = storage::read_value(TOKENS_KEY) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<StoredToken>(item).ok())
        .map(MtokCustomToken::from)
        .collect()
}

fn write_tokens(tokens: &[MtokCustomToken]) -> bool {
    let encoded = Value::Array(
        tokens
            .iter()
            .map(|token| serde_json::to_value(StoredToken::from(token)).unwrap_or(Value::Null))
            .collect(),
    );
    storage::write_value(TOKENS_KEY, encoded).is_ok()
}

/// One `eth_call`, returning the raw hex result.
fn eth_call(chain_id: u32, to: &str, data: &str) -> Option<String> {
    pool::call(
        chain_id,
        "eth_call",
        json!([{ "to": to, "data": data }, "latest"]),
    )
    .ok()?
    .get("result")
    .and_then(Value::as_str)
    .map(str::to_owned)
}

/// Decode an ABI-encoded `string` return: offset, length, then the bytes.
///
/// Hand-decoded rather than routed through `alloy` because the shape is fixed
/// and the failure mode matters more than the generality: a malformed answer
/// must produce `None`, not a panic and not a mojibake symbol that then gets
/// saved into somebody's token list forever.
fn decode_string(hex: &str) -> Option<String> {
    let bytes = hex_bytes(hex)?;
    if bytes.len() < 64 {
        return None;
    }
    let offset = usize::try_from(u64::from_be_bytes(bytes[24..32].try_into().ok()?)).ok()?;
    let length_at = offset.checked_add(24)?;
    if bytes.len() < length_at.checked_add(8)? {
        return None;
    }
    let length = usize::try_from(u64::from_be_bytes(
        bytes[length_at..length_at + 8].try_into().ok()?,
    ))
    .ok()?;
    let start = offset.checked_add(32)?;
    let end = start.checked_add(length)?;
    if bytes.len() < end {
        return None;
    }
    String::from_utf8(bytes[start..end].to_vec())
        .ok()
        .map(|text| text.trim_matches('\0').to_owned())
        .filter(|text| !text.is_empty())
}

/// Decode an ABI-encoded `uint8`: the last byte of the word.
fn decode_u8(hex: &str) -> Option<u8> {
    let bytes = hex_bytes(hex)?;
    (bytes.len() >= 32).then(|| bytes[31])
}

fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    let body = hex.strip_prefix("0x").unwrap_or(hex);
    if body.len() % 2 != 0 {
        return None;
    }
    (0..body.len() / 2)
        .map(|i| u8::from_str_radix(&body[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

impl Machine for ManageTokens {
    const LABEL: &'static str = "manage_tokens";

    fn boot_event(_cx: &App) -> Event {
        Event::Start
    }

    fn perform(operation: &MtokOperation) -> Answer<MtokShellResult> {
        match operation {
            MtokOperation::MulticallErc20Meta { chain_id, address } => {
                let (chain_id, address) = (*chain_id, address.clone());
                Answer::Blocking(Box::new(move || {
                    let symbol =
                        eth_call(chain_id, &address, SEL_SYMBOL).and_then(|r| decode_string(&r));
                    let name =
                        eth_call(chain_id, &address, SEL_NAME).and_then(|r| decode_string(&r));
                    let decimals =
                        eth_call(chain_id, &address, SEL_DECIMALS).and_then(|r| decode_u8(&r));

                    // All three, or nothing. A token with a symbol and no
                    // decimals renders an amount at the wrong magnitude, and
                    // once saved it is wrong for as long as it is in the list.
                    let meta = match (symbol, name, decimals) {
                        (Some(symbol), Some(name), Some(decimals)) => Some(MtokTokenMeta {
                            symbol,
                            name,
                            decimals,
                        }),
                        _ => None,
                    };
                    MtokShellResult::ChainMetaResolved {
                        chain_id,
                        address,
                        meta,
                    }
                }))
            }

            MtokOperation::ReadCustomTokens => Answer::Now(MtokShellResult::CustomTokensLoaded {
                tokens: read_tokens(),
            }),

            MtokOperation::WriteCustomToken { token } => {
                let mut tokens = read_tokens();
                // Replace by id rather than append: adding the same token twice
                // is a person correcting themselves, not two tokens.
                tokens.retain(|existing| existing.id != token.id);
                tokens.push(token.clone());
                Answer::Now(if write_tokens(&tokens) {
                    MtokShellResult::Saved
                } else {
                    MtokShellResult::SaveFailed
                })
            }

            MtokOperation::RemoveCustomToken { id } => {
                let id = id.clone();
                let mut tokens = read_tokens();
                let before = tokens.len();
                tokens.retain(|token| token.id != id);
                let removed = tokens.len() != before && write_tokens(&tokens);
                Answer::Now(if removed {
                    MtokShellResult::Removed { id }
                } else {
                    MtokShellResult::RemoveFailed { id }
                })
            }

            // The balance fetch reads the token list on every run, so there is
            // no separate token cache to drop on the desktop. Answered, because
            // a skipped operation leaves the core waiting.
            MtokOperation::InvalidateTokenCache => Answer::Now(MtokShellResult::CacheInvalidated),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(chain_id: u32, address: &str, symbol: &str) -> MtokCustomToken {
        MtokCustomToken {
            id: format!("{chain_id}_{address}"),
            chain_id,
            contract_address: address.to_owned(),
            symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            decimals: 6,
            network_name: "Gnosis".to_owned(),
        }
    }

    /// The stored record uses the names every other client reads.
    #[test]
    fn a_stored_token_uses_the_shared_field_names() {
        let stored = serde_json::to_value(StoredToken::from(&token(100, "0xabc", "USDC")))
            .unwrap_or_else(|_| unreachable!("must serialize"));
        let object = stored
            .as_object()
            .unwrap_or_else(|| unreachable!("an object"));
        assert_eq!(object.get("id").and_then(Value::as_str), Some("100_0xabc"));
        assert_eq!(object.get("chainId").and_then(Value::as_u64), Some(100));
        assert_eq!(
            object.get("contractAddress").and_then(Value::as_str),
            Some("0xabc")
        );
        assert_eq!(
            object.get("networkName").and_then(Value::as_str),
            Some("Gnosis")
        );
        assert!(
            !object.contains_key("contract_address"),
            "Rust spelling leaked"
        );
    }

    /// Adding the same token twice is a correction, not a duplicate.
    #[test]
    fn saving_the_same_token_twice_replaces_it() {
        storage::tests::with_temp_state("mtok-replace", || {
            let save =
                |t: MtokCustomToken| match ManageTokens::perform(&MtokOperation::WriteCustomToken {
                    token: t,
                }) {
                    Answer::Now(result) => result,
                    _ => unreachable!("local"),
                };
            assert_eq!(save(token(100, "0xabc", "USDC")), MtokShellResult::Saved);
            assert_eq!(save(token(100, "0xabc", "USDC.e")), MtokShellResult::Saved);

            let tokens = read_tokens();
            assert_eq!(tokens.len(), 1, "the same id must not appear twice");
            assert_eq!(tokens[0].symbol, "USDC.e", "the newer record wins");
        });
    }

    /// Removing something that is not there is a FAILURE — the row is still on
    /// the screen and the core has to know.
    #[test]
    fn removing_a_missing_token_reports_failure() {
        storage::tests::with_temp_state("mtok-remove", || {
            match ManageTokens::perform(&MtokOperation::RemoveCustomToken {
                id: "nope".to_owned(),
            }) {
                Answer::Now(MtokShellResult::RemoveFailed { id }) => assert_eq!(id, "nope"),
                _ => unreachable!("removing nothing must report failure"),
            }
        });
    }

    /// The ABI decoders, against real answers.
    #[test]
    fn the_erc20_decoders_read_real_return_data() {
        // `decimals()` -> 6, as a uint8 in a 32-byte word.
        assert_eq!(
            decode_u8("0x0000000000000000000000000000000000000000000000000000000000000006"),
            Some(6)
        );
        // `symbol()` -> "USDC": offset 0x20, length 4, then the bytes.
        let usdc = "0x0000000000000000000000000000000000000000000000000000000000000020\
                    0000000000000000000000000000000000000000000000000000000000000004\
                    5553444300000000000000000000000000000000000000000000000000000000";
        assert_eq!(decode_string(usdc).as_deref(), Some("USDC"));

        // Truncated, empty and non-hex answers are `None`, never a panic and
        // never a garbage symbol that gets saved forever.
        assert_eq!(decode_string("0x"), None);
        assert_eq!(decode_string("0xzz"), None);
        assert_eq!(decode_u8("0x00"), None);
        assert_eq!(
            decode_string("0x0000000000000000000000000000000000000000000000000000000000000020"),
            None,
            "an offset with no length behind it"
        );
    }

    /// Metadata is all-or-nothing, live: a real ERC-20 answers all three.
    #[test]
    #[ignore = "reads a real ERC-20 on Gnosis"]
    fn a_real_token_resolves_its_metadata() {
        // USDC on Gnosis.
        const USDC: &str = "0xDDAfbb505ad214D7b80b1f830fcCc89B60fb7A83";
        let Answer::Blocking(work) = ManageTokens::perform(&MtokOperation::MulticallErc20Meta {
            chain_id: 100,
            address: USDC.to_owned(),
        }) else {
            unreachable!("metadata is network work");
        };
        match work() {
            MtokShellResult::ChainMetaResolved { meta, .. } => {
                let meta = meta.unwrap_or_else(|| unreachable!("the token did not answer"));
                println!(
                    "  {} / {} / {} decimals",
                    meta.symbol, meta.name, meta.decimals
                );
                assert_eq!(meta.decimals, 6, "USDC has six decimals");
                assert!(!meta.symbol.is_empty());
            }
            other => unreachable!("wrong variant: {other:?}"),
        }
    }
}
