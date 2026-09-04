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

## Phase 5 — the activity feed, and the day boundary

`executor/activity_feed.rs` (six operations) plus `executor::day_start_ms`.

| Gate | Result |
|---|---|
| `cargo test` | ✅ **143 passed · 0 failed · 13 ignored** (031 opened at 125) |
| `cargo fmt --all --check` | ✅ clean · gallery ✅ · warnings 1, pre-existing |
| `check-windows.sh` | ✅ the `cfg(not(unix))` path type-checks |

### The day boundary is the shell's, and getting it wrong is visible

The core's words: *"LOCAL-midnight epoch ms — computed by the shell, which owns the
device timezone."* `vela-core` deliberately ships **no timezone database**, so this is
the one fact it cannot derive.

Grouping by UTC day files a 20:00 transaction in Tokyo under **tomorrow**, and a 19:00
one in New York under **today** when it belongs to yesterday. That is wrong for part of
every day for everybody outside Greenwich — not a rounding error, a heading with the
wrong transactions under it.

So the offset comes from `localtime_r`, and `libc` moved from a Linux-only dependency
to a `cfg(unix)` one. `localtime_r` rather than a value read once at startup, because
the offset must include **daylight saving as of this instant** — a cached offset is
wrong twice a year.

**Windows has no `localtime_r`** and `GetTimeZoneInformation` is not wired, so it
returns 0 and groups by UTC day. Recorded as a debt with its consequence spelled out
rather than left as a silent `#[cfg]`. `check-windows.sh` type-checks that path.

The test asserts the property rather than a figure, which is what makes it runnable on
a machine whose timezone it does not know: two instants an hour apart share a
boundary, two a day apart do not.

### Three answers that are reports, not decisions

- **A legacy row reports `kind: None`.** The core reads absent as `send` by its own
  rule (`t.type ?? 'send'`). Substituting `Send` in the shell would hide a legacy row
  from the core's own rule about legacy rows.
- **Deleting a missing record answers `DeleteFailed`.** A delete that removed nothing
  is a failure, not a quiet success: the row is still on the person's screen and the
  core has to know it is still there.
- **A haptic is answered on a machine with no haptics.** Skipped would leave the core
  waiting; the celebration simply runs with one fewer sense.

### Own accounts resolve locally, and case does not matter

`ResolveRecipientIdentity` checks the person's own accounts first — on disk, no
network — and compares lowercased. A wallet that misses its own account on casing
labels it a stranger. The ENS/name-service half is the same waterfall
`contacts::resolve_identity` still owes; they should land together rather than be
written twice.

### `ScanIncomingTransfers` answers zero, marked `// live in 032`

Receipt discovery is `getLogs` over the transfer allowlist plus `token_trust`
admission, and the records it persists are the same store 032's send path writes.
Zero new records is a true statement about a scan that found none — the feed simply
has nothing to celebrate yet.

## Phase 6 — the feed reaches the screen

`wallet/live.rs::activity_rows` and the home's activity list bound to `ActivityFeed`.

| Gate | Result |
|---|---|
| `cargo test` | ✅ **148 passed · 0 failed · 13 ignored** (031 opened at 125) |
| `cargo fmt --all --check` | ✅ clean · gallery ✅ · warnings 1, pre-existing |
| `*fixtures.rs` deleted lines in 031 | ✅ **0** |

### The bug this phase nearly shipped

`FeedItem` carries both `value` and `decimals`, which reads like raw-integer-plus-scale
— the same shape `BalanceToken` uses, where `balance` **is** raw. It is not. The web
renders it with `trimBalance(item.value)` and the core sums it with `parseFloat`,
neither of which scales: **`value` is the human amount already**.

Scaling by `decimals` would have printed every activity figure 10¹⁸ times too large,
and it would have looked deliberate — two fields that plainly belong together, used
together. A test now pins it: `1.5` renders as `+1.5`.

### Three render decisions the core deliberately does not make

- **Headers are dropped here, not filtered out of the core.** `FeedView::rows`
  interleaves day headers with items because the full Activity screen draws them; the
  home preview is a flat short list. Asking the core for a different shape would move
  a render decision into the machine.
- **The kind comes from the record, not the item.** `FeedItem` has only a direction;
  `FeedView::transactions` carries `kind`. Looking it up keeps the dApp distinction the
  mocks draw — a swap is not "sent", and labelling it so loses the one word that
  explains where the money went.
- **Privacy is read from the BALANCE view**, not the feed's own flag. Every money
  surface masks together; reading two flags is how one ends up out of step. The figure
  goes, the unit stays — H5's rule, with a test asserting the number cannot survive.

### The badge tint is the settings table

Read through `settings::model::chain_tint` — the same table the network rows use, which
is the same table the mocks use. A second colour map for the same chains is how one
screen's Polygon stops matching another's.

## Phase 7 — three more machines

`receive_watch`, `payment_request` and `manage_tokens`.

| Gate | Result |
|---|---|
| `cargo test` | ✅ **156 passed · 0 failed · 14 ignored** (031 opened at 125) |
| `cargo fmt --all --check` | ✅ clean · gallery ✅ · warnings 1, pre-existing |
| live ERC-20 read | ✅ `USDC / USD//C on xDai / 6 decimals` |

### Six of seven machines are wired

`rpc_pool`, `balance_dashboard`, `activity_feed`, `receive_watch`,
`payment_request`, `manage_tokens`. Only `token_trust` remains.

