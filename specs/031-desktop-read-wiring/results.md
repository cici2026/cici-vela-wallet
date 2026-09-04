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

## Phase 1 — the routing authority

`src/executor/pool.rs`: the `rpc_pool` machine on its own thread, with a blocking
call API, six-tier endpoint collection, and bans that persist.

| Gate | Result |
|---|---|
| `cargo test` | ✅ **128 passed · 0 failed · 9 ignored** (031 opened at 125) |
| `cargo fmt --all --check` | ✅ clean |
| warnings (forced) | ✅ 1, pre-existing |
| live read through the pool | ✅ see below |

### Why the pool is a thread, not a resident

Every other machine lives in `resident.rs`, driven from the main thread and rendered
by a screen. The pool has neither property: its callers are **background workers doing
blocking HTTP** — the balance fetch, the activity read, a recipient probe — and they
need an answer on the thread they are already on. Routing them through the main thread
would put a multi-second round trip in front of the next frame, which is precisely
what the resident host exists to avoid.

So the pool owns a thread, and callers block on a reply channel. One `OnceLock`, and
no way to make a second session — the ban map, per-endpoint statistics and race
winners are facts about the network **every** caller shares. Two sessions means an
endpoint banned by the balance fetch and retried by the activity read a second later.

### The live read, and why it is the right test

```
golden Safe: 0.76997 xDAI via the pool
second read agreed — one session, shared state
```

That figure matches an independent `eth_getBalance` taken by hand. And chain 100's
**built-in default endpoint is `rpc.gnosischain.com`** — the one that answers this
client with 403. So the pool could only produce that number by scoring it, failing
over and reaching a different endpoint. A single-endpoint client cannot read this
balance at all, which makes the inherited 403 debt the most useful possible first test
of exactly what this phase wires.

### What this file owns, and what it must never

Two things the core cannot have: **the fetch**, and **the reply channel the caller is
waiting on**. Everything about *where a call goes next* — six-tier source scoring, EMA
latency, cooldowns, temp and permanent bans, four-way error classification, the
three-pass sweep, the all-banned self-rescue — is `rpc_pool.rs`'s 1,975 lines. If this
file grows an `if` that decides where a call goes, it is in the wrong file.

Two details the core's comments insisted on and this file obeys:
- **Bans are not filtered during collection.** "Do NOT filter banned URLs — bans are
  this core's state." The shell offers every endpoint; the core decides which is dead.
- **The body never enters the core.** A `PostOutcome` reports only whether there was
  an `error` member; the shell holds the JSON per `(call_id, url)` and hands over the
  one the verdict names. A 3 MB `eth_getLogs` answer stays out of the machine's state.

### A tier deliberately not implemented, recorded rather than forgotten

Tiers 5 and 6 are the chain index's endpoint list. `LoadPoolConfig` is answered
synchronously on the pool thread, and an index round trip there would stall every
first call on a chain behind an HTTP fetch. The core orders whatever it is given, so
adding those tiers later changes no rule — it is a debt, not a divergence.

## Phase 2 — the wallet reads its own money

`executor/balances.rs` (the multi-chain fetch the core delegates whole) and
`executor/balance_dashboard.rs` (seven operations, the 24h total cache, the privacy
flag).

| Gate | Result |
|---|---|
| `cargo test` | ✅ **133 passed · 0 failed · 10 ignored** (031 opened at 125) |
| `cargo fmt --all --check` | ✅ clean |
| warnings (forced) | ✅ 1, pre-existing |
| `scripts/sweep-gallery.sh` | ✅ every state rendered |

### The live read, across every chain

```
11 chains answered, 1 did not
  chain 100 : 769970000000000000 xDAI
  unreachable: [4217]
```

`769970000000000000` wei is **0.76997 xDAI** — the golden Safe's known balance, now
reached through the pool's routing rather than a hand-written endpoint list.

**The second line is the one that matters.** Tempo comes back in `failed_chain_ids`,
not as a zero-balance token, and a test asserts the two lists are disjoint. A wallet
that renders an unreachable chain as empty **under-reports somebody's money and looks
completely normal doing it** — which is SC-003, and the reason the core takes the two
lists separately rather than a single token array.

### Scope of this cut, stated rather than implied

**Native coins only.** ERC-20 needs Multicall3 aggregation and a token list; prices
need a source. Both are additive: the core already accepts a `Vec<BalanceToken>` and
already knows what an unpriced one means. `price_usd` is `None`, never `0` and never
`1` — the same discipline `display_currency` made explicit in 030.

Two more deliberate gaps, marked in the code rather than left to be discovered:
- **`force` is accepted and ignored**, because this cut keeps no 5-minute shell TTL —
  every fetch is live. Adding the TTL later changes no core rule, since the core
  already says when it wants one bypassed.
- **No streaming.** The core supports `ChainAssetsArrived` so a home fills in as
  chains answer; that needs a way to push events into a resident from a worker, which
  this cut does not build. Twelve chains run in parallel and settle once — correct,
  just less alive.

### The split the cache respects

The core's words: *"The shell applies the 24h TTL"* and *"The CORE decides when this
may happen — the complete-results-only write gate."* Both halves are obeyed exactly.
Expiry reads as **absent**, not as a stale figure — the hero would rather show a
skeleton than yesterday's number presented as today's — and this file never writes
uninvited, because caching a total assembled from a partial fetch is how a wallet
remembers a number that was never true.

### A red that was my test, not the code

`only_unexpired_rows_reach_the_switcher` failed with 0 rows where 1 was expected. The
cause was the test seeding a fixed past timestamp while the operation reads the
**real** clock — so both rows were legitimately expired and the executor was right.
Fixed by seeding from the same clock. Worth recording because the failure looked
exactly like a broken TTL.

