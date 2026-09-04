//! The multi-chain balance fetch — the work `BalanceOperation::FetchTokens`
//! hands over whole.
//!
//! That operation names **no chain and no URL**: the core delegates the entire
//! fetch and rules only on what comes back (which totals may be cached, when a
//! partial result may be retried, what an unreachable chain means). So this file
//! is a service, not a mapping, and it is the first of its kind on the desktop —
//! 030's three machines had a JSON file for an outside world.
//!
//! ## Scope of this cut
//!
//! **Native coins only.** Every chain's own coin, across every network the
//! wallet knows, fetched in parallel and priced at `None`.
//!
//! ERC-20 balances need Multicall3 aggregation and a token list; prices need a
//! source. Both are real work and both are additive — the core already accepts a
//! `Vec<BalanceToken>` and already knows what to do with an unpriced one, which
//! is the same `rate: None` discipline 030 established: a missing price is
//! reported as missing, never as zero and never as one.
//!
//! ## Why parallel, and why not streaming
//!
//! Twelve chains at up to a few seconds each is a minute of serial waiting, so
//! each chain gets a thread and the answers are joined. The core also supports
//! *streaming* partial results (`Event::ChainAssetsArrived`) so a home screen
//! can fill in as chains answer — that needs a way to push events into a
//! resident from a worker, which this cut does not build. Recorded as a debt:
//! settling once is correct, just less alive.

use std::thread;

use serde_json::{Value, json};

use vela_core::app::balance_dashboard::BalanceToken;
use vela_core::app::network_admin::BUILTIN_CHAINS;

use crate::executor::pool::{self, PoolError};

