# Results — 032 desktop-money-wiring

## Baselines — recorded 2026-09-05, branch point `049617f5` (tip of `031-desktop-read-wiring`)

Stacked on 031 for the reason 031 stacked on 030: this cut edits
`wallet/page.rs` and `flows/` heavily, and 031's live builders are the ones the
money screens extend. `origin/main` is 21 commits ahead of the branch point and
none of them touch `rust/` or `app-desktop/` (checked), so the stack is not
rebased yet.

### Desktop at the branch point

| | |
|---|---|
| `cargo test` (app-desktop/vela-wallet) | **222 passed · 0 failed · 26 ignored** |
| `src/**/*.rs` | 81 files |
| `wallet/page.rs` | 6,473 lines ⚠ |
| gallery states | 36 |
| `// live in 032` markers | 2, both `network_admin::ClearBundlerCache` |

### vela-core at the branch point

| | |
|---|---|
| `cargo test -p vela-core --features i18n-all,crux` | **1,234 passed · 0 failed** (summed over every test binary) |
| features | `crux`, `identicon-raster`, `bindings`, `i18n-*` — no fixture feature |

### The four machines of group A

| Machine | Core lines | Operations |
|---|---|---|
| `send` | 4,224 | 18 |
| `fee_policy` | 2,322 | 6 |
| `batch_import` | 1,648 | 3 |
| `tx_tracker` | 958 | 6 |
| **total** | **9,152** | **33** |

Group B (`clear_signing` 4,841 · `approval_guard` 2,425 · `sign_request`
2,096 = 9,362) is drawn (DCS1–8, 33 gallery scenarios) and waits on a request
source; it is not in this cut's critical path.

### What the web tier ported, and what this cut must write in Rust

| Web source (`app-web/vela-wallet/src/lib`, at `origin/main`) | Lines |
|---|---|
| `services/safe-transaction.ts` (the Safe user-operation assembly, verbatim from Expo) | 3,068 |
| `services/bundler-service.ts` (the relay client) | 948 |
| `flows/core/send-executor.ts` | 533 |
| `signing/core/sign-executor.ts` | 402 |
| `dev/passkey-fixture.ts` | 350 |
| `wallet/core/tracker-executor.ts` | 278 |
| `flows/core/fee-executor.ts` | 217 |
| `flows/core/batch-executor.ts` | 90 |

**Already in Rust and not to be ported**: the money math (`fee_policy`'s
`same_asset_fee_limit`, `to_base_units`, `max_native_sendable`, the reserves,
`encode_erc20_transfer`), Safe address derivation and the setup calldata
(`safe.rs`), EIP-712 hashing (`eip712.rs`), the WebAuthn digest and DER→raw
conversion (`webauthn.rs`), ABI encode/decode (`abi.rs`, `alloy-dyn-abi`),
and the desktop's own `executor/abi.rs` from 031.

**Genuinely missing in Rust**: the 4337 user operation itself — `initCode`
assembly, `executeUserOp` / MultiSend calldata, the SafeOp EIP-712 hash, the
Safe WebAuthn signature envelope (`abiEncodeWebAuthnSig`, the per-key signer
proxy `r` field), the dummy signature for estimation, the in-band fee leg —
and the relay's wire shapes.

### The blocker the handover named, and why it is phase 1

Every desktop signing path ends at a real authenticator: USB CTAP2, caBLE,
the macOS platform vault, `webauthn.dll`. None can be handed a private key, so
the parallel space's three P-256 scalars cannot sign on the desktop through
any path that exists. Every acceptance test in this cut needs that signer; it
is written first, in `vela-core`, so the bytes it emits come from the same
kernels a real assertion is checked with.

## Phase 1 — the fixed keyset signs in Rust, and the desktop has a parallel space

**What shipped**: the thing the 031 handover said to do first.

- **`vela-core::dev_fixtures`** (feature `dev-fixtures`, default OFF, ~300
  lines + 9 tests): the same three P-256 scalars and credential ids the Expo
  and web clients carry in `passkey-fixture.ts`, derived into the same three
  single-key Safes and the same multi-key Safe; a `build_assertion` that emits
  the exact bytes an authenticator would (`{"type":"webauthn.get",…}` client
  data, `rpIdHash ‖ 0x05 ‖ 0`, low-S DER over `sha256(authData ‖
  sha256(clientDataJSON))`); a `build_registration` whose `fmt: none`
  attestation object the core's own `extract_attestation_public_key` and
  `attested_credential_id` parse back, with the FROZEN `0x45` flags byte; and
  the web's signer-selection rule (`resolve_signer`). No new dependency — the
  signer is the `p256` the registry group proof already uses.
