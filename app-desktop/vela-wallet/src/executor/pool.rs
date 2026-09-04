//! The RPC pool — one routing authority for the whole process.
//!
//! ## Why this is a thread and not a gpui resident
//!
//! Every other machine in this client lives in [`crate::resident`], driven from
//! the main thread and rendered by a screen. The pool has neither property. Its
//! callers are background workers doing blocking HTTP — the balance fetch, the
//! activity read, a recipient probe — and they need an answer *on the thread
//! they are already on*. Routing them through the main thread would put a
//! multi-second network round trip in front of the next frame, which is the one
//! thing the resident host was built to avoid.
//!
//! So the pool owns a thread. Callers send a request and block on a reply; the
//! thread runs `CoreHost<RpcPool>` and performs the pool's own effects inline,
//! where blocking is free.
//!
//! ## One session, and why that is not a detail
//!
//! The ban map, the per-endpoint statistics and the fastest-RPC race winners are
//! **facts about the network that every caller shares**. Two sessions means an
//! endpoint banned for the balance fetch and retried by the activity read a
//! second later, and a ban map that disagrees with itself — which is the bug the
//! web port names in its own header. Hence one `OnceLock`, and no way to make a
//! second.
//!
//! ## What lives here and what does not
//!
//! This file owns exactly two things the core cannot: **the fetch**, and **the
//! reply channel the caller is waiting on**. Which endpoint to try, in what
//! order, after which failure, under which ban, for how long — six-tier source
//! scoring, EMA latency, cooldowns, temp and permanent bans, the four-way error
//! classification, the three-pass sweep, the all-banned self-rescue — is
//! `rpc_pool.rs`'s 1,975 lines and is not re-derived here. If this file grows an
//! `if` that decides where a call goes next, it is in the wrong file.

use std::collections::HashMap;
use std::io::Read as _;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use vela_core::app::network_admin::{
    BUILTIN_CHAINS, NetProviderId, PROVIDER_ORDER, build_provider_rpc_url,
};
use vela_core::app::rpc_pool::{
    Event, RpcBanEntry, RpcCallVerdict, RpcEndpointSeed, RpcKind, RpcOperation, RpcShellResult,
    RpcSource, RpcTransportOutcome,
};

use crate::core_host::CoreHost;
use crate::executor::{proxy, storage};

/// Curated public fallbacks (`PUBLIC_RPCS`, rpc-pool-endpoints.ts:50-60).
///
/// A data table, ported verbatim. It is tier 4 of six — below a user override, a
/// configured provider and the built-in default, above whatever the chain index
/// happens to list.
const PUBLIC_RPCS: &[(u32, &[&str])] = &[
    (
        1,
        &["https://ethereum-rpc.publicnode.com", "https://1rpc.io/eth"],
    ),
    (
        56,
        &[
            "https://bsc-rpc.publicnode.com",
            "https://bsc.drpc.org",
            "https://bsc.meowrpc.com",
        ],
    ),
    (
        137,
        &[
            "https://polygon-bor-rpc.publicnode.com",
            "https://1rpc.io/matic",
        ],
    ),
    (
        42161,
        &[
            "https://arbitrum-one-rpc.publicnode.com",
            "https://1rpc.io/arb",
        ],
    ),
    (
        10,
        &["https://optimism-rpc.publicnode.com", "https://1rpc.io/op"],
    ),
    (
        8453,
        &["https://base-rpc.publicnode.com", "https://1rpc.io/base"],
    ),
    (
        43114,
        &[
            "https://avalanche-c-chain-rpc.publicnode.com",
            "https://1rpc.io/avax/c",
        ],
    ),
    (
        100,
        &[
            "https://gnosis-rpc.publicnode.com",
            "https://1rpc.io/gnosis",
        ],
    ),
    (196, &["https://rpc.xlayer.tech", "https://xlayer.drpc.org"]),
];

/// What a caller asked for and where to send the answer.
enum Request {
    Call {
        chain_id: u32,
        kind: RpcKind,
        method: String,
        params: Value,
        reply: Sender<Result<Value, PoolError>>,
    },
    /// Drop every endpoint's state for a chain, or for all of them.
    Refresh { chain_id: Option<u32> },
}