### `MulticallErc20Meta` is three calls, and the name is the core's word for a want

The operation is named for what the web does — one `aggregate3` against
Multicall3. This asks `symbol()`, `name()` and `decimals()` separately: three
round trips, no Multicall3 encoding, the same answer. The name describes *what
the core wants*, not how; folding them into one call later changes nothing it sees.

**The failure rule is not deferred.** Metadata is all-or-nothing: a token with a
symbol and no decimals renders an amount at the wrong magnitude, and once saved it
stays wrong for as long as it is in the list. `None` unless all three answered.

The ABI decoders are hand-written and tested against real return data, including the
shapes that must produce `None` rather than a panic or a garbage symbol: a truncated
word, a non-hex body, an offset with no length behind it. A mojibake symbol saved into
somebody's token list is forever.

### Two "failed" answers that could easily have been quiet successes

- **Removing a token that is not there** answers `RemoveFailed`. The row is still on
  the screen.
- **Saving the same token twice replaces it** rather than appending. Adding the same
  contract again is a person correcting themselves, not two tokens.

### A float that is safe because of what it is *for*

`receive_watch`'s `TokenSnapshot.balance` is an `f64`, which would be wrong for money.
It is safe here because the number is only ever compared against an earlier snapshot
of itself to answer "did it go up" — never rendered, never summed. The comment says so
at the conversion, because the next person to read it will reasonably wonder.

### The pay-link base is the web wallet, deliberately

`payment_request`'s `base_url` points at `getvela.app`, not a desktop URL scheme. A
pay link is for somebody else to open, and a scheme most people cannot follow is a
link that does not work.

---

# 交接:下一个会话从这里开始

工作区 `/Volumes/data/production/vela-wallet-native`,分支 `031-desktop-read-wiring`
(叠在 `030-desktop-live-shell` 上,后者叠在 `029-native-repair` 上,均未合并)。

## 先读这三样

1. **本文件**(031 账本)与 `specs/030-desktop-live-shell/results.md`(含 SC 判定与
   五条结转欠账)、`specs/029-native-repair/results.md`。
2. `app-desktop/vela-wallet/src/resident.rs` 的模块注释 —— 解释了为什么常驻机器是
   gpui entity 而不是 `Global`,以及 `Answer` 为什么把线程边界做成类型。
3. `src/executor/pool.rs` 的模块注释 —— 为什么 pool 是独立线程而非 resident。

## 立刻可跑的闸门

```bash
cd /Volumes/data/production/vela-wallet-native/app-desktop/vela-wallet
cargo fmt --all --check && cargo test && scripts/sweep-gallery.sh
# 真网测试必须【按模块】跑,原因见本文件 Phase 4:
env -u all_proxy -u http_proxy -u https_proxy \
  cargo test executor::pool -- --ignored --test-threads=1
```

基线:**156 passed · 0 failed · 14 ignored**,fmt clean,36 个画廊状态,1 个既有
warning(`BLE_CHANNEL_SUPPORTED`)。

## 031 还剩五件

| # | 事 | 备注 |
|---|---|---|
| 1 | `token_trust` | 最后一台机器(1,937 行 core / 6 ops) |
| 2 | ERC-20 余额 | `balances.rs` 目前**只读原生币**;需要 Multicall3 编码 + 代币列表 |
| 3 | 价格 | `price_usd` 全是 `None`,所以余额英雄区还渲染不出法币数字 |
| 4 | `resolve_identity` | 唯一还挂 `// live in 031` 的臂;与 `activity_feed` 的名字查询是同一个瀑布,**应一起做** |
| 5 | 收款/资产屏绑定 + 收账 | SC 判定表 |

## 三条容易踩的坑(我踩过)

1. **`str.replace` 静默不匹配** —— `cargo fmt` 会把目标重排。改文件后必须**重新读回
   并断言新文本在盘上**,只在写之前断言是不够的(我为此调试过一个陈旧文件两轮)。
2. **真网测试不能同进程一起跑** —— pool 是进程级单例线程,`with_temp_state` 会在它
   脚下换掉进程级的 `VELA_STATE_DIR`。两者各自都对,但不能共处一个进程。
3. **别把链上数字钉进断言** —— 金标 Safe 余额在本次会话中间就变了(0.76997 →
   0.75897)。断行为,不断金额。

## 032 开工前必须先做的一件事

**固定密钥集签名者要用 Rust 写进 vela-core(`dev-fixtures` feature),而且要在 032
的**第一个** phase,不是花钱那一刀里。** 那三把是裸 P-256 私钥,而 desktop/Android/iOS
每条签名路径都通向**导不进密钥的真实认证器**,所以金标密钥集目前在三端一条路都走不
通。这是三刀花钱 spec 共同的前置条件;放到 032 中段才发现,验收标准就无法满足。
vela-core 已有全部零件(`webauthn.rs` / `registry_proof.rs` / p256),约 150 行。

## 033 开工前必须先测的一件事

给 `vela-core-uniffi` 加 17 台机器的 `bridge_object!` 之前,**先测体积**。spec 019
记录的闸门是 arm64-v8a **+785,864 剥离字节**;我的估算是再加 **+3~5 MB**,必须在
033 的 plan 签字前用半小时的探针量出来(三刀分别量:3 / +A / +B / +C),而不是事后。