## Phase 3 — the hero shows the person's own money

`wallet/live.rs` and the hero bound to `BalanceDashboard`.

| Gate | Result |
|---|---|
| `cargo test` | ✅ **138 passed · 0 failed · 10 ignored** (031 opened at 125) |
| `cargo fmt --all --check` | ✅ clean |
| warnings (forced) | ✅ 1, pre-existing |
| `scripts/sweep-gallery.sh` | ✅ every state rendered |

### No fallback to the fixture, and that is the point

`balance_model` returns the mock only when there is **no session**. For a real one it
returns whatever the core says — including a skeleton while the count is in flight.
Falling back to `$1,383.28` there would be the app showing somebody a stranger's money
and calling it theirs.

The three states the tests pin are the ones the core went to trouble over:
- **unknown → skeleton, never `$0`** (invariant ②). A wallet that shows zero while it
  is still counting has told the person their money is gone. A test asserts the
  rendered integer contains no digit at all.
- **a real zero is not unknown.** Different state, drawn differently, on purpose.
- **hidden withholds by construction** (invariant ⑧). The core already nulls the
  total; the test asserts the shell does not reintroduce a figure.

### What a 50-second run of the real app produced

```
[vela-wallet] core: balance_dashboard booting
keys written: vela.accounts, vela.activeAccountIndex, vela.rpc.banned
balanceCache: ABSENT
BANNED: https://rpc.gnosischain.com  (temporary)
```

Both of those lines are the system working, and neither is obvious:

1. **The pool banned `rpc.gnosischain.com` by itself.** That is the endpoint 030
   phase 2 found refusing this HTTP client with 403 while curl gets 200 — recorded
   then as a debt with no fix. It now needs none: the pool met it, classified it,
   banned it, routed around it, and wrote the ban down so the next launch does not
   spend a request rediscovering it. The debt closed itself the moment the machinery
   that owns the decision was wired.
2. **No balance cache was written, and that is correct.** Tempo was unreachable, so
   the result was partial, and the core's complete-results-only write gate (invariant
   ⑥) refused to ask for the write. A cached total assembled from eleven of twelve
   chains is a number that was never true. Distinguishing "the core refused" from
   "the fetch had not finished" needed a 50-second run rather than a 25-second one —
   the 25s run looked identical and would have supported the wrong conclusion.

## Phase 4 — the handoff contract, mostly closed

Four of the five arms 030 marked `// live in 031` are live.

| Arm | 030 | now |
|---|---|---|
| `contacts::classify_recipient` | `code: None` | `eth_getCode` through the pool |
| `display_currency::read_device_currency` | `None` | the region's ISO-4217 |
| `display_currency::resolve_rate` | `None` | the configured fiat endpoint |
| `network_admin::invalidate_pools` | acknowledged no-op | `pool::refresh` |
| `contacts::resolve_identity` | `None` | **still owed** — the waterfall needs the passkey index and name services |

| Gate | Result |
|---|---|
| `cargo test` | ✅ **137 passed · 0 failed · 13 ignored** |
| `cargo fmt --all --check` | ✅ clean · gallery ✅ · warnings 1, pre-existing |
| live, per module | ✅ pool 1 · balances 1 · display_currency 2 · contacts 1 · network_admin 3 |

Live evidence: `USD → CNY = 6.71907`, `USD → JPY = 156.014`; the golden Safe classifies
as **171 bytes of code** and `0x000…001` as **`0x`** — a verdict, and a different
answer from `None`.

**Two currency arms were one debt, and that is why they landed together.** A desktop
always had a region (`Loc::from_env`); what it lacked was a *rate*, and the core
persists a seeded currency only after a real rate resolves. A region candidate is
useless until something can price it.

### Three tests changed with the arms, which is the contract working

`a_chosen_currency_comes_back_unpriced_rather_than_invented` asserted `rate: None`.
That was correct in 030 and is wrong now, so it became
`a_chosen_currency_comes_back_priced` — the visible half of a fail-closed arm going
live. Likewise `the_unavailable_lookups_answer_unknown_rather_than_a_verdict` split:
history stays honestly empty (local), classification became a live test asserting a
contract and a non-contract answer **differently**, because conflating `0x` with
`None` is how a wallet calls somebody's own address a contract.

### Three process failures of mine, all the same shape

1. **A `str.replace` that matched nothing** because `cargo fmt` had reflowed the
   target — the same trap as 030 phase 6.
2. **A script that asserted *before* writing and aborted between the two**, leaving
   the file untouched while its log said "flipped". I then debugged a stale file.
3. Fixed by **verifying after the write**, not only asserting before it. Every edit
   since re-reads the file and asserts the new text is on disk.

### A test that failed because the world changed

`the_golden_safe_reads_across_chains` pinned `769970000000000000` wei. It went red at
`758970000000000000` — the Safe's balance **moved on-chain**, 0.011 xDAI spent by
something outside this session. A wallet balance is not a constant, and a test that
fails when the world changes is reporting the wrong thing. It now asserts what must
hold: Gnosis answered, the quantity is non-zero, the symbol and decimals are right.
The exact figures in this document stay as they are — point-in-time evidence, not
pins.

### Live tests run per module, and the reason is a real property

Run as one `executor::` set they interfere; per module they are green. The cause is
structural rather than accidental: **the pool is a process-wide singleton thread** and
`with_temp_state` swaps a process-wide `VELA_STATE_DIR` underneath it. Both facts are
correct on their own — one pool per process is the architecture, and per-test isolation
is how storage tests work — and they simply cannot share a process. These are
`#[ignore]`d manual gates, so per-module is the documented way to run them:

```
cargo test executor::pool -- --ignored --test-threads=1
```