/// Why a routed call produced no answer.
#[derive(Debug, Clone, PartialEq)]
pub enum PoolError {
    /// Every endpoint failed every pass. `rate_limited` is the self-healing
    /// transient case (invariant ④) and is worth showing differently.
    Failed { rate_limited: bool },
    /// `eth_getLogs` hit a range cap; the caller splits and retries.
    RangeCap { max_span: f64 },
    /// The pool thread is gone. Only reachable if it panicked.
    Unavailable,
}

static POOL: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();

fn sender() -> &'static Mutex<Sender<Request>> {
    POOL.get_or_init(|| {
        let (tx, rx) = channel::<Request>();
        std::thread::Builder::new()
            .name("vela-rpc-pool".to_owned())
            .spawn(move || run(&rx))
            .ok();
        Mutex::new(tx)
    })
}

/// One routed JSON-RPC call. **Blocks** — call it from a worker, never a frame.
#[allow(
    dead_code,
    reason = "the read machines' entry point; wired by 031's next phase, and exercised by this module's live test"
)]
pub fn call(chain_id: u32, method: &str, params: Value) -> Result<Value, PoolError> {
    dispatch(chain_id, RpcKind::Rpc, method, params)
}

/// The same, against the chain's bundler rather than its RPC.
#[allow(dead_code, reason = "the money path's entry point, wired by spec 032")]
pub fn bundler_call(chain_id: u32, method: &str, params: Value) -> Result<Value, PoolError> {
    dispatch(chain_id, RpcKind::Bundler, method, params)
}

/// Forget an endpoint's measured state — after the settings screen edits it.
#[allow(dead_code, reason = "wired to network_admin's invalidate_pools next")]
pub fn refresh(chain_id: Option<u32>) {
    if let Ok(tx) = sender().lock() {
        let _ = tx.send(Request::Refresh { chain_id });
    }
}

fn dispatch(chain_id: u32, kind: RpcKind, method: &str, params: Value) -> Result<Value, PoolError> {
    let (reply, answer) = channel();
    {
        let Ok(tx) = sender().lock() else {
            return Err(PoolError::Unavailable);
        };
        if tx
            .send(Request::Call {
                chain_id,
                kind,
                method: method.to_owned(),
                params,
                reply,
            })
            .is_err()
        {
            return Err(PoolError::Unavailable);
        }
    }
    answer.recv().unwrap_or(Err(PoolError::Unavailable))
}

// ---------------------------------------------------------------------------
// The thread
// ---------------------------------------------------------------------------

/// What the shell is holding for one in-flight call.
struct InFlight {
    params: Value,
    reply: Sender<Result<Value, PoolError>>,
    /// The last body received, per URL. `Conclude { Respond { url } }` names
    /// which one the core accepted — the core never sees a body itself.
    bodies: HashMap<String, Value>,
}

fn run(rx: &std::sync::mpsc::Receiver<Request>) {
    let mut host = CoreHost::<RpcPoolApp>::new();
    let mut inflight: HashMap<String, InFlight> = HashMap::new();
    let mut next_id: u64 = 0;

    // Bans persist across launches; a pool that forgot them would re-try an
    // endpoint the last session already proved dead.
    let entries = read_bans();
    let pending = host.dispatch(Event::BansLoaded { entries });
    drain(&mut host, pending, &mut inflight);

    while let Ok(request) = rx.recv() {
        match request {
            Request::Call {
                chain_id,
                kind,
                method,
                params,
                reply,
            } => {
                next_id += 1;
                let call_id = format!("c{next_id}");
                inflight.insert(
                    call_id.clone(),
                    InFlight {
                        params,
                        reply,
                        bodies: HashMap::new(),
                    },
                );
                let pending = host.dispatch(Event::CallRequested {
                    call_id,
                    chain_id,
                    kind,
                    method,
                    now_ms: now_ms(),
                });
                drain(&mut host, pending, &mut inflight);
            }
            Request::Refresh { chain_id } => {
                let event = match chain_id {
                    Some(chain_id) => Event::RefreshChain { chain_id },
                    None => Event::InvalidateAll,
                };
                let pending = host.dispatch(event);
                drain(&mut host, pending, &mut inflight);
            }
        }
    }
}