/// Every chain the wallet reads for. The built-ins plus whatever the person
/// added — a custom network nobody reads is a network nobody has.
fn chains() -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = BUILTIN_CHAINS
        .iter()
        .map(|chain| (chain.chain_id, chain.native_symbol.to_owned()))
        .collect();
    if let Ok(Some(Value::Array(items))) =
        crate::executor::storage::read_value(crate::executor::storage::KEY_CUSTOM_NETWORKS)
    {
        for item in items {
            let Some(chain_id) = item
                .get("chainId")
                .and_then(Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
            else {
                continue;
            };
            if out.iter().any(|(id, _)| *id == chain_id) {
                continue;
            }
            let symbol = item
                .get("nativeSymbol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            out.push((chain_id, symbol));
        }
    }
    out
}

/// One chain's native balance, or `None` if the chain could not be reached.
///
/// `None` and zero are different answers and the core treats them differently:
/// a chain that answered zero is empty, a chain that did not answer is
/// unreachable, and rendering the second as the first is how a wallet quietly
/// under-reports somebody's money.
fn native_balance(chain_id: u32, address: &str, symbol: &str) -> Option<BalanceToken> {
    let response = pool::call(chain_id, "eth_getBalance", json!([address, "latest"]));
    let body = match response {
        Ok(body) => body,
        Err(PoolError::Failed { .. } | PoolError::RangeCap { .. } | PoolError::Unavailable) => {
            return None;
        }
    };
    let hex = body.get("result").and_then(Value::as_str)?;
    let wei = u128::from_str_radix(hex.trim_start_matches("0x"), 16).ok()?;

    Some(BalanceToken {
        chain_id,
        symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        // The core takes the raw integer as a string and owns every decimal
        // decision. A shell that divided here would be choosing a precision
        // nobody asked it to choose.
        balance: wei.to_string(),
        decimals: 18,
        token_address: None,
        // No price source in this cut. `None`, never 0 and never 1 — the same
        // rule `display_currency` made explicit in 030.
        price_usd: None,
        spam: false,
    })
}

/// Every chain's native balance for one address, fetched in parallel.
///
/// Returns the tokens found and the chains that could not be reached, which the
/// core needs separately: the second list is what lets the home say "this chain
/// is unreachable" instead of adding a silent zero to the total.
#[must_use]
pub fn fetch_native(address: &str) -> (Vec<BalanceToken>, Vec<u32>) {
    let mut handles = Vec::new();
    for (chain_id, symbol) in chains() {
        let address = address.to_owned();
        handles.push(
            thread::Builder::new()
                .name(format!("vela-balance-{chain_id}"))
                .spawn(move || (chain_id, native_balance(chain_id, &address, &symbol)))
                .ok(),
        );
    }

    let mut tokens = Vec::new();
    let mut failed = Vec::new();
    for handle in handles.into_iter().flatten() {
        match handle.join() {
            Ok((_, Some(token))) => tokens.push(token),
            Ok((chain_id, None)) => failed.push(chain_id),
            // A panicked worker is a chain we did not read. It is not a reason
            // to lose the eleven that answered.
            Err(_) => {}
        }
    }
    // Deterministic order: the core sorts for display, but a stable input makes
    // a test's failure readable.
    tokens.sort_by_key(|token| token.chain_id);
    failed.sort_unstable();
    (tokens, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in chain is read, and a custom network joins them.
    #[test]
    fn the_fetch_covers_builtins_and_custom_networks() {
        crate::executor::storage::tests::with_temp_state("balances-chains", || {
            assert_eq!(chains().len(), BUILTIN_CHAINS.len());

            let custom = json!([{ "chainId": 7_777_777, "nativeSymbol": "ETH" }]);
            if crate::executor::storage::write_value(
                crate::executor::storage::KEY_CUSTOM_NETWORKS,
                custom,
            )
            .is_err()
            {
                unreachable!("could not seed");
            }
            let all = chains();
            assert_eq!(all.len(), BUILTIN_CHAINS.len() + 1);
            assert!(all.iter().any(|(id, _)| *id == 7_777_777));

            // A custom network duplicating a built-in must not double-read it.
            let dup = json!([{ "chainId": 100, "nativeSymbol": "xDAI" }]);
            if crate::executor::storage::write_value(
                crate::executor::storage::KEY_CUSTOM_NETWORKS,
                dup,
            )
            .is_err()
            {
                unreachable!("could not seed");
            }
            assert_eq!(chains().len(), BUILTIN_CHAINS.len());
        });
    }

    /// The golden Safe, across every chain, through the pool.
    ///
    /// The assertion that matters is not the total: it is that Gnosis reports
    /// the known figure AND that chains which fail come back in `failed` rather
    /// than as a zero-balance token. A wallet that renders an unreachable chain
    /// as empty under-reports somebody's money and looks completely normal
    /// doing it.
    #[test]
    #[ignore = "reads every chain for a real address"]
    fn the_golden_safe_reads_across_chains() {
        crate::executor::storage::tests::with_temp_state("balances-live", || {
            const GOLDEN: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";
            let (tokens, failed) = fetch_native(GOLDEN);

            println!(
                "  {} chains answered, {} did not",
                tokens.len(),
                failed.len()
            );
            for token in &tokens {
                if token.balance != "0" {
                    println!(
                        "    chain {} : {} {}",
                        token.chain_id, token.balance, token.symbol
                    );
                }
            }
            if !failed.is_empty() {
                println!("    unreachable: {failed:?}");
            }

            let gnosis = tokens
                .iter()
                .find(|t| t.chain_id == 100)
                .unwrap_or_else(|| unreachable!("Gnosis did not answer: failed={failed:?}"));
            assert_eq!(
                gnosis.balance, "769970000000000000",
                "the golden Safe's known Gnosis balance"
            );
            assert_eq!(gnosis.price_usd, None, "no price source in this cut");

            // The two lists are disjoint by construction, and that is the
            // property the home screen depends on.
            for chain_id in &failed {
                assert!(
                    !tokens.iter().any(|t| t.chain_id == *chain_id),
                    "chain {chain_id} is both answered and unreachable"
                );
            }
        });
    }
}
