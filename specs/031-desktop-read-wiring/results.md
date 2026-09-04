# Results — 031 desktop-read-wiring

## Baselines — recorded 2026-09-04, branch point `6324ba39` (tip of `030-desktop-live-shell`)

Stacked on 030 for the same reason 030 stacked on 029: both cuts edit
`wallet/page.rs` heavily, and 031's gates are the ones 029 added.

### Desktop at the branch point

| | |
|---|---|
| `cargo test` | **125 passed · 0 failed · 8 ignored** |
| `src/**/*.rs` | 61 files |
| `wallet/page.rs` | 4,255 lines ⚠ |
| warnings (forced rebuild) | 1 (`BLE_CHANNEL_SUPPORTED`, pre-existing) |
| gallery states | 36 |

### The seven machines

| Machine | Core lines | Operations |
|---|---|---|
| `rpc_pool` | 1,975 | 7 |
| `token_trust` | 1,937 | 6 |
| `activity_feed` | 1,117 | 6 |
| `balance_dashboard` | 1,096 | 7 |
| `payment_request` | 687 | 2 |
| `manage_tokens` | 657 | 4 |
| `receive_watch` | 377 | 3 |
| **total** | **7,846** | **35** |

### The service layer this cut must port — and the part it must not

`BalanceOperation::FetchTokens { address, force, pull }` names **no chain and no
URL**. The core delegates the whole multi-chain fetch to the shell and rules only on
what comes back, so 031 ports a service layer as well as writing executors. That is a
shape 030 did not have, and it is why this cut is materially larger.

| Web source (at `f9bcb278`) | Lines |
|---|---|
| `services/wallet-api.ts` | 788 |
| `services/abi.ts` | 342 |
| `services/rpc-pool.ts` | 334 |
| `services/chains.ts` | 231 |
| `services/recipient-identity.ts` | 223 |
| `services/rpc-pool-endpoints.ts` | 222 |
| `services/activity.ts` | 203 |
| `services/token-metadata.ts` | 159 |
| `services/price-service.ts` | 130 |
| `services/native-price.ts` | 100 |
| `services/balance-cache.ts`, `endpoint-admission.ts`, `incoming-transfers.ts`, `tokens.ts`, `currency-rate.ts`, `fiat-rate-quote.ts` | 343 |
| **total** | **~3,075** |

**Already in Rust, and not to be ported:** calldata decoding and selector maths
(`vela-core/abi.rs`, 539), hex/quantity codecs and keccak (`primitives.rs`, 202), Safe
derivation (`safe.rs`, 760) — and every routing *rule*, which is `rpc_pool.rs`'s 1,975
lines. The shell must not reimplement six-tier scoring, EMA latency, cooldowns, bans,
error classification, the three-pass sweep or the all-banned self-rescue.

**Genuinely missing and needed:** the ABI *encoders* web hand-rolled — `encAggregate3`
/ `decAggregate3` (Multicall3), `encBalanceOf`, `encDecimals`, `encGetEthBalance`.
`alloy-dyn-abi` is already a `vela-core` dependency, so these are assembly rather than
new machinery.

### Owed from 030 (FR-007's handoff contract)

| Arm | Answers today | Must become |
|---|---|---|
| `contacts::resolve_identity` | `identity: None` | the identity waterfall |
| `contacts::classify_recipient` | `code: None` | `eth_getCode` via the pool |
| `display_currency::resolve_rate` | `rate: None` | the rate source chain |
| `display_currency::read_device_currency` | `None` | the region's ISO-4217 |
| `network_admin::invalidate_pools` | acknowledged no-op | a real pool invalidation |

### A debt inherited from 030 that shapes this cut

`rpc.gnosischain.com` refuses this HTTP client with **403** while curl gets 200 —
measured through a bare `ureq::Agent` and through `proxy::agent`, against all three
Gnosis endpoints. It is `registry.rs:362`'s **first** endpoint and it is Gnosis's
default in the chains table. The pool's scoring and ban rules will meet it
immediately, which makes it a useful first real test of exactly the behaviour this
cut is wiring — and a reason not to assume any single endpoint answers.