type RpcPoolApp = vela_core::app::rpc_pool::RpcPool;

fn drain(
    host: &mut CoreHost<RpcPoolApp>,
    mut pending: Vec<crate::core_host::Pending<RpcOperation>>,
    inflight: &mut HashMap<String, InFlight>,
) {
    while let Some(next) = pending.pop() {
        let result = perform(&next.operation, inflight);
        pending.extend(host.resolve(next.id, result));
    }
}

fn perform(operation: &RpcOperation, inflight: &mut HashMap<String, InFlight>) -> RpcShellResult {
    match operation {
        RpcOperation::LoadPoolConfig { chain_id } => {
            let (rpc_endpoints, bundler_endpoints) = collect_endpoints(*chain_id);
            RpcShellResult::PoolConfig {
                chain_id: *chain_id,
                rpc_endpoints,
                bundler_endpoints,
                now_ms: now_ms(),
            }
        }

        RpcOperation::JsonRpcPost {
            call_id,
            url,
            method,
            x_rpc_url,
            timeout_ms,
        } => {
            let params = inflight
                .get(call_id)
                .map_or(Value::Array(Vec::new()), |call| call.params.clone());
            let started = Instant::now();
            let (outcome, body) = post(url, method, &params, x_rpc_url.as_deref(), *timeout_ms);
            if let (Some(body), Some(call)) = (body, inflight.get_mut(call_id)) {
                // Held for `Conclude`. The core classifies from the `error`
                // member alone and never sees a body — which is what keeps a
                // 3 MB `eth_getLogs` answer out of its state.
                call.bodies.insert(url.clone(), body);
            }
            RpcShellResult::PostOutcome {
                call_id: call_id.clone(),
                url: url.clone(),
                outcome,
                latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                now_ms: now_ms(),
            }
        }

        RpcOperation::ProbeChainId {
            chain_id,
            url,
            timeout_ms,
        } => {
            let started = Instant::now();
            let (_, body) = post(url, "eth_chainId", &json!([]), None, *timeout_ms);
            let reported = body
                .as_ref()
                .and_then(|value| value.get("result"))
                .and_then(Value::as_str)
                .and_then(|hex| u32::from_str_radix(hex.trim_start_matches("0x"), 16).ok());
            RpcShellResult::ChainIdProbed {
                chain_id: *chain_id,
                url: url.clone(),
                reported,
                latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                now_ms: now_ms(),
            }
        }

        // All randomness is injected. The core is a pure function of its inputs,
        // which is why its jitter is a question rather than a `rand::random`.
        RpcOperation::DrawJitter { call_id } => RpcShellResult::Jitter {
            call_id: call_id.clone(),
            value: unit_random(),
        },

        RpcOperation::StartBackoff { call_id, delay_ms } => {
            std::thread::sleep(Duration::from_millis(u64::from(*delay_ms)));
            RpcShellResult::BackoffElapsed {
                call_id: call_id.clone(),
                now_ms: now_ms(),
            }
        }

        RpcOperation::PersistBans { entries } => {
            write_bans(entries);
            RpcShellResult::Persisted
        }

        RpcOperation::Conclude { call_id, verdict } => {
            if let Some(call) = inflight.remove(call_id) {
                let answer = match verdict {
                    RpcCallVerdict::Respond { url } => {
                        call.bodies.get(url).cloned().ok_or(PoolError::Failed {
                            rate_limited: false,
                        })
                    }
                    RpcCallVerdict::RangeCap { max_span, .. } => Err(PoolError::RangeCap {
                        max_span: *max_span,
                    }),
                    RpcCallVerdict::Failed { rate_limited } => Err(PoolError::Failed {
                        rate_limited: *rate_limited,
                    }),
                    // Not answers to a routed call; a caller waiting on one of
                    // these asked the wrong question.
                    RpcCallVerdict::BundlerBase { .. } | RpcCallVerdict::BestRpcUrl { .. } => {
                        Err(PoolError::Failed {
                            rate_limited: false,
                        })
                    }
                };
                // A caller that gave up is not an error: the receiver is simply
                // gone, and the pool has nothing to be sad about.
                let _ = call.reply.send(answer);
            }
            RpcShellResult::Concluded
        }
    }
}