- **The golden locks moved into Rust** and held on the first run:
  `0xD400866e…130b`, `0x031d7D57…772b`, `0x58cd0ce6…1d3d`, and the multi-key
  `0x88cCA0Ee…6894` that every 031 live sweep read from.
- **`app-desktop/src/parallel_space.rs`**: the runtime gate
  (`VELA_PARALLEL_SPACE=1`, behind the cargo feature), the signer seam
  (`register` = first fixture not already founding the wallet, so the
  machine's own exclude list drives a three-key creation through all three;
  `assert` = the named credential, or fixture #1 / `VELA_PARALLEL_SIGNER=n`
  for the discoverable ceremony), and the badge — violet, not a token,
  rendered by `Root` over whichever screen is up.
- **Four seams in `executor/passkey.rs`**: `register`, `assert`,
  `supported`, `platform_supported` consult the space first. Without the
  feature the lines do not exist.
- **CI**: the `rust` job's clippy and test steps now enable
  `vela-core/dev-fixtures`, so the locks are enforced somewhere; the wasm
  canary deliberately does not.

**The artifact rule, met**: `rust/` changed, so `rust/pkg-web` was rebuilt
and committed. The wasm is **3,630,664 bytes — the same count 026 recorded**;
only the source fingerprint (and with it the asset's name) moved, because
the fingerprint hashes every source file and a feature-gated module is a
source file. `verify-web.mjs` 46,408 green; `gen-onboarding-types --check`
current.

**Recorded, not done**:
- The fixture assertion carries no user handle (`user_id_hex: None`), as the
  web's does. A parallel-space login therefore names the wallet from the
  registry, not from the credential. Same behaviour as web; stated so the
  next reader does not go looking for a bug.
- SC-302 (sign in to the golden Safe through the real onboarding machines)
  is not yet exercised end to end — that needs the registry round trip and
  is the natural first live check once a send exists to justify it.
- `platform_supported()` answers `true` in the space so the "this device"
  row is offered; the fixture reports `platform` attachment, so that is the
  row it is.

**Gates**: vela-core **1,234 → 1,243** (with `dev-fixtures`; 1,234 without —
the feature adds, never changes) · desktop **222 → 225** with the feature,
222 without · fmt clean both crates · `cargo clippy --workspace --all-targets
--features vela-core/dev-fixtures -- -D warnings` clean · desktop clippy adds
no warning in the touched files.

## Phase 2 — the user operation, in Rust, cross-checked

**What shipped**: `vela-core::user_op` (~640 lines, 21 tests), the pure half
of `safe-transaction.ts` — the part every native tier would otherwise write by
hand. Unconditional (no feature): it is money code the uniffi tiers will need.

- **Calldata**: `executeUserOp` (CALL), the MultiSend batch (DELEGATECALL
  into `MULTI_SEND`, packed `op ‖ to ‖ value ‖ len ‖ data`), the
  lone-call-stays-single rule, `transfer(address,uint256)`, the in-band fee
  leg in both shapes, `is_plain_transfer_call` by shape not size.
- **InitCode**: `factory ‖ createProxyWithNonce(singleton, setupData, saltNonce)`
  for one key (byte-identical to the historical single-owner setup, because
  it takes `compute_safe_address`'s own `setup_data`/`salt_nonce`) and for a
  founding set (`compute_safe_address_multi`'s).
- **The signer rule**: `signer_address_for` — shared `WEBAUTHN_SIGNER` for
  keys[0] or a one-key wallet, the key's own counterfactual proxy for a later
  key, an error for a foreign credential (never mis-encoded).
- **Hashes**: the SafeOp EIP-712 digest under the Safe4337Module domain; the
  SafeMessage digest under the Safe's own domain (EIP-1271, group B's).
- **The signature envelope**: `validity(12) ‖ r=verifier ‖ s=65 ‖ v=0 ‖
  len ‖ abi.encode(bytes authData, string clientDataFields, uint r, uint s)`;
  the EIP-1271 form without the window; the estimation dummy built by the
  same encoder (37-byte authData, `r = s = 1`).
- **Wire**: the v0.7 dictionary (`factory`/`factoryData` split, paymaster
  quartet, Vela extension fields such as Tempo's `feeToken`).
- **Padding and parsers**: `pad_gas_estimate` (×1.5, floors, +10,000);
  `parse_existing_user_op_hash`; `parse_hex_quantity`.

**Cross-checked, not just ported.** Three of the tests hold the hand-laid
bytes against an INDEPENDENT implementation already in this crate:
- `calculate_safe_op_hash` == `eip712::hash_typed_data` over the SafeOp
  typed data (13 fields, two of them `uint48`), and moves with the chain id;
- `compute_safe_message_hash` == the same hasher over `SafeMessage(bytes)`;
- the WebAuthn payload == `alloy_dyn_abi`'s `abi_encode_params` of
  `(bytes, string, uint256, uint256)` with a 37-byte and a 51-byte tail.
All three agreed on the first run. The web vector suite's assertions
(selectors, word layout, DELEGATECALL bit, factory prefix, signer rule,
parsers) are ported alongside.

**Two divergences, both stricter**, recorded in the module note: `r`/`s`
must be exactly 32 bytes; a non-hex quantity is a typed error rather than a
thrown `SyntaxError`.

**Not ported here, on purpose**: everything that reads the chain or the
relay — `isDeployed`, the nonce and its 10 s cache, the gas signals, the
quote, the estimate, the submit and its 3× retry, the receipt wait. Those are
the desktop executor's (phase 3), in `sendUserOpInBand`'s order. Tempo's
variant (`feeToken` extension + splitter) is a later phase. `encode_erc20_transfer`
now exists twice — `fee_policy`'s hex-string form behind `crux`, this
byte form without — because FR-308 forbids touching the machine file; the
next machine change should make `fee_policy` call this one.

**Gates**: vela-core **1,243 → 1,264** · fmt clean · `cargo clippy
--workspace --all-targets --features vela-core/dev-fixtures -- -D warnings`
clean · `rust/pkg-web` rebuilt — the wasm is **3,630,664 bytes again**: the module is dead code to the wasm crate, so only the source fingerprint moved.

## Phase 3 — the relay, the reads, and the submit spine

**What shipped**: the desktop side of the money path below the screens —
five new executor modules, one pool extension, one marker flipped.

- **`executor/relay.rs`** (the bundler over HTTP): the REST base asked of the
  pool (`bundler_base`, invariant ③ — the same relay the op goes to) with the
  configured/built-in host as fallback; `/v1/account` (30 s cache, Tempo's
  pathUSD branch, a corrupted `settlementRecipient` degrading to the deposit
  address) and `/v1/treasury` (404 = uncovered, anything else transient =
  `Unknown`, never `Uncovered`); `vela_getInBandGasQuote` rows parsed with
  the web's admission rules (8 s cache, the no-native-price stablecoin
  filter); the raw `pimlico_getUserOperationGasPrice` tier, UNJUDGED;
  `eth_estimateUserOperationGas`; `eth_sendUserOperation` with the 3×/3 s
  busy retry; the receipt and status polls collapsed to the tracker's typed
  axis; and the two wording parsers the machines leave to the shell.
- **`executor/chain.rs`**: `is_deployed` (only `true` is cached; an
  indeterminate read is an error, never a guess), the EntryPoint nonce (10 s,
  bumped after a submit), the raw gas signals, `chain_gas_price` with the
  5-gwei default, `verify_chain_ready`.
- **`executor/user_op.rs`**: `simulate_gas` (the quote's estimate, byte-
  identical to the submit's MultiSend) and `submit` — `sendUserOpInBand` and
  `sendUserOpTempo` in their order: chain ready → deployment + nonce (the two
  refusals) → placeholder-leg estimate → the DISPLAYED fee baked in (or the
  web's send-time fallback quote through `fee_policy`'s amount rule) → SafeOp
  hash → the passkey seam → the contract-signature envelope → the AA20 guard
  → submit with `[existingHash]` recovery → nonce bump. The failure
  vocabulary maps 1:1 onto `SendSubmitFailure`.
- **`executor/fee.rs`**: `impl Machine for FeePolicy` — six operations, six
  reads, the keys of an undeployed account read from the stored record.
- **`executor/tracker.rs`**: `impl Machine for TxTracker` as the app-resident
  it must be — the pending-record sweep over `vela.transactionHistory`, the
  in-place patch, the receipt's authentic logs held per hash and handed to
  `token_trust::receipt_confirmed` (the single auto-add entry point) — plus
  `start` (boot + a 3 s tick loop that re-fetches the resident so a sign-out
  cannot strand it) and `submitted`.
- **`executor/pool.rs`** learned the two questions that are not routed calls:
  `bundler_base` and `best_rpc_url`, answered from the core's own
  `BundlerBase` / `BestRpcUrl` verdicts through a query table beside the
  in-flight map.
- **`network_admin::ClearBundlerCache` is live** — the last two `// live in
  032` markers are gone.

**Live, against Gnosis** (per module, `--test-threads=1`, proxy unset):

| Read | Answer |
|---|---|
| relay base | `https://vela-relay.getvela.app` — no chain suffix; treasury **Covered** |
| golden Safe quotes | XDAI native, recipient `0xee2c…f0dd`, balance 0.75897; USDC and USDT rows at 0 |
| fast bundler quote | `maxFeePerGas 17`, no network/relayer fields (the core's fallback applies) |
| account info | `settlementRecipient 0xee2c…f0dd`, status ACTIVE, **`activeDepositAddress` absent** |
| golden nonce / signals | `0x…04`; `eth_gasPrice 11 · baseFee 10 · tip 2 → 12` |
| a dust transfer's estimate | `100000 / 112472 / 101600` |
| an unknown hash | receipt: reached, nothing; status: `None` (the relay has no status method, or answers nothing) |

Two of those are worth a line. The relay publishes no `activeDepositAddress`
for the golden Safe on Gnosis — so `fee_recipient()` is the settlement
recipient alone, and a funding sheet that showed the deposit address would
show nothing; the in-band path does not need it. And `eth_getUserOperationStatus`
answered nothing for an unknown hash, which the tracker reads as
`StatusUnavailable` — an honest unknown, exactly the case the core's window
logic exists for.

**Not exercised**: a real submit. `submit` is compiled and its envelope is
proven on the fixture keyset (the second key's own proxy in `r`, a foreign
credential refused), but no dust has moved — that is SC-303 and it needs the
send host of the next phase to carry a quote the core displayed.

**Gates**: desktop **225 → 244** with the feature (240 without) · fmt clean
· warnings 1 (pre-existing) · `rust/` untouched this phase.

## Phase 4 — the send host, and the screens

**What shipped**: the desktop sends. Every drawn send panel (DSD1L–DSD4L,
DSD2eL, DSD2fL) now reads the `send` and `fee_policy` machines when a person
is signed in, and every affordance on them is an EVENT to the core.

- **`executor/send.rs`** — the nineteen operations as the web executor has
  them: the token fetch through `balances::fetch_all` (chains read AFTER the
  fetch), the credential lookup, the record persistence in one write, the
  identity waterfall, the recipient risk (`eth_getCode` with the EIP-7702
  delegated-EOA exemption + the prior-interaction read), and `SubmitUserOp`
  as the sign closure over `passkey::assert` — routed on the first key's
  transports (a phone over caBLE, a platform vault, or a security key asked
  DISCOVERABLY so any founding key on it can answer). Four arms are the
  screen's and say so.
- **`wallet/money.rs`** — `SendHost`: one `send` + one `fee_policy` per
  journey, born with the flow and dropped with it; `EstimateFee` answered by
  the live fee session (deployment read → `QuoteRequested` → settle on the
  same view the card renders); the card's re-quotes mirrored back as
  `FeeBusyChanged` / `FeeUpdated`; `TrackSubmitted` handed to the resident
  tracker whose view the host observes and forwards as the three
  `ReceiptUpdate` verdicts; the ceremony channel's poll for PIN / pick /
  touch / QR; `SigningStarted` raised by the sign closure and dispatched
  once.
- **`flows/live.rs`** — `send_pick`, `send_form`, `send_confirm`,
  `send_receipt`, `fee_token`, `contact_pick`, and `send_panel` (the core's
  stage → the panel; the two pickers are the core's flags, the fee sheet is
  the page's). The receipt reads `receipt.status`, never `tx_status`.
- **`flows/panels.rs`** — the drawn gaps closed as props: per-row listeners
  on the token, contact and fee-coin rows; editable amount and recipient
  fields (the value is the core's; the field holds no copy); the Max chip;
  an address-book pill beside the typed recipient. Fixtures untouched; the
  gallery renders every state exactly as before (sweep: 36, every state
  rendered).
- **`wallet/page.rs`** — the flow stack is REBUILT from the core's stage on
  every frame of a live send; the chevron asks the core to step back; the
  column's close and the core's `Close` both drop the machines; the cable's
  three dialogs and the core's alert render over the column; the tracker
  starts on sign-in. Eight new strings resolve from keys the corpus already
  had (`send.txConfirmedTitle`, `send.txSubmitting`, …).

**Live, headless, for the golden Safe** (`wallet::money` — a synchronous
pump of both machines, the same performs as the host minus the thread):

| step | the core said |
|---|---|
| `Open` | 1 holding: **xDAI on Gnosis, 0.75897** — the number 031's hero showed |
| `SelectToken` → `SetRecipient` (fixture #2) → `SetAmount "0.001"` | `can_continue: true`, no warning |
| `Continue` | stage **Confirm**; fee **0.010 xDAI** native, quoted (not a local fallback), recipient `0xee2c…f0dd`; treasury none; no alert; **`can_confirm: true`** |

That fee is the very figure 026's web sweep signed on the same Safe
(0.001 sent + 0.010 in-band). The slide is behind `VELA_LIVE_SEND=1` and was
NOT pulled: it spends dust, and that is the founder's call (SC-303).

**Recorded, not done**:
- `AddNetwork` from a locked request answers the ported `catch` (`Error`);
  the settings wizard is the desktop's add-network journey and the send flow
  has no `/pay` link to arrive from yet.
- `SimulateCalls` answers no simulation — no engine on the desktop.
- The batch importer (DSD2cL) still draws the fixture; its machine is phase 5.
- The scan row (DS1) stays a picture; no camera pipeline.
- A real USB / caBLE / vault signature for a send has not been run on a
  device this session; the parallel space's signer went through the same
  `passkey::assert` seam login uses.

**Gates**: desktop **244 → 251** with the feature (247 without) · fmt clean
· 1 pre-existing warning · gallery sweep every state rendered ·
`check-windows.sh` green · `rust/` untouched.

## Phase 5 — paying many at once

**What shipped**: the batch importer runs on its machine, and DSD2cL is
the last send panel to stop being a picture.

- **`executor/batch.rs`** — the three capabilities the core cannot have:
  the USD→fiat rate through `display_currency::resolve_rate` (never a
  display helper that ends in `?? 1` — a CNY payroll split at 1:1 is ~7× the
  intended payout behind a green button); a picked table as text for
  CSV/TSV/TXT (the core parses it) or as a cell matrix for a workbook
  (`calamine` reads the first sheet, integral floats print as the person
  typed them, short rows are padded so column positions hold); the file's
  name for the sheet.
- **`wallet/money.rs`** — the `batch_import` core is born when the send
  machine raises `show_batch_import` and dropped when it falls, so a stale
  paste or rate can never survive a re-open (the core is new). The two
  dialogs are gpui's and awaited in the host with the file work off-thread;
  `applied` seeds the send machine's split editor with exactly the core's
  drafts and closes the sheet. The desktop's paste: the drawn box is not a
  text editor and the custom field ignores ⌘V, so clicking the box reads the
  clipboard.
- **`flows/live.rs::batch_import`** — the rate line in its three states
  (loading / a number / "enter one manually"), the no-price hint, the
  preview rows (a converted row says the token amount, an unconverted one
  its raw fiat), the rejected count (one / other plural keys), the over-cap
  and over-balance notices side by side, the Apply CTA counting only the
  rows that will be sent and drawn shut when the core shut it.
- **`flows/panels.rs`** — the unit toggle's halves, the paste box, the
  file and template affordances, the rate as a field with its Auto pill:
  bindable props, fixtures untouched, the gallery pixel-unchanged.
- **One new dependency**: `calamine` (pure Rust, no system library), read
  only. The template the sheet saves is CSV text the core composes.

**Headless, hermetic**: a pasted two-row USD table previews two rows, counts
the address-less line as rejected, converts at the self-priced rate, and
applies to exactly two drafts (`wallet::money::a_pasted_table_applies_to_its_rows`).

**Recorded**: no workbook fixture is committed, so the xlsx path is covered
by the cell codec and the not-a-zip refusal rather than a real sheet; the
sweep mode (N tokens → one address) stays fixture, as on web.

**Gates**: desktop **251 → 256** with the feature (252 without) · fmt clean
· 1 pre-existing warning · gallery sweep every state rendered · `rust/`
untouched.

## Phase 6 — the screen says what the core refuses

**The finding, from a sweep of my own two phases.** `SendView` carries
sixteen fields that are judgements — what is wrong, what is waiting, what may
not proceed — and the live builders I wrote in phase 4 read **none** of them.
Type more than you hold and the Continue button simply does not respond:
the core computed the sentence (`send.warnNotEnoughToken`, in every locale)
and the desktop threw it away. Fifteen more sat beside it.

| the core says | phase 4 | now |
|---|---|---|
| `amount_warning` (4 sentences) | dropped | amber notice under the fee |
| `same_asset_fee_issue` | dropped | title + "sending X + fee Y needs Z, you have B" + "you can send up to M" + **Edit amount** |
| `split_over_balance` | dropped | red notice |
| `confirm_amount_issue` | dropped | red notice + Edit amount, confirm page only |
| `denom_toggle_reason` | dropped (no ⇄ control drawn) | said as a warning, since no control exists to explain itself |
| `treasury_bootstrap` | **silent dead end** | the funding sheet's own words + the top-up address + the shortfall + **Check now** |
| `lock_error` / `add_network_msg` | dropped | the lock's title/body + the last attempt's outcome + **Add this network** |
| `can_continue` / `can_confirm` | CTA always armed | drawn shut, answering to nothing |
| `estimating_gas` / `sending` / `tx_status` | nothing | the button says "Estimating…" and keeps its accent — busy is not disabled |

Every word came from the corpus (`send.warn*`, `send.sameFeeToken*`,
`send.lock.*`, `componentsUi.funding.*`, `componentsUi.gas.estimating`);
**zero new keys**. One drawn model gained `notice` and `cta_state`; the
fixtures are untouched and the gallery renders exactly as before.

**One traversal, two readers.** `build_notice` returns the sentence AND the
way out (`NoticeWayOut::{RetryAfterBootstrap, AddNetwork, EditAmount}`), so
the button and the words can never disagree about what is being offered. The
page maps the way-out to the core's own recovery event.

**Two more drawn-but-dead affordances closed**: a group in the contact picker
now seeds a split with everybody in it (the hand-off web calls 群发转账), and
the notice's action is the only new button — it is the core's, not one this
file invented.

**A wrong assumption the test caught.** I asserted that an over-balance amount
disables Continue. It does not: the ported gate arms it and the refusal
arrives as an ALERT when it is pressed (`SendAlertKind::InsufficientBalance`,
`can_continue` stays true for an unlocked send). So between typing and
pressing, the warning sentence is the **only** thing on screen — which is
precisely why dropping it was worse than it looked. The test now drives the
real machine through both steps and asserts the sentence, the armed button,
the alert and that the flow stays on the form.

**Two more of the same class, found by carrying the sweep into phase 5's
work**: the fee sheet drew a coin that cannot cover the fee as selectable
(`FeeOptionView.insufficient` dropped — the core's invariant ⑧ says such a
row is shown for context and is NOT selectable, because paying gas in it only
produces a doomed operation), and a picked file the shell could not read
raised `BatchView.file_error` that nothing showed — a picker that silently
does nothing is indistinguishable from a broken one. Both fixed; the fee gate
lives in ONE place (the panel dims the row and drops its listener from the
core's own flag — a second gate in the page would be a second opinion about
one fact), and the over-balance refusal now carries the figure it is about.

**Still not drawn** (recorded, not hidden): the ⇄ fiat/token control (its
refusal is now spoken, but the control itself is a drawing the desktop does
not have), the multi-token sweep picker (`multi_select_mode` and its
checkbox column), and per-row editing of a split (a seeded group's amounts
are typed in the batch importer or not at all).

**Gates**: desktop **256 → 258** with the feature (254 without) · fmt clean ·
1 pre-existing warning · gallery sweep every state rendered · the live spine
still reaches Confirm with the relay's real 0.010 xDAI quote.

# 交接:下一个会话从这里开始

**范围:只做 desktop。** 分支 `032-desktop-money-wiring`(叠在 031 → 030 → 029 上,均未合并)。
工作区 `/Volumes/data/production/vela-wallet-native`,五个 phase,六个提交。

## 一句话状态

**桌面能发钱,只差最后一推**:金标 Safe 真网走到确认页(真持仓、真报价 0.010 xDAI、
滑块已武装),`SlideConfirm` 藏在 `VELA_LIVE_SEND=1` 后面没拉——花真钱是创始人的决定
(SC-303)。固定密钥集签名者在 vela-core(`dev-fixtures`),4337 UserOp 装配在 vela-core
(`user_op.rs`,与 EIP-712 哈希器和 alloy ABI 编码器交叉验证),中继/链读/提交主干、
fee_policy 与 tx_tracker 常驻、send 宿主与七块屏(含批量导入)全接。**A 组五个 phase 全完。**

## 立刻可跑的闸门

```bash
cd /Volumes/data/production/vela-wallet-native/app-desktop/vela-wallet
cargo fmt --all --check && cargo test --features dev-fixtures && cargo test \
  && scripts/sweep-gallery.sh && scripts/check-windows.sh
# 真网(按模块,不并发;proxy 变量要清掉):
env -u all_proxy -u http_proxy -u https_proxy \
  cargo test --features dev-fixtures executor::relay -- --ignored --test-threads=1
# …同样跑 executor::chain / executor::user_op / wallet::money(到确认页为止)
cd ../../rust && cargo fmt --all --check \
  && cargo clippy --workspace --all-targets --features vela-core/dev-fixtures -- -D warnings \
  && cargo test -p vela-core --features i18n-all,crux,dev-fixtures
```

基线:desktop **258 passed(feature on)/ 254(off)· 32 ignored**,vela-core **1,264**,
fmt clean,gallery 36 态全渲染,1 个既有 warning(`BLE_CHANNEL_SUPPORTED`)。

**动过 `rust/` 就要**:`node rust/scripts/build-web.mjs`(不是 `--check`——指纹一定会动,
要重建入库)→ `verify-web.mjs` → `gen-onboarding-types.mjs --check`。本刀两次都是
wasm 3,630,664 字节不变、只有指纹改名。

## SC-303:那一推怎么拉

```bash
env -u all_proxy -u http_proxy -u https_proxy VELA_LIVE_SEND=1 VELA_PARALLEL_SPACE=1 \
  cargo test --features dev-fixtures live_the_golden_safe_reaches_confirm -- --ignored --nocapture
```
它会用 fixture #1 签 SafeOp、真提交到 vela-relay、打印 userOpHash;花 0.001 + 0.010 xDAI。
收据核对:余额差 = 0.011,与屏幕数字逐位对上(026 的 web 巡检就是这个数)。跑完把
结果写进 SC-303 的判决。**注意** `VELA_PARALLEL_SPACE=1` 是进程级 env,测试里
`passkey::assert` 靠它路由到固定密钥集;不设它,签名会去找 USB 钥匙。

## 还欠的

| # | 事 | 状态 |
|---|---|---|
| 1 | ~~Phase 5 批量导入~~ | **已交付**(phase 5):`executor/batch.rs` + SendHost 里的 `CoreHost<BatchImport>`,DSD2cL 活了。没提交 xlsx 样张,真表格实机点一次 |
| 2 | **B 组签名面板**(`clear_signing` `approval_guard` `sign_request`,9,362 行) | 图 DCS1–8 画好、33 个手写场景;`user_op::compute_safe_message_hash` 与 `build_eip1271_signature` 已备好。请求来源仍缺(C 组要 web 引擎) |
| 3 | 真实认证器签一笔发送(USB / caBLE / 平台库) | 本刀没插过钥匙。走的是登录同一条 `passkey::assert` 缝,理论上同路;实机跑一次 |
| 4 | `SendOperation::AddNetwork` | 答 `Error`(移植的 catch 分支)。锁定请求要加网时应走设置向导 |
| 5 | `SimulateCalls` | 桌面没有模拟引擎,答 `None` |
| 6 | 031 留的五件:收藏控件、设置页新建/登录账户、扫码、余额流式、Windows 日界线 | 原样 |
| 7 | Tempo 提交路径 | 已移植(`submit_tempo`)但没在 Tempo 链上跑过 |
| 8 | ⇄ 法币/代币切换控件、多币归集(sweep)选择器、拆分行逐行改额 | 桌面**没画**。phase 6 已把 ⇄ 的拒绝理由说出来了(核心的 `denom_toggle_reason`),但控件本身要图 |

## 本刀最值得记的四件事

1. **`#[allow(dead_code)]` 会把被调用者也标成活的。** 给 `user_op::submit` 加一个
   allow,`chain.rs` 里十个"never used"一起消失。接线前用它压警告,接线后记得删。
2. **fee 会话必须只有一个。** `EstimateFee` 由确认卡渲染的同一个 `fee_policy` 会话回答,
   宿主在它 `busy=false` 时用**它渲染的那个视图**结算——web 记录了四次因为拆成两个对象
   而失败的集成。`SyncMoney` 测试就是这条规则的无 gpui 版本。
3. **收据读 `receipt.status`,不读 `tx_status`。** 核心签完名就把 `tx_status` 翻成
   confirmed;真正追链的是 receipt 自己的状态。读错一个字段就是"钱到了"的谎话。
4. **gpui 细节两条**:`AsyncApp::update` 直接返回值(不是 Result),`Entity::update`
   在 AsyncApp 上返回 `()`;`cargo test` 只吃一个过滤词,第二个会被静默丢弃(我以为跑了
   两组测试,其实一组都没跑)。

## 028 合并后要立刻做的(web 会话 2026-09-05 预警,commit `6cec4ddf`,尚未在 origin/main)

028 把联系人导入/导出的规则从桌面的 `executor/contact_io.rs` **提进了核心**
(`app/contacts_io.rs`),并改了 `contacts.rs` 的事件与视图字段。rebase 到含 028 的 main 后:

1. **编译断点(自报)**:`src/contacts/live.rs` 测试辅助函数手写的 `ContactsView` 字面量
   要补 `import_failure: None, export: None, sections: Vec::new()`。
2. **编译器看不见的语义偏差**:坏文件(非法 JSON、空表/无地址列的 CSV)在 web 上会被
   **拒绝**(`ContactsView.import_failure: malformed_json | no_address_column | empty |
   unknown_group`),在桌面上现在仍是"成功导入 0 条"。修法 = 把 `page.rs` 的
   `import_contacts` / `export_contacts` 改成派发核心事件
   `ImportFile { content, filename, into_group, now_ms }` / `ImportAcknowledged` /
   `ExportRequested { scope, format, exported_at_iso }` / `ExportTaken`(导出文件在
   `ContactsView.export` 里一次性出现,壳只负责存盘对话框),然后**删掉**
   `executor/contact_io.rs` 及其测试。028 的 results.md Phase 6b 记了这条偏差,以这个切换为终点。
3. 新事件 `add_group_members` / `remove_group_member` / `set_contact_groups`——桌面分组
   的"添加成员"现在走哪条事件,切换时顺手核对。
4. `send.rs`:`picked_address` 自己关选择器(本刀的 Dsd2e 监听已经"选中 + 关闭"双发,
   新核心下第二个事件是空操作);`open()` 立刻把 `prefilled_recipient` 放进 `recipient`。
5. **`rust/pkg-web` 会冲突**:028 重建了 wasm(`08aa37e9ddf9`),032 也重建过(`df236de771e0`)。
   后合并的一方 `node rust/scripts/build-web.mjs` 重建入库即可,别手动合。028 不动 `ci.yml`。
6. **分组字母归核心了**:`ContactsView.sections: Vec<ContactSection { letter, addresses }>`
   (新 `app/contacts_initials.rs`,逐码点拼音首字母表:阿豪→A、妈妈→M、地址→#;A–Z 再 #)。
   桌面 `contacts/live.rs` 自己的 `section_of` / `sections()` 归档规则应改读 `view.sections`
   并删掉本地规则——同一个人不能在两端归到不同字母下。

## 每次接手仍要跑的一条 grep

```bash
grep -n 'fixtures::' src/wallet/page.rs
```
本刀新增的 Dsd 臂全部走 `send_views(cx)` 门:有宿主读核心,没宿主画 mock;phase 5 后
`FlowPanel::Dsd2c` 也读 `batch_view`。已登录能点到的界面里只剩 explore(等 web 引擎)
和 DS1 扫码(等相机)在画 mock。

**但 031 那条 grep 不够。** 它抓"还在画 mock 的界面";phase 6 抓到的是另一类——
**界面是活的,却把核心算出的判断丢在地上**。第二条 grep,每接完一台机器就跑:

```bash
# 视图给了什么(判断字段) vs live 构造器读了什么
grep -o "    pub [a-z_]*" ../../rust/crates/vela-core/src/app/<machine>.rs
grep -o "view\.[a-z_]*\|send\.[a-z_]*\|option\.[a-z_]*" src/flows/live.rs | sort -u
```
差集里每一个 `warning` / `issue` / `failure` / `can_*` / `insufficient` /
`*_error`,都是核心替人算好、屏幕却不说的一句话。phase 6 一次找出十六个,
其中三个(余额警告、手续费币种不可选、文件读不出)直接影响钱。