// ---------------------------------------------------------------------------
// The two things the core cannot have
// ---------------------------------------------------------------------------

/// One POST, classified into the five outcomes the core distinguishes.
///
/// The body comes back separately: the core is told only whether there was an
/// `error` member, because classification is its job and payload size is not.
fn post(
    url: &str,
    method: &str,
    params: &Value,
    x_rpc_url: Option<&str>,
    timeout_ms: u32,
) -> (RpcTransportOutcome, Option<Value>) {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let agent = proxy::agent(Duration::from_millis(u64::from(timeout_ms)));
    let mut request = agent.post(url).header("content-type", "application/json");
    // Invariant ②: a bundler call carries the same-chain RPC the pool verified.
    if let Some(rpc) = x_rpc_url {
        request = request.header("X-Rpc-Url", rpc);
    }

    let mut response = match request.send_json(&payload) {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(status)) => {
            return (RpcTransportOutcome::HttpError { status }, None);
        }
        Err(ureq::Error::Timeout(_)) => return (RpcTransportOutcome::Timeout, None),
        Err(_) => return (RpcTransportOutcome::Network, None),
    };

    let mut text = String::new();
    if response
        .body_mut()
        .as_reader()
        .read_to_string(&mut text)
        .is_err()
    {
        return (RpcTransportOutcome::Network, None);
    }
    let Ok(body) = serde_json::from_str::<Value>(&text) else {
        return (RpcTransportOutcome::NonJson, None);
    };
    if !body.is_object() {
        return (RpcTransportOutcome::NonJson, None);
    }

    let error = body.get("error").and_then(|error| {
        Some(vela_core::app::rpc_pool::RpcErrorInfo {
            code: error
                .get("code")
                .and_then(Value::as_i64)
                .and_then(|code| i32::try_from(code).ok()),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    });
    (RpcTransportOutcome::Response { error }, Some(body))
}

/// The six tiers, in order (`collectRpcUrls`, rpc-pool-endpoints.ts:68-127).
///
/// Bans are NOT filtered here — they are the core's state, and it says so:
/// "Do NOT filter banned URLs — bans are this core's state."
fn collect_endpoints(chain_id: u32) -> (Vec<RpcEndpointSeed>, Vec<RpcEndpointSeed>) {
    let mut rpc = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |url: String, source: RpcSource, into: &mut Vec<RpcEndpointSeed>| {
        if url.is_empty() || !seen.insert(url.clone()) {
            return;
        }
        into.push(RpcEndpointSeed { url, source });
    };

    let builtin = BUILTIN_CHAINS.iter().find(|c| c.chain_id == chain_id);

    // 1. a per-network override, but only when it differs from the default.
    if let Some(config) = stored_network_config(chain_id)
        && !config.is_empty()
        && builtin.is_none_or(|c| c.rpc_url != config)
    {
        add(config, RpcSource::User, &mut rpc);
    }

    // 2. configured providers, in the canonical order.
    let keys = stored_provider_keys();
    for id in PROVIDER_ORDER {
        if let Some((_, key)) = keys.iter().find(|(provider, _)| *provider == id)
            && let Some(url) = build_provider_rpc_url(id, chain_id, key)
        {
            add(url, RpcSource::Provider, &mut rpc);
        }
    }

    // 3. the built-in default, then a custom network's own endpoint.
    if let Some(chain) = builtin {
        add(chain.rpc_url.to_owned(), RpcSource::Default, &mut rpc);
    }
    if let Some(custom) = stored_custom_rpc(chain_id) {
        add(custom, RpcSource::Default, &mut rpc);
    }

    // 4. the curated public fallbacks.
    if let Some((_, urls)) = PUBLIC_RPCS.iter().find(|(id, _)| *id == chain_id) {
        for url in *urls {
            add((*url).to_owned(), RpcSource::Public, &mut rpc);
        }
    }

    // Tiers 5 and 6 are the chain index, which this cut does not fetch here:
    // `LoadPoolConfig` is answered synchronously on the pool thread and an
    // index round trip would stall every first call on a chain behind it. The
    // index tiers are a recorded debt, not an oversight — the core orders
    // whatever it is given, so adding them later changes no rule.

    // The bundler is one URL per chain, from the relay, plus a custom override.
    let mut bundler = Vec::new();
    if let Some(url) = stored_custom_bundler(chain_id) {
        add(url, RpcSource::User, &mut bundler);
    }
    add(
        format!("https://vela-relay.getvela.app/{chain_id}"),
        RpcSource::Default,
        &mut bundler,
    );

    (rpc, bundler)
}

fn now_ms() -> f64 {
    crate::executor::now_ms()
}

/// A uniform draw in [0,1). The pool's jitter, injected rather than generated
/// in the core.
fn unit_random() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(nanos) / f64::from(1_000_000_000u32)
}

fn read_bans() -> Vec<RpcBanEntry> {
    let Ok(Some(Value::Array(items))) = storage::read_value(storage::KEY_RPC_BANNED) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<RpcBanEntry>(item).ok())
        .collect()
}

fn write_bans(entries: &[RpcBanEntry]) {
    let encoded = serde_json::to_value(entries).unwrap_or(Value::Null);
    let _ = storage::write_value(storage::KEY_RPC_BANNED, encoded);
}

fn stored_network_config(chain_id: u32) -> Option<String> {
    stored_array(storage::KEY_NETWORK_CONFIG, chain_id, "rpcURL")
}

fn stored_custom_rpc(chain_id: u32) -> Option<String> {
    stored_array(storage::KEY_CUSTOM_NETWORKS, chain_id, "rpcURL")
}

fn stored_custom_bundler(chain_id: u32) -> Option<String> {
    stored_array(storage::KEY_CUSTOM_NETWORKS, chain_id, "bundlerURL")
}

fn stored_array(key: &str, chain_id: u32, field: &str) -> Option<String> {
    let Ok(Some(Value::Array(items))) = storage::read_value(key) else {
        return None;
    };
    items.into_iter().find_map(|item| {
        (item.get("chainId").and_then(Value::as_u64) == Some(u64::from(chain_id)))
            .then(|| item.get(field).and_then(Value::as_str).map(str::to_owned))
            .flatten()
            .filter(|url| !url.is_empty())
    })
}

fn stored_provider_keys() -> Vec<(NetProviderId, String)> {
    let mut out = Vec::new();
    let Ok(Some(Value::Object(map))) = storage::read_value(storage::KEY_RPC_PROVIDERS) else {
        return out;
    };
    for (id, field) in [
        (NetProviderId::Alchemy, "alchemy"),
        (NetProviderId::Drpc, "drpc"),
        (NetProviderId::Ankr, "ankr"),
    ] {
        if let Some(key) = map.get(field).and_then(Value::as_str)
            && !key.is_empty()
        {
            out.push((id, key.to_owned()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six tiers, in order, with bans deliberately NOT filtered.
    #[test]
    fn endpoints_are_collected_in_source_order() {
        storage::tests::with_temp_state("pool-tiers", || {
            let (rpc, bundler) = collect_endpoints(100);
            let sources: Vec<RpcSource> = rpc.iter().map(|e| e.source).collect();

            // With no override and no provider key, the first tier present is
            // the built-in default, then the curated public list.
            assert_eq!(sources.first(), Some(&RpcSource::Default));
            assert!(
                sources.iter().any(|s| *s == RpcSource::Public),
                "the curated fallbacks must be offered: {sources:?}"
            );
            assert!(
                rpc.iter().any(|e| e.url == "https://rpc.gnosischain.com"),
                "Gnosis's built-in endpoint must be in the pool"
            );

            // Deduped by URL: the built-in and the public list overlap.
            let mut urls: Vec<&str> = rpc.iter().map(|e| e.url.as_str()).collect();
            let count = urls.len();
            urls.sort_unstable();
            urls.dedup();
            assert_eq!(urls.len(), count, "an endpoint was offered twice");

            assert!(
                bundler.iter().any(|e| e.url.ends_with("/100")),
                "the relay's chain base must be the bundler endpoint"
            );
        });
    }

    /// A configured override outranks everything, and only when it DIFFERS from
    /// the default — "override" and "the same value" are not the same fact, and
    /// the tier is what the core scores on.
    #[test]
    fn an_override_equal_to_the_default_is_not_a_user_tier() {
        storage::tests::with_temp_state("pool-override", || {
            let builtin = BUILTIN_CHAINS
                .iter()
                .find(|c| c.chain_id == 100)
                .unwrap_or_else(|| unreachable!("Gnosis is built in"));

            let same = json!([{ "chainId": 100, "rpcURL": builtin.rpc_url }]);
            if storage::write_value(storage::KEY_NETWORK_CONFIG, same).is_err() {
                unreachable!("could not seed");
            }
            let (rpc, _) = collect_endpoints(100);
            assert!(
                !rpc.iter().any(|e| e.source == RpcSource::User),
                "an override identical to the default is not an override"
            );

            let different = json!([{ "chainId": 100, "rpcURL": "https://my.node.example" }]);
            if storage::write_value(storage::KEY_NETWORK_CONFIG, different).is_err() {
                unreachable!("could not seed");
            }
            let (rpc, _) = collect_endpoints(100);
            assert_eq!(rpc.first().map(|e| e.source), Some(RpcSource::User));
            assert_eq!(
                rpc.first().map(|e| e.url.as_str()),
                Some("https://my.node.example")
            );
        });
    }

    /// Bans round-trip through the same key every other client uses.
    #[test]
    fn bans_persist_under_the_shared_key() {
        storage::tests::with_temp_state("pool-bans", || {
            assert!(read_bans().is_empty());
            write_bans(&[RpcBanEntry {
                url: "https://dead.example".to_owned(),
                banned_at_ms: 1_756_000_000_000.0,
                permanent: true,
            }]);
            let raw = storage::read_value(storage::KEY_RPC_BANNED)
                .ok()
                .flatten()
                .unwrap_or_else(|| unreachable!("nothing written"));
            assert!(raw.is_array(), "the ban map is a list of entries");
            let back = read_bans();
            assert_eq!(back.len(), 1);
            assert!(back[0].permanent);
        });
    }

    /// The pool, against the real network, through the real routing.
    ///
    /// This is SC-002's evidence and it is deliberately a READ of the golden
    /// Safe: the balance it returns is checkable by hand, and chain 100's
    /// built-in endpoint is the one that answers this client with 403 — so the
    /// pool has to fail over to reach it. A single-endpoint client cannot.
    #[test]
    #[ignore = "hits the real Gnosis pool"]
    fn the_pool_reads_the_golden_safe_by_failing_over() {
        storage::tests::with_temp_state("pool-live", || {
            const GOLDEN: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";

            let balance = call(100, "eth_getBalance", json!([GOLDEN, "latest"]))
                .unwrap_or_else(|error| unreachable!("the pool could not read: {error:?}"));
            let hex = balance
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_else(|| unreachable!("no result member: {balance}"));
            let wei = u128::from_str_radix(hex.trim_start_matches("0x"), 16)
                .unwrap_or_else(|_| unreachable!("not a quantity: {hex}"));
            #[allow(clippy::cast_precision_loss, reason = "display only")]
            let xdai = wei as f64 / 1e18;
            println!("  golden Safe: {xdai} xDAI via the pool");
            assert!(
                wei > 0,
                "the golden Safe is funded; a zero means we read nothing"
            );

            // A second call proves the session is shared: whatever the first
            // call learned — which endpoint answered, which one is banned — is
            // still known, and the answer agrees.
            let again = call(100, "eth_getBalance", json!([GOLDEN, "latest"]))
                .unwrap_or_else(|error| unreachable!("second read failed: {error:?}"));
            assert_eq!(
                again.get("result"),
                balance.get("result"),
                "two reads of one address disagreed"
            );
            println!("  second read agreed — one session, shared state");
        });
    }
}
