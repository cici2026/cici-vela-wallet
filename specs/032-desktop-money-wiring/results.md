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
(0.001 sent + 0.010 in-band). The slide was behind `VELA_LIVE_SEND=1` — it
spends dust, and that was the founder's call. **They made it on 2026-09-07;
the slide is pulled, and SC-303 is proven.** See below.

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

**The sweep, carried across every machine the desktop reads.** The same
comparison — a view's fields against what a live builder reads — run over
`balance_dashboard`, `activity_feed`, `manage_tokens`, `receive_watch`,
`display_currency`, `contacts`, `network_admin`, `token_trust`,
`payment_request`. Triaged:

| unread | verdict |
|---|---|
| `BalanceView.{failed_chain_ids, rate_limited_chain_ids}` | **correct** — the core hands over `banner_chain_ids` (failed MINUS rate-limited, invariant ⑦) and 031 reads that. A rate limit lifts on its own and must never raise the "fix your RPC" banner |
| `BalanceView.{balance_partial, cached_total_usd, last_refreshed_at_ms}` | covered by `notice: StillUpdating` and `holdings_loading`, both read; no "as of" line is drawn |
| `MtokView.save_error` | **a real gap, fixed here** — adding a token could fail and the button just did nothing, twice |
| `NetWizardView.{phase, error}`, `NetView.last_added_chain_id` | the settings add-network wizard: a debt, and the same class. Not this cut's surface |
| `FeedView.{toast, new_item_id}` | the "money arrived" celebration — no drawing on the desktop |
| `ContactRecipientView.{saved, verified, is_contract, first_interaction}` | the contacts detail's trust line. The SEND path's own risk is live (phase 4); this one is 028's territory |
| `PaymentRequestView` (10/14) | the pay-link surface — `/pay` has no desktop entry yet (debt #4's neighbour) |
| `TrustView`, `TrustSimView`, `TrustIncomingView` | consumed by the executor and the feed writer, not by a display model |
| `BalanceSwitcherView` | the home's own account switcher — undrawn on the desktop (settings has one, 031) |

**Still not drawn** (recorded, not hidden): the ⇄ fiat/token control (its
refusal is now spoken, but the control itself is a drawing the desktop does
not have), the multi-token sweep picker (`multi_select_mode` and its
checkbox column), and per-row editing of a split (a seeded group's amounts
are typed in the batch importer or not at all).

**Gates**: desktop **256 → 258** with the feature (254 without) · fmt clean ·
1 pre-existing warning · gallery sweep every state rendered · the live spine
still reaches Confirm with the relay's real 0.010 xDAI quote.

## SC-303 — the dust moved

创始人 2026-09-07 点头,滑块拉了。

```
after continue: stage=Confirm fee=("10000000000000000", Native, "0xee2cca98…f0dd")
                fee_busy=false treasury=None alerts=[] can_confirm=true
relay: submitting sender=0x88cCA0…6894 nonce=…04 initCode=no callData=452B signature=429B
after slide: user_op_hash=Some("0x4d1cf38350afc661b18caa4c9862ffcef2d2a579ac1e4f6e3cec24d4e529849c")
test result: ok. 1 passed … finished in 88.86s
```

**链上核对(不看 `tx_status`——见第 3 条教训,那个字段签完名就翻)**:

| 查什么 | 屏幕说 | 链上 |
|---|---|---|
| 金标 Safe 余额差 | 0.001 + 0.010 = **0.011** | 0.75897 → **0.74797**,差 **0.011000**,逐位对上 |
| 收款方 fixture #1 `0x031d…772b` | +0.001 | 1.01599999… xDAI(收到) |
| 中继收费地址 `0xee2c…f0dd` | 0.010 | 0.078896… xDAI(收到) |
| **`receipt.status`** | — | **`0x1`**,`eth_getUserOperationReceipt` 的 `success: true` |
| 上链交易 | — | `0x98c8f65c6a9fa77906113022974ef2af2f74049c61ef718aa00fad0cfe9adfc9`,区块 48120671 |

**SC-303:达标。** 桌面从真持仓、真报价、固定密钥集签名、真中继提交,到链上收据状态,
整条链路走通,数字和屏幕一致。

## Phase 7 — the same sweep, on somebody else's screen

交接表里的第 9 条:加网络向导。**不是本刀画的界面**,但是本刀 phase 6 那个毛病的同一株
——核心把判断算好了,屏幕不说。这一刀把 phase 6 的规矩搬过去。

`NetWizardView` 有 `phase` 和 `error`,`NetView` 有 `last_added_chain_id`;桌面
`settings/live.rs` 一个都没读(第二条 grep 的差集)。后果不是难看,是**对话框看着坏了**:

| 核心说什么 | 之前 | 现在 |
|---|---|---|
| `phase: Searching` | 无 | 转圈 + "Searching…" |
| `phase: Resolving` / `Checking` | 无——点完一条建议,索引解析加一轮 RPC 竞速,几秒里对话框一动不动 | 转圈 + "Checking compatibility…" |
| `error: AlreadyAdded` | 无——最常撞上的那条:挑一条钱包已有的链,向导原地停死,CTA 也不画,对话框像是没反应 | "This network is already added" |
| `error: NotFound` | 无 | "Chain info not found" |
| `error: NoRpcEndpoint` | 无 | "{链名} RPC unavailable"——链已经解析出来了,句子就该点名它;底下那个自定义 RPC 框就是出路 |
| `error: NotCompatible`(扫码路径) | 无 | "Incompatible" |
| `last_added_chain_id` | 没读:**按下就关**对话框 | 关不关由核心说了算 |

最后一条是本刀改动里唯一动了行为的。`add_confirmed` 的每一道门(未加载、非 `Checked`、
不兼容)都是**静默** `return done()`,而壳按下就把对话框关掉——一旦哪道门拦住,人按了一下,
屏幕消失,什么都没加。现在壳在派发前后各读一次 `last_added_chain_id`,变了才关;没变就把
对话框留在原地,上面那行拒绝理由自己会说话。

### 顺着同一把尺子往下查:网络卡片上的那句"已保存"

第二条 grep 对 `network_admin` 的 38 个视图字段跑完,还剩四个没读:`rpc_save_deferred`、
`explorer_health`、`bundler_url`、`native_symbol`。后两个是资料不是判断,`explorer_health`
记进欠账;`rpc_save_deferred` 当场修了,因为它**不是沉默,是小谎**:

改完 RPC 一失焦,卡片下面那句提示写着"Saved as soon as you leave the field"。可核心这时
把 `rpc_save_deferred` 置了真——覆盖值**还没写**,要等 RPC 自报的 chain id 对上才写。
那几秒里屏幕替一件没发生的事打了包票。现在三态按核心的分量排:拒绝(说清它到底服务哪条链)
> 待判(`componentsUi.funding.checking` = "Checking…")> 那句常驻提示。抽成
`settings::live::override_hint`,页面只负责画——+2 个测试。

**新增语料键 0 个**,和 phase 6 一样。六句话本来就都在语料里:`addToken.errorAlreadyAdded`
/ `errorChainNotFound` 是因为 `NetWizardErrorKind` 一份服务两个调用方(核心的不变量①),
扫码路径和加代币页早就在说同样的话;`assets.rpcUnavailableSingle` 带 `{{name}}`,正好点名。

新增 9 个测试(`settings::live::wizard_tests`)。要点:`select_chain` 在 `loaded` 之前
**fail closed**,所以测试必须先把 `ReadStore` 答掉——不答的话向导测试是空跑,一条都验不到。

### 编译器早就在报的第 11 条

`cargo test` 有一条 `field \`notice\` is never read`(`flows/fixtures.rs` 的 `AddToken`)。
那个字段是 **phase 6 自己加的**:`live.rs` 从 `MtokView.save_error` 填了它,而
`panels.rs::add_token` 从来没画。**代币存不进的那句话,phase 6 送到显示模型就死在那儿了**
——和文件读不出被吃掉是同一个缺陷,只是往外挪了一层,而且唯一注意到的是一条没人看的警告。
现在画在 CTA 上方(复用 `notice_card`)。

顺带:交接里写的基线"1 个既有 warning"不实。`--tests` 下有 3 个,多出来的两个是
`AddToken.notice`(真缺陷,已修)和 `user_op.rs` 测试里的 `to_hex`(feature 关掉时没人用,
已按 feature 门住)。现在两种 feature 配置下都确实只剩 `BLE_CHANNEL_SUPPORTED` 一个。

## Phase 8 — 028 并进来,桌面那份分叉删掉

创始人 2026-09-07 点批:**就在这棵树上并**。`origin/main` = `61568f22`(PR #186)。

**冲突五处,四处是生成物**:`.specify/feature.json`(取本侧)、`rust/pkg-web/*` 与
`public/vela_core_bg.*.wasm`(两边都重建过 wasm)。按交接第 5 步重建入库:
`node rust/scripts/build-web.mjs` → 新指纹 `1b6c8ce4be03`,**3,725,860 字节**
(本分支原 3,630,664——028 的新事件与拼音表在里面);`verify-web.mjs` 46,513 例全绿,
`gen-onboarding-types.mjs --check` 25 个类型现行。唯一手并的是
`vela-core/src/lib.rs` 的一句文档注释:取 main 的措辞,因为它把两个壳都点了名。

**一个意外的好消息**:`user_op.rs` 在 main 里已经和本分支**逐字节相同**——028 Phase 8
把它当作 web 那份 TypeScript 装配的第二实现来对照。所以本 stack 在 `rust/` 下真正独有的
只剩 `dev_fixtures.rs` 和那个 feature。

**六步的结果**:

| # | 事 | 结果 |
|---|---|---|
| 1 | `contacts/live.rs` 测试字面量 | 补了三个字段——但**不是填 `Vec::new()`**:那样每行都会归到 `#`,测试照过、什么也没证。改成调核心的 `section_contacts` 现算 |
| 2 | 导入/导出改派核心事件 | 已改。`ImportFile`/`ImportAcknowledged`/`ExportRequested`/`ExportTaken`,**`executor/contact_io.rs` 565 行连测试一起删** |
| 3 | `add_group_members` 等三个新事件 | 桌面根本没有成员选择器,无处可核对;记为将来的能力 |
| 4 | `send.rs` 两条 | 都不用改:Dsd2e 的"选中 + 关闭"双发本来就是按"哪个核心都画同一个屏"写的,新核心下第二发是空操作;`prefilled_recipient` 进 `recipient` 现在是核心自己做 |
| 5 | `pkg-web` 冲突 | 见上,重建入库 |
| 6 | 分组字母归核心 | 已改,见下 |

**第 2 条是这次并树真正的理由。** 坏文件(非法 JSON、没有地址列的 CSV、空表)在 web 上被
**拒绝**,在桌面上原来是"成功导入 0 条"——从外面看和一本空通讯录一模一样。现在壳只负责
读字节和它自己的失败(文件读不出),**关于这些字节的一切判断都归核心**;拒绝优先于报告,
因为被拒的文件什么也没写,"新增 0、跳过 0"会把它描述成一次成功的空导入。

**第 6 条改了行为,不只是搬家。** 桌面原来的 `section_of` 只认 ASCII:阿豪归 `#`。核心的
`contacts_initials.rs` 逐码点拼音首字母:阿豪 → A。**同一个人在两端归到不同字母下**,
这种事没人会报但人人会注意到。顺带修掉桌面独有的一个 bug:原来按**连续段**分组,书序里
不相邻的两个 A 会变成两个 A 段;核心的目录不会。

### 并完之后的两个数字,和一条藏了很久的假绿

**desktop 263(feature on)/ 259(off)· vela-core 1,282**(并树前 267/263 · 1,264)。
桌面**少了 4**。原因说清楚:
删掉的 `executor/contact_io.rs` 带走了它自己的 **6 个测试**,本刀新增 2 个,净 −4。
那 6 个测的规则没有消失,是搬到了核心:`rust/crates/vela-core/tests/app_contacts.rs`
有 **57 个测试**,四种拒绝(`MalformedJson` / `NoAddressColumn` / `Empty` / `UnknownGroup`)
都在里面。SC-306 写的是"两个 crate 的测试数严格增加":核心侧 +18(1,264 → 1,282)是增的,
桌面侧是减的,**因为删的是一份重复实现**——如实记在这里,而不是让它看起来像退步。

闸门全绿:两端 fmt clean、clippy `-D warnings` 无话、gallery 36 态全渲染、
Windows 通过类型检查、`verify-web` 46,513 例。

**028 的暖报价撞上了 phase 6 的测试驱动。** `every_refusal_the_core_makes_reaches_the_screen`
的 pump 有个 `unreachable!` 兜底,新核心从**表单**就发一次 `EstimateFee`(`tx: None,
batch: None`——028 phase 10 的"选中就暖一次报价"),于是当场炸。处理同 15 秒竞速:
挂着不答。这条和 028 web 会话记的"FIFO 驱动遇上新计时器操作"是同一个坑的两端。

**闸门命令本身会瞒报。** 交接里(以及本文件上面)那条
`cargo test 2>&1 | tail -3 && …`,`&&` 接的是 **`tail` 的退出码**,永远是 0——
上面那次真实的测试失败,后台任务报的是 **exit 0**,我是靠读输出才发现的。
以后跑闸门要么加 `set -o pipefail`,要么别把 `cargo test` 接进管道再用 `&&` 串。

老测试 `sectioning_groups_without_reordering` 断言的正是那条桌面规则(字母按书序)。
它被**改写而不是删掉**:字母是目录(A–Z 然后 `#`),字母**之内**仍是书序(收藏优先、
最近其次)——后半句一直是对的。新增 2 个测试(拼音首字母、一个字母一段)。

## Phase 9 — 并完之后再跑一次第二条 grep

核心在脚下换过了(028 给 `send.rs` 加了 229 行、`contacts.rs` 加了 349 行),而桌面的
live 构造器是在那之前写的。所以并完树立刻重跑普查:**核心所有 View 的判断字段 vs
桌面读了什么**,7 个未读,逐条判:

| 未读字段 | 判定 |
|---|---|
| `SendReceiptView.hold_reason` | **真缺陷,关钱。已修** |
| `BalanceView.failed_chain_ids` | 有意不读——核心给了 `banner_chain_ids`(失败减限流,不变量⑦),`wallet/live.rs:608` 已注明 |
| `RpcPoolView.failed_chains` | 同上,它是余额横幅的来源,余额那台机器已经在读 |
| `FeeView.stale` | 有意不读。**核心自己写着**:"Staleness is advisory — it does not disable confirm, because today's UI does not either",真正的门在提交侧(`tempo_quote_is_stale`、中继的 in-band gate)。要做刷新控件得先有图 |
| `SignFundingView.denial_reason` | B 组,桌面还没有请求来源 |
| `PaymentRequestView.can_copy` / `can_save` | **要创始人定**,见下 |

### 修的那个:收据不说它为什么停着

核心把收据的"停"分成两种(`SendHoldReason`),桌面一种都没读:

- **`FeeHold`** — 手续费涨过了你批准的数,**交易排着队,费用回落会自动发出去**。
  状态仍是 `Submitted`,所以屏幕说的是普通的"等待确认"。那不是同一件事:
  人盯着一笔可能很久不动的转账,屏幕上没有一个字解释。
- **`FeeRejected`** — 费用一直没回落,**什么都没发出去**,出路是按当前费用重发,
  不是重试同一笔。屏幕原来只给一句通用错误。

**新增语料键 0 个**——`send.txHeldFees` 和 `send.txRejectedFees` 这两句话一直在语料里,
一字不差,**而且 web 也没读**(`hold_reason` 在 app-web 只出现在一个测试夹具里)。
这条是跨端的:核心算了,两个壳都没说。+2 测试,基线 **265 / 261**。

> **给 web 那边的人**(不是我的范围,但漏在同一处):
> `app-web/vela-wallet/src/lib/flows/live-send.ts:572` 的 `submitted` 分支写死
> `captions: [m['send.txWaitingConfirm']]`,`failed` 分支同理没有 hold 的位置。
> `hold_reason` 在 app-web 只出现在 `live-send.test.ts:270` 的夹具里(`null`)。
> 两句语料键:`send.txHeldFees`、`send.txRejectedFees`。
> 注意两个标志不互斥,别把两句合成一句(下面那段)。

**改完自己又抓到一个边**:两个模型标志**不互斥**——先 `FeeHold` 后失败的收据,
`fee_held` 还留着。第一版我把 `hold_reason` 当成一句话往两个分支里塞,
于是"排着队、费用回落会自动发出去"有可能印在一张**失败**的收据下面。
现在每个分支只认自己那个原因,并且有一条测试专门盯着这个组合。
**核心把两件事分开了,壳就不能把它们合成一句。**

### 要创始人定的:收款页的确认门

`PaymentRequestView` 的 `can_copy` / `can_save` 都等于 `acknowledged`——人得先确认过
一个提示,才允许复制/保存收款请求。桌面的收款页是**活的**(它已经在读 `payment_request`
决定二维码内容),但这两个字段和 `acknowledged`、`gate_loading` 都没读,等于门是开的。

**没有自作主张给它加门**:记忆里 Receive 的链上门(issue #14)是 2026-07-03 被判为
过时关掉的(passkey 才是信任根)。这个门是不是同一件事、还该不该有,是产品判断,
不是接线判断。

## Phase 10 — 那张从来没被打开过的表

欠账第 1 条的后半句:"没提交 xlsx 样张"。查下来比缺个样张更糟——**workbook 那条路
一次都没在测试里跑过**。`executor/batch.rs` 的四个测试测的是:扩展名判断、单元格格式化、
一个"不是 zip 的假 xlsx"、以及 CSV。`calamine::open_workbook_auto` → `worksheet_range_at(0)`
这一句,也就是真正读表的那句,从来没执行过。

"从 Excel 把工资表拿进来"是**花钱的路**,以前每一个测它的测试用的都是测试里自己敲的文本。

现在提交了一张真表 `app-desktop/vela-wallet/tests/fixtures/payroll-sample.xlsx`
(1,737 字节,手工装配的 OOXML:五个 part,inlineStr + 数字单元格),故意做了三件事:

| 表里有什么 | 想钉住什么 |
|---|---|
| `5000`(整数) | 表格眼里是数字,人眼里是 `5000`——不能变成 `5000.0` |
| `173.88` | 上面那条去 `.0` 不能变成无差别截断 |
| 只有一个单元格的**短行** | 读的时候要补齐到最宽;不补,下一行的值会滑进别人的金额列 |

两个测试:一个在 `read_table` 旁边,证明 calamine 把文件读成了那个矩阵;一个在
`wallet/money.rs`,把**同一个文件**经 `PickFile` 喂进 `BatchImport` 一路走到付款行——
表头不算行、短行进 `rejected` 计数(不是悄悄丢掉)、合计 `5173.88`、
Apply 之后两行金额 `5000` 和 `173.88` 一位不差。

**"真表格实机点一次"仍然欠着**——那要人去点文件对话框。但现在它不是唯一的证据了。

### 顺手:浏览器端点的探针结果(欠账 10b)

网络卡片的浏览器地址栏和 RPC 地址栏用的是**同一个组件、同一个徽章位**,RPC 传了
`rpc_badge`,浏览器传的是 `None`。核心两个都探了,`explorer_health` 就这么扔了——
量到延迟的浏览器和根本没探过的长得一模一样。改成传 `explorer_badge`,一行,
用的是已经有的 `probe_badge`,没有新画面。

## Phase 11 — Windows 的日界线,以及怎么验一段编译不了的代码

031 留的第 5 件:`GetTimeZoneInformation` 没接,`local_utc_offset_seconds()` 在
非 unix 上直接返回 0。后果不小:**Windows 上活动列表按 UTC 分日**,晚上的转账归到明天,
"每天有一段时间,谁的列表都在说错话"。

**这段代码在这台机器上编译不了。** 桌面 app 的依赖树要编 C(ThorVG、resvg、hidapi),
`--target x86_64-pc-windows-gnu` 会死在 build script 里,连 Rust 都到不了——
这正是 `check-windows.sh` 只检一个独立小 crate 的原因。所以验证分三层:

1. **算术单独拆出来、不带 `unsafe`、每个平台都编都测**。`windows_offset_seconds`
   是纯函数,4 个测试在本机跑:
   - **符号**:Win32 定义 `UTC = local + bias`,所以偏移是 bias 取反。柏林冬天 bias −60 → +3600。
     搞反了柏林就成 UTC−1、纽约成 UTC+5。
   - **季节**:DAYLIGHT 要用 `DaylightBias`。夏天误用 `StandardBias` 就差一小时,
     而且差得"看着很合理"。
   - **半小时区**:印度 UNKNOWN + bias −330 → +19800,整点假设会丢掉它。
   - **调用失败**:`TIME_ZONE_ID_INVALID` 时结构体根本没填,必须**退回 UTC 而不是拿垃圾算**
     ——错得没规律比错得有规律更糟。
2. **FFI 那几行,原样抬进一个隔离 crate 交叉编译**(`x86_64-pc-windows-gnu`,
   `clippy -D warnings` 也过)。这一步当场抓到:**windows-sys 0.59 只导出
   `TIME_ZONE_ID_INVALID`**,另外三个 id 不存在,所以按值匹配、并对导出的那个下
   `const _: () = assert!(… == u32::MAX)`——将来哪个版本改了编号会编译失败,
   而不是悄悄挪掉所有人的日界线。
3. **依赖连线**:`cargo tree --target x86_64-pc-windows-gnu -i windows-sys@0.59.0`
   确实显示 `vela-wallet` 这条边。Cargo.toml 第 150 行那段警告是认真的
   ——这个 crate 就曾经被写进 macOS 的 target 段里、Windows 路径整个没链上而闸门全绿。

顺手把 cfg 从 `not(unix)` 收紧成 `windows`:函数体现在依赖 `windows-sys`,而它只在
`cfg(windows)` 下存在;既不是 unix 又不是 windows 的目标现在会**找不到这个函数**,
比再默默按 UTC 分一次日要好。

**没验的那一层写在这里**:没有在 Windows 上跑过。编译、clippy、算术都过了,
行为没有。和 `check-windows.sh` 自己的说明是同一句话。

### 把那次验证做成闸门,而不是一句记录

上面第 2 层本来是我在临时目录里手工做的一次性动作。现在写进 `scripts/check-windows.sh`:
它把 `src/executor/mod.rs` 里那两个函数**原样抬**进一个临时 crate(没有 C),
交叉编译 + `clippy -D warnings`。抬取是**故意死板的文本匹配**,函数被改名就大声失败,
而不是悄悄什么都没检。

**并且验过它会失败**:把 `info.StandardBias` 改成 `info.StandrdBias`,脚本以 1 退出、
指着那个字段报错;改回来就绿。一条不会失败的闸门比没有闸门更糟,这是本刀
"管道吃掉退出码"那条的同一个教训。

> **闸门的第二个坑**(第一个是管道吃退出码):Bash 工具的**工作目录会留在上一条命令**。
> 我在隔离 crate 里 `cd` 过一次,下一条闸门就在**那个目录**跑了 `cargo fmt --all --check`,
> 报的是隔离 crate 的格式问题。闸门命令自己带上 `cd`,别指望继承。

## Phase 12 — 余额一条一条地到

031 留的第 4 件,也是欠账表里最后一件**纯工时**的事。核心一直支持流式:

```text
AccountChanged ─► reset ─► ReadBalanceCache ∥ FetchTokens ──► stream:
    ChainAssetsArrived (merge per chain, slow chains keep last value)
```

桌面一直没用。`FetchTokens` 是一次 `Answer::Blocking`:十二条链十二个线程一起跑,
然后 **join 完再一次性回答**。后果是——**一条 RPC 不通,整个英雄区就按它的超时僵着**,
另外十一条早就答完了,屏幕上却还是骨架(或者昨天的缓存总额)。

### 壳缺一条缝

`Answer` 只有 `Now` / `Blocking` / `After` 三种,都是**答一次**。流式要的是"边做边说",
所以加了第四种:

```rust
Streaming(Box<dyn FnOnce(&Sink<E>) -> T + Send>)
```

`Sink<E>` 可 Clone、可跨线程(十二个线程共用一个),每次 `send` 变成一个**事件**,
在主线程按顺序 dispatch 进这台机器,**全部在它自己的结果之前**。这条顺序就是全部契约:
`balance_dashboard` 把每次到达并进 token 列表、只结算一次,**结算之后再来的快照会把
已经算过的链复活**。

实现上顺序是自然保证的,不是靠小心:排空循环在 sink 被丢弃时结束,而 sink 是在
`work` 返回时丢的。写了一个不带 gpui 的测试盯着这条(12 个事件按序、然后才是结果),
外加一条"接收端没了 send 不能炸"——窗口在取余额途中被关掉,十一条链手里还攥着 sink。

`Answer` 从 `Answer<T>` 变成 `Answer<T, E>`,十个 `Machine::perform` 签名跟着改成
`Answer<XShellResult, Self::Event>`(编译器一个个指出来的)。`wallet/money.rs` 那个
**屏幕自己拥有的** fee 泵也补了同一条臂——`fee_policy` 今天不流式,但写成 `unreachable!`
的话,它哪天开始流式就是确认页上的一次 panic,而这不过是资深泵里同样的八行。

### 取数那半边

`fetch_all` 拆成 `fetch_all_streaming(address, &Arc<ChainSink>)`,每条链的线程算完
**自己那条链的 token 就立刻报**;`fetch_all` 就是传一个什么都不做的 sink,所以账户切换器
和现有测试一行没动。快照必须是"那条链的",因为核心的合并规则是**按 chain_id 替换、
其余保留**;空快照(那条链什么都没有)正确地什么也不改。

`FetchAccountAssets` **故意不流式**:它读的是切换器里别人的账户,快照会被并进当前账户。
核心的文档也是这么写的("never streams")。

结算仍然只有一次:核心要完整图景才能决定写不写缓存、哪些链算失败。

### 证据

**五个**同步测试驱动器(它们自己在测试里跑泵)也补了这条臂,共用一个
`resident::run_streaming` 而不是各写一遍。第五个只在 `--features dev-fixtures` 下编译,
所以第一遍闸门才发现它——**闸门要跑两种 feature 配置,不是一种**。真网那条 `#[ignore]` 测试:金标 Safe 上
**报告不止一份**、**其中一份在结算之前就已经把钱和一个能画的总额交到核心手里**、
结算后的持仓不少于那一份。

**第一版这条测试写错了,而且是真网跑出来打脸的**:我断言"**第一份**报告就带着钱"。
金标 Safe 只在 Gnosis 上有 xDAI,**十二条链里十一条正确地报了空快照**,
先答完的几乎必然是空的那些。断言改成"**存在**一份结算前的报告带着钱"——
这才是这个功能的主张。多亏跑了真网,不然这条测试会一直是个假的规格。

顺带清了三条自己带进来的警告(`#[must_use]` 落在类型别名上、`try_next` 已弃用、
测试模块里多余的 `StreamExt`)——第 6 条教训的现场复习:两种配置各只剩
`BLE_CHANNEL_SUPPORTED` 一个。

**这条测试证明什么、不证明什么**(先写清楚,免得名字比内容大):它证明报告存在、
每条链报自己那份、第一份在结算之前就有可画的总额。它**不**证明墙钟意义上的"更早"
——同步驱动器 `run_streaming` 保的是**顺序**,不是并发(它自己的文档就这么写)。
真正的时间性归 async 泵,而这个仓库没有 gpui 测试夹具能驱动它。**这一层没测。**

## Phase 13 — 桌面的 web 引擎:选型,和一次跑通的探针

创始人问"接哪家、你会不会接"。没有列表格,直接接了一个跑起来。

### 决定性的事实(都是查出来的,不是记得的)

| 事实 | 出处 |
|---|---|
| `gpui::Window` **实现 `HasWindowHandle`** | `crates/gpui/src/window.rs:6390` |
| macOS 交出来的是 **`AppKitWindowHandle`(NSView)** | `gpui_macos/src/window.rs:1918-1921` |
| Windows 交出来的是 **`Win32WindowHandle`(HWND)** | `gpui_windows/src/window.rs:581` |
| gpui 钉 `raw-window-handle = "0.6"` | Zed 根 `Cargo.toml:758` |
| wry 0.56.1 也钉 `raw-window-handle = "0.6"` | wry `Cargo.toml:152` |
| `build_as_child<W: HasWindowHandle>` | wry `src/lib.rs:1571` |

**两边在同一个版本的 `raw-window-handle` 上碰头**,所以句柄类型是同一个类型
——这正是本仓库 Cargo.toml 里那条"两个 rwh 版本会变成两个类型"的警告说的事,
这次是它成立的一面。

### 探针:`VELA_WEBVIEW=1`

`main.rs` 在开窗时把一个真 webview 挂成子视图,注入一段脚本,脚本回调 IPC。跑出来两行:

```
[vela-wallet] webview: attached as a child of the gpui window
[vela-wallet] webview ipc: vela:probe
```

第二行才是重点:**注入的脚本在页面里执行了,并且通过 IPC 通道说回来了**——
`window.ethereum` 要的那条缝是通的,不只是"画出了像素"。截图确认页面可见。

### 三个必须先说清楚的代价

1. **合成在 gpui 之上,不在里面。** 原生子视图就是这样。
   **我第一版把这条写成"签名面板要盖在浏览器上"——写错了**,创始人当场纠正:
   桌面的签名面板是**第三列**(`PanelId::Signing` → `panel_scaffold`,和收款、
   资产详情同一个脚手架),它挨着浏览器、把浏览器挤窄,不盖在上面。手机版
   clearsigning 那些图看起来像盖上去,是因为手机只有一列。
   这条约束真正落在两个地方:**离开浏览器时**(原生子视图不会因为 gpui 换了路由
   就消失,必须显式藏)、和**居中弹窗**(扫码、设置对话框会被画在 webview 底下)。
2. **Linux 是另一件事。** wry 在 Linux 上 `os-webview` 拉 gtk + webkit2gtk + soup3,
   而 CI 的 `desktop` job 和 `desktop-linux-packages.yml` 都没装这些;而且
   `build_as_child` 在 Linux **只支持 X11、不支持 Wayland**,还要 `gtk::init` +
   在 gpui 的循环旁边推 GTK 的循环,而 gpui 有 Wayland 就走 Wayland。
   所以本刀把 wry 放在 `[target.'cfg(not(target_os = "linux"))'.dependencies]`——
   **不是忘了 Linux,是把它记成一个单独的决定**:桌面浏览器先只上 macOS + Windows,
   Linux 明说"暂不支持",还是为 Linux 单开一个顶层窗口(要 GTK 双循环,脆)。
3. **objc2 会有两份。** wry 要 0.6.4,本 crate 为了跟 gpui 一致钉 0.5。两份能共存,
   因为跨过去的只有一个 rwh 裸指针,没有 objc2 类型。编译时间会长一点。

clippy 警告数 **42 → 42**(基线也是 42,用 stash 量过),没有新增。

### 没做的

探针就是探针:固定 bounds、一段内联 HTML。真正的 C 组还要按列的布局跟随 bounds、
导航/前进后退、per-site 权限、把 027 的 `inpage.js`/`protocol.js` 接到
`with_initialization_script` + `with_ipc_handler` 上,再驱动 `dapp_session`
`dapp_permissions` `browser_history` 三台机器(3,771 行)。

## Phase 14 — 浏览器成了一列(C 组的壳)

创始人点批"先 A 后 B":先把浏览器做实,B 组的签名请求才有来源。

**真 Uniswap 现在跑在 explore 那一列里**(截图为证:app.uniswap.org 的
"Swap anytime, anywhere." + 真兑换组件,上面是 Vela 自己的标签条和工具栏,
左边是侧边栏)。

### 位置跟着列走

webview 的 bounds 由**拥有那块矩形的元素在 paint 阶段**给出(`gpui::canvas`),
所以窗口缩放、第三列(签名面板)打开挤窄浏览器,它都跟着走。只有 bounds 真的变了
才跨平台边界调 `set_bounds`——不然一秒六十次。

### 那条约束的正确形状

我 phase 13 把它写成"签名面板要盖在浏览器上",**是错的**,创始人纠正了:
桌面签名面板是第三列,和收款/资产详情同一个 `panel_scaffold`,挨着而不是压着。
真正要管的是两处:

1. **离开浏览器**。原生子视图不会因为 gpui 换路由就消失。所以每一帧只要不是在画
   浏览器列,就 `webview::hide()`——漏了这一句,webview 会浮在钱包上面。
2. **居中弹窗**(扫码、设置对话框)会被画在 webview 底下。这两个还没处理,记账。

### provider:027 的脚本原样注入

`inpage.js` **一个字没改**地 `include_str!` 进来(434 行,扩展里那份)。扩展是
MAIN world + isolated world 用 `window.postMessage` 对话,wry 没有 isolated world,
所以补了**十一行 bridge**:把同样的信封转给 `window.ipc`,答案再用 `window.postMessage`
送回去。provider 分辨不出区别,这正是重点——第二份 EIP-1193 实现就是第二套 bug。

**origin 由宿主读,不信页面。** 扩展的 content script 存在的理由就是"带两个页面伪造不了的
事实:哪个标签页、哪个 origin"。这里同样:origin 从 `webview.url()` 读,
页面在信封里自称的 origin 一律忽略。

**请求现在被拒绝而不是被吊着**:`dapp_session` 三台机器还没接,所以答 4900
(不是 4001——027 D37:永不结算的 promise 是这条通路最坏的产出,而"干净的拒绝"
和"提交了但卡住"必须能分辨)。

### 顺手修的真 bug

收藏格子原来**每一个都只是把 `browsing` 置真**,页面画同一张 mock——点 Aave 出 Uniswap。
现在每个格子带着自己的 host 去 `navigate`。这个 bug 在 mock 时代看不出来,页面一真就是错的。

### 还欠

地址栏是画的静态 host(图就是这么画的,没自作主张改成可编辑);标签条还是 mock;
逐站点权限、历史、`dapp_session`/`dapp_permissions`/`browser_history` 三台机器(3,771 行)
都还没接。Linux 依然在 `cfg(not(target_os = "linux"))` 外面——**创始人已定:Linux 要支持,
但先上 mac + win**。

## Phase 15 — B 组开工:第一台机器(approval_guard)

签名面板三台机器,9,362 行核心。**这是多刀的活,不是一刀**。本 phase 落第一台,
把模式立住。

`approval_guard` 只问三个 `eth_call`,但三个都是关于同一件事:**人到底同意让合约动多少**。

| 操作 | 为什么它关钱 |
|---|---|
| `ReadTokenMetadata` | 没有 decimals,授权额度就渲染在错误的数量级上。`1000000` 是一千个还是一个,只由这一个调用决定 |
| `ReadErc20Allowance` | `increaseAllowance` 是**加**不是**换**。结果总额 = 已有 + 增量;只显示增量就低估了人正在同意的东西 |
| `ReadErc20Balance` | 「永不无限额」要给人一个能填的数,自己的余额是他能推理的那个(issue #86) |

核心把失败分了级,壳不能抹平:`None` 元数据是"整批读失败",而某个代币**不在**列表里
是"这个解析不出来"——两者的兜底不同。批次回来长度不对时我返回 `None` 而不是空列表,
就是这条。

**新增 `abi::enc_allowance`**(`allowance(address,address)`,选择器 `dd62ed3e`)。

### 一个只有真网能抓到的错

第一版 `eth_call` 写成 `pool::call(...).ok()?.as_str()`——**`pool::call` 答的是整个
JSON-RPC 信封,不是 result**。把信封当字符串读,一个完全正常的调用会静静地答 `None`,
于是三个好读变成"整批失败"。真网测试当场炸出来;离线测试永远看不到,因为它根本不发请求。
房子里现成的写法(`manage_tokens::eth_call`)是 `.get("result")`,照抄就对——
**这就是"先看隔壁怎么写"比"自己想当然"便宜的地方**。

两个测试:一条离线(**没有代币要读 ≠ 读失败**,必须答 `Some(vec![])` 且不发请求),
一条 `#[ignore]` 真网(Gnosis 上的 USDC.e:符号有、6 位小数、余额读得到)。

## Phase 16 — B 组第二台:clear_signing 的读

三台里最大的一台(4,841 行核心),五个操作。这一层的分工比别处更要紧,
因为**降级阶梯的每一级都是"知道多少"的判断**,壳一旦替它判断,级就悄悄塌了:

- 404 的描述符 ≠ 解析失败的描述符
- **revert 的 `eth_call` ≠ 够不着的 `eth_call`**。前者是真答案("这不是 ERC-721"),
  后者是"我们没能问到"。掉不掉一级,取决于分得清
- 没人认识的选择器 ≠ 我们忘了去查的选择器

所以答案原样回去:body-或-`None`、result-或-`None` **外加一个独立的 `rpc_error` 标志**、
查不到就 `[]`。级由核心挑。

| 操作 | 做法 |
|---|---|
| `HttpGet` | 5s 预算(`NET_TIMEOUTS.descriptor`),200 才给 body。404/超时/断网都答 `None`——对核心是同一件事 |
| `RpcEthCall` | 走 pool,**信封里有 `error` ⇒ `rpc_error: true`**(这次没再踩 phase 15 那个信封坑) |
| `SelectorDbLookup` | 三个库**并发问再合并**(不是竞速):Sourcify 4byte、OpenChain、4byte.directory。openchain 形状的两个过滤过垃圾所以排前,4byte.directory 补缺口并按 id 升序(最小 id 是规范签名)。进程内缓存 |
| `Timer` | `Answer::After` 用 gpui 的计时器,不是停一个线程——一次解析要问三个问题,那就是三个白等的线程 |
| `Now` | `Answer::Now` |

`chain_tokens::data_base()` 改成 `pub(crate)` 共用:描述符和代币索引必须从**同一个**
配置端点拿,不然设置页改了地址只有一个调用方跟着变。

两个测试:一条离线(**什么算选择器**:大小写、可选 `0x`、整段 calldata 取前四字节、
太短/非十六进制都不问),一条 `#[ignore]` 真网(`transfer(address,uint256)` 必须在候选里
——它要是找不到了,通用解码那一级就没了,每一笔不认识的转账都掉到盲签;
外加第二次调用必须走缓存,不然签名面板每敲一个键就锤三个公共服务)。

## Phase 17 — B 组第三台:sign_request(会花钱的那条)

七个操作。**这是这个 app 里第二条会把钱送出去的路径**,所以先说清楚它复用了什么、
拒绝发明什么。

### 复用,不是重写

`SignAndSubmit` 就是 032 给发送流写的那条 `user_op::submit`(passkey → 装配 → 提交),
**故意是同一条**:dApp 的交易和人自己的转账要由一份实现装配、定价、签名,
否则"显示会发生什么的那张单"和"真正让它发生的代码"就是两个意见。
`SignContext` 也不是抄一份 `SendContext`,而是**调用它**再取字段——
同一个账户该用哪种仪式,两个答案就是一个钱包在这屏要手机、那屏要安全钥匙。

### 报两次,差别要紧

`SignAndSubmit` 用的是 phase 12 那条 `Answer::Streaming` 缝:

- **中途**把中继接受的 `user_op_hash` 通过 `Event::OpSubmitted` 送回核心。
  **在等收据之前**——等待期间窗口被关掉,重开时也得知道这笔提交过,
  否则一笔已提交的交易看起来像从没发生过。
- **一次**最终结果。交易给的是**真 tx hash**(等到收据),因为
  `eth_sendTransaction` 就该 resolve 成 tx hash;**userOpHash 不是 tx hash**,
  dApp 拿它去查会永远查不到。

收据等 90 秒封顶。超时给 userOpHash 而不是错误:**dApp 的 promise 必须结算**,
而"卡住但已提交"和"干净的失败"必须分得开(027 D37 的双花风险)。

### 拒绝发明的两个数

- **`funding` 给 `None`**。中继的 `account_info` 有存款地址和余额,但**没有
  threshold / recommended**——那是**策略数字**,而一个凭空编数字的充值屏会让人打错金额。
  核心对"凑不出 funding"的既定兜底就是普通失败,那是诚实的。
- **赞助给 `Denied { reason: None }`**,不是 `Funded`:桌面没有赞助通路,
  `Funded` 会是"别人替你付了"的谎。

`SubmitFailure` 本来就是**类型化**的枚举,所以这里一处字符串匹配都没有——
核心文档警告的那层正则(`parseBundlerUnderfunded`、`PasskeyErrorCode.CANCELLED`)
032 的发送路径已经付过一次,不付第二次。

### 四个测试,和我自己漏的那个洞

params → calls 是纯函数,离线测得到,而它正是"数字错了就是金额错了"的地方:
十六进制上线、**十进制进核心**;缺 value 是 0、缺 data 是 `0x`(大多数合约调用不带钱);
批量保序(重排过的批量是另一笔交易)。

第四条测"读不出来就拒绝,而不是提交一个空批量"——**写完才发现我自己留了这个洞**:
`wallet_sendCalls` 的 `calls: []` 原本会答 `Some(vec![])`,那会装配出一笔什么都不做、
却照样收费的 user operation。检查放在映射**之后**,因为"每一条都读不出来"和
"本来就是空的"产出同一个空向量。

### 还没接

模块整体挂着 `#[allow(dead_code, reason = "wired by the signing panel's host, phase 18")]`
——和 030→031 之间的 `pool::call` 同一个状态,标出来而不是让警告数失去意义。
**host 落地时必须摘掉**:allow 会连带把被调用者标活(第 1 条教训),挂着的时候
这个模块内部的任何死代码都看不见。

`persist_record` / `update_record` 目前是空实现:dApp 历史记录要落到发送流写的同一个
交易存储里(这样一笔 dApp 签名和一笔转账在历史里长得一样),那要先确定键与形状。

## Phase 18 — 宿主:四台机器跑成一列

web 的签名单读**四个视图**(`sign` / `clear` / `guard` / `fee`),所以桌面的宿主也持四台。
形状照 `SendHost` 抄——**发送列已经证明了"跨机器的一段旅程要一个主人、一台机器一个泵"**,
而它踩过四次的相关性规则(第 2 条教训:fee 会话必须只有一个)不值得重新发现一遍。

**三台机器分别被告知,谁也不等谁**:解码是一次网络往返,授权编辑器不是。
等最慢的那个才开单,就是每次取描述符时人盯着一张白单。

**关闭由核心说了算**:`SignSurface::Hidden` 才是"这个请求结束了",不是这个文件
对某次点击的解释。

### 谁回答 dApp

`SendResponse` 在执行器里答 `Screen`,因为**只有这一层知道请求是从哪条通路来的**。
今天只有一条(浏览器列),所以答案走 `webview::respond`;将来有两条时,
核心一直带着的 `transport_id` 就是用来选的——那个 id 存在的理由正是
**一个响应绝不能发给另一个站点**。

`webview::respond` 把核心的判决翻成 provider 在等的信封。**错误码是核心的**
——4001 是拒绝、4900 是卡住但已提交——壳自己挑码就可能把"拒绝"报成"失败",
而 dApp 对这两件事的处理不一样。

### 两个翻译,两条测试

宿主要把请求翻给解码器,这两处翻错都不会崩,只会**悄悄降级**:

- **批量从第一条腿解码**(和手机单一样)。`value` 缺就是缺,不能变成 0。
- **`eth_signTypedData_v4` 的文档是第二个参数**。读第一个就是把地址喂给解码器,
  于是每一个 typed 请求都掉到盲签那一级——没人会当成 bug 看,只会觉得
  "清晰签名在这儿从来没生效过"。顺带处理了有些站点把文档当对象而不是字符串传。

### 差最后一跳

宿主还没有人构造:**浏览器的 ipc handler 要够到 page 才能开一个**,
而那需要在 wry 的回调里拿到 gpui 的句柄。这一跳我没在长会话的尾巴上赶——
它是"哪个页面、哪个窗口、什么时候通知"的三岔口,赶出来的版本会是下一次
"看着能跑但少通知一次"的来源。

模块挂 `#[allow(dead_code, reason = "opened by the browser's request hop, phase 19")]`,
和 phase 17 同一个规矩:**接上时必须摘**,因为 allow 会连带把被调用者标活。

## Phase 19 — 最后一跳,和一个"什么都不报"的失败

浏览器的 ipc handler 现在够得到 page,dApp 的请求真的走进机器了。全程实测:

```
browser rpc: eth_chainId from http://127.0.0.1:8137/
browser answer: {"dir":"res","id":"…:1","error":{"code":4900,…}}
browser rpc: eth_accounts …
browser answer: {"dir":"res","id":"…:2",…}
```

页面 → provider → bridge → ipc → sink → page → 判决 → deliver → provider → promise 结算,
id 逐个对上。

### 这一跳为什么要延后一拍

wry 从平台回调里调 ipc handler,而 `AsyncApp::update` 会 **borrow 那个 app cell**;
在另一次 borrow 里面同步这么干,在钱包里就是一次 panic。所以 sink 走
`AsyncApp::spawn` 落到前台执行器,活儿在下一个 runloop 轮次里做。

### 谁决定开不开那一列

**核心。** 我第一版写成"来请求就 `panel = Signing`"——那会让页面只是问一句
"现在是哪条链"就给人推一张签名单。改成:开完机器看 `SignSurface`,
`Hidden` 就什么都不显示。dApp 发的大多数东西人根本不该看见。

### 那个什么都不报的失败

接完第一版,**一个请求都没到**。原因:`inpage.js` 第 29 行是
`import { CHANNEL, … } from './lib/protocol.js'` —— 它是 **ES 模块**,
而 initialization script 是 classic。原样注入就是**语法错误**:
文件根本没跑、`window.ethereum` 从来没出现、于是每一个请求都"静静地从未发生"。
**没有任何东西报告这件事**:宿主收不到错误,页面只是没有钱包。

修法是把 `protocol.js` 去掉 `export` 前缀、`inpage.js` 去掉那一行 `import`,
拼进一个 IIFE。**磁盘上两个文件一个字节都没动**,两个模块关键字是在 Rust 里去掉的。
没选"自定义协议 + 动态 `import()`"是因为**严格 CSP 的 dApp 可以拒绝它**,
而"在某些站点能用"的 provider 比哪儿都不能用更糟。

两条测试盯着这里:注入的脚本里**不许再有 import/export**(哪个文件再长出一个就大声失败),
以及**它还得是真的那个 provider**(channel 常量、EIP-6963 公告、两个文件的长度)
——一个悄悄拼出空字符串的实现能完美通过前一条。

### 读方法暂时被拒绝,不是被回答

`sign_request` 只管**签名方法**;`eth_chainId` / `eth_accounts` 是读和权限,
归 `dapp_session` / `dapp_permissions`(C 组,没接)。所以 page 按方法分流:
签名的进机器,其余**答 4900**。

**不是 4001**:人没有拒绝,而一个把"拒绝"读出来的 dApp 会告诉他们"你拒绝了某件
你从没看见的事"。也**没有让壳自己回答**读方法——那是壳替核心做决定。

### allow 摘掉了

phase 17/18 挂的两个 `allow(dead_code)` 都拿掉了,警告数仍是 **42**——
这就是接线是真的的证明:allow 一摘,编译器在这三个执行器和宿主里找不出一个死项
(第 1 条教训说的正是 allow 会连带把被调用者标活)。

### 还欠

签名列现在开得起来,但**画的还是 fixture**:核心视图 → 已画好的 block 渲染器
那个 live 构造器还没写。C 组三台机器(读、权限、历史)没接,读方法因此被拒。

## Phase 20 — 签名单读核心,不再读手写场景

`signing/live.rs` 是 `fixtures.rs` 的**兄弟,不是替代**:两边都产出 `SigningModel`,
面板挑谁喂它。这正是让 33 个画好的场景在真请求到来后**仍然可评审**的东西,
也让"画廊没变"成为 diff 能证明的事。

### 滑块要三台机器一起点头

`SignView.confirm_gate_open` 的文档自己写着:"the shell must AND it with
`GuardView.confirm_allowed` and `FeeView.confirm_fee_ready`"。
少任何一台,都是**一次没人同意过的签名**:
少了 fee 的,是在没人拿到过的价格上开滑块;少了 guard 的,是在没人设过上限的
无限额授权上开滑块。测试逐个把三台按掉,每次都必须关。

### 警告按"最坏先读"排

单子是从上往下读的,所以顺序不是风格:
`to_own_token`(把代币转给它自己的合约 = 不可逆销毁)排第一,
然后 `best_effort`(4byte 恢复的,是"解析通过的猜测",不是谁发布的描述符)、
`partial`(描述符声明的字段比解析出来的多)、`unverified`(小数没人验证 = 数量级没人验证)、
`expired`。**每一条都是核心已经算好的旗标**,壳只是把它说出来。

`detail` 字段**不进摘要**——它们是 Advanced 的。提上来就是把"这笔在干什么"
埋进"它用什么参数干的"底下。

### 没解码出来就不画

`result` 是 `None` 时返回空,而不是一个空的 intent——空 intent 读起来像
"这笔什么也不做"。

### 费用只有一个格式化器

复用 `flows::live::fee_text`(改成 `pub(crate)`)。**两个格式化器就是两个关于
"这笔要花多少"的答案**,而且是在两块给同一个操作定价的屏幕上。
未定价时渲染成它的 `—` 而不是消失:**没有那一行读起来像"免费"**。

### 第四台机器还没跑,而这件事是明说的

宿主持 `fee_view`,但那是一台**pristine 的** `fee_policy`——它答
`confirm_fee_ready: false`,所以这条缝没接上之前**滑块一直是关的**。
这是对的失败(三方 AND 存在的理由就是不让人在没有价格时确认),
而它是**held 的一个视图**、不是每帧新建一个:等 fee 会话落地时,
只有一个地方要接上,也只能有一个(第 2 条教训:fee 会话必须只有一个)。

四条测试:三方 AND(逐个按掉)、Advanced 字段不进摘要、每个旗标都变成一条警告
且最坏的排第一、没解码就不画。

## Phase 21 — fee 会话:滑块能开了

B 组最后一块。宿主现在持第四台机器 `fee_policy`,**一个请求一个会话**。

### 报价从哪来

`sign_request` **没有** `EstimateFee` 操作——签名单自己驱动 `fee_policy`:
请求打开时读一次部署状态(后台),然后 `QuoteRequested`。
只有交易报价:`personal_sign` 不花钱,给它挂一条网络费,是在一个从不碰链的签名上
写一个费用。

**部署状态读不出来就不报价**。猜"已部署"会发出一个没有 initCode 的操作,
猜"未部署"会给一个活账户挂上 initCode——两种猜法算出的费用都是**另一笔操作的费用**。
所以宁可没有报价:滑块保持关闭,这正是 `confirm_fee_ready: false` 的意思。

### 签下去的价 = 屏幕上的价

`approve()` 的报价是从 **`fee_view` 读的**——**渲染确认卡的那同一个视图**——
而不是重新问一次。重新问就是第二个数字,于是**人同意的那个数字和被签下去的那个数字
不是一个**。这就是第 2 条教训在桌面上的样子(web 记了四次因为拆成两个对象而失败的集成)。

两条测试:被签的报价必须逐字段等于被显示的那个;没定价的单子**带 `None` 而不是 0**
——0 是一个费用主张,而提交会把它签下去。(后者理论上到不了,因为滑块是关的。)

### 滑块现在真的会响

`slide_to_confirm` 多了一个 `action`。**只在三台机器都点头时才传**:
一个关着却仍然挂着动作的滑块,是一个核心说了不、却等着一次点击说是的控件。
mock 传 `None`——画着但不答应任何事,画廊因此一个像素没变。

**这套词汇里没有拒绝按钮**:关掉这一列就是拒绝。所以这个控件唯一能做的事就是确认。

## Phase 22 — 让一个真请求走到面板上,三个只有跑起来才看得见的错

不是点真 dApp(那要点 Connect,而合成点击要 TCC 授权),而是让一个本地页面直接发一笔
**真的 `eth_sendTransaction`**:Gnosis 上一笔 ERC-20 转账,`transfer(address,uint256)`
的真 calldata。通路都通了,问题是**解码出来的东西对不对**。

### 第一次:`rejected: 4902`

页面拿到的是 CHAIN_NOT_ADDED。原因:**宿主开出来的机器对这个钱包一无所知**——
我从没告诉过 `sign_request` 有哪些网络、有哪些账户。它默认一条链都没有,
于是**每一笔交易都被拒**。

修法:`begin()` 里先发 `NetworksChanged`(内置链 + 用户加的,和设置页同一份名单
——设置说在、签名说不在,是钱包在跟自己吵架)和 `AccountsChanged`,再发 `RequestArrived`。

**没有测试能抓到这个**,因为要点在于"机器没被告知什么"。

### 第二次:面板开了,但它说的是 mock 的话

单子画出来了,内容是对的(Send、数量 1、收款方 `0x031d7d…772b`、真费用 0.01 xDAI),
但**抬头写着 "Uniswap / app.uniswap.org"**——请求其实来自 `127.0.0.1:8137`。
我的 live 构造器只换了 blocks / fee / confirm,**dapp 身份和网络徽章还是 fixture 的**。

**签名屏上认错请求方,是它能犯的最严重的错**:那正是人被要求判断的那一件事。
徽章还写着 Ethereum,而费用是 xDAI——两处互相矛盾,谁也没说破。

修法:宿主保留 `origin` 和 `chain_id`,抬头和徽章从请求画。
**名字就用 host 本身**:从域名猜一个好看的名字是猜,而这个位置上的猜正是
仿冒域名冒充真站点的路子;图里那些漂亮名字要靠请求带 dApp 身份,现在还没有。

### 第三次:滑块上写着"确认兑换",而这是一笔转账

confirm 文案也还是 fixture 的。改成读核心的 `ClearConfirm`——
核心自己注明 **`Confirm` 永远不是 "Approve"**,那个动词只属于真正的代币授权
(那是 `approval_guard` 的地界)。intent 以英文规范键传过来,壳只翻它有词的那几个,
其余显示中性动词——**给读中文的人看一个英文键,比显示"确认"更糟**。

### 还差的那一个:代币符号 —— 查下来是**四端共有的核心缺口**

现在仍然显示 `1 0x2a22…`——**数量对(小数解析出来了),符号是原始地址**。
追下去不是接线问题:

`clear_signing::format_token_amount` 的符号来自 `known_token_symbol`,而它查的是
**一张写死 19 条的 `KNOWN_TOKENS` 表**(`services/tokens.ts` 移植过来的),
**而且只按地址查、不带 chain id**——表里第 17–19 条是 Polygon USDC、Polygon USDC.e、
Arbitrum USDC。**Gnosis 一条都没有。**

`clear_signing` 一共四个事件(`ResolveTransaction`/`ResolveTypedData`/
`MessagePresented`/`Cleared`),**没有一个能让壳把代币元数据递进去**;
它的探针只有 ERC-165 和 decimals,**没有 symbol**。所以核心没有任何途径知道
它表外的符号——**这 19 个地址之外的每一个代币,在四个端上都显示成 `0x…`**,
因为四个端跑的是同一份核心。桌面只是第一个把它照出来的。

**这需要一个决定,我没有替你做**:

| 选项 | 代价 |
|---|---|
| A. 给 `clear_signing` 加一条"壳提供代币元数据"的缝(事件或操作) | 改的是四端共用的核心机器,要重生成 ts-rs 两套、重建 wasm;而且 `app/` 下这些文件的演进现在是 web 会话在管 |
| B. 壳侧兜底:字段带着 `token_address`(核心注明是给 logo 用的),当核心退回到地址缩写时,用壳自己认识的符号替换 | 壳在改核心产出的字符串。要严格守住"只在等于核心那个兜底串时才替换",否则就是盲改 |
| C. 只往 `KNOWN_TOKENS` 里加几条 | 治不了本:每条链都有自己的代币,而这张表连 chain id 都不带 |

我倾向 **A**——符号是"人在读自己转什么",它应该和 decimals 一样是核心问得到的事实,
而不是每个壳各自打的补丁。但它动的是别人在管的文件,所以等你点。

## Phase 23 — 代币符号:核心多问一句 `symbol()`

按上一段那三个选项里的 **A** 做:给核心一条问符号的路,而不是每个壳各打各的补丁。
创始人点的。

### 为什么是探针,不是新事件

我原来把 A 写成"加一个事件让壳递元数据"——那要动 `Event`、要每个壳配合。
实际最小的形状是**照着 decimals 再来一个**:核心早就在用 `RpcEthCall` 问
`decimals()`,只是从来没问过 `symbol()`。所以只加 `ClearProbe::Symbol`
和一个选择器常量。

**四个端一行都不用改**:每个壳的 `RpcEthCall` 都是把 `probe` 原样回显的
(web 的 `clear-executor.ts` 是,我这刀写的桌面执行器也是),
所以新探针天然被回答。**先查了再动**,不是改完祈祷。

### 顺序:表 → 链 → 地址

`KNOWN_TOKENS` 那 19 条仍然优先(它是 TS 那边的权威),然后是链自己答的,
最后才是地址缩写。**只有最后一档会让人对着一个合约地址读金额。**

**只有真词才教缓存**:revert、空答案、解不出 UTF-8 的,都留着原来的兜底——
一个乱码符号挂在金额旁边,读起来像一个没听过的真符号,比地址更糟。

### 探针不设门

符号和 decimals 一起发,但 **warm 那一步仍然只等 decimals**。
慢的符号不该把单子卡住;它没赶上,代价只是回到这条探针出现之前的样子。

### ABI string 两种形状

`symbol()` 的返回有两种布局,因为 ERC-20 早于字符串约定:寻常的
`[offset][length][data]`,和 **bytes32**(MKR 那一代返回一个定长字)。
长度对不上载荷时按第二种读——**越界的长度正是 bytes32 答案在偏移读法眼里的样子**。
三条测试钉住:两种布局、多字节符号 `USD₮0`、以及 revert/空/非 UTF-8 都答 `None`。

### 四个测试挂了,是驱动不是产品

核心测试的 `Sut::resolve` 是 **FIFO**(`pending.pop_front()`),
每个地址多一个操作就把位置全错开了——**和 028 记的"FIFO 驱动遇上新计时器操作"
是同一个坑**。改法是让那些测试也回答符号探针(按发出顺序),
或者把 `drop_oldest` 补成两次。产品行为没错:问符号正是这一刀的目的。

vela-core **1,285 passed**、clippy `-D warnings` 干净;wasm 重建
`6b0ea32cc2ed`(3,727,829 字节),`gen-core-types` 两套镜像都跑了
(`ClearProbe.ts` 现在是四个变体),`verify-web` 46,513 例全绿。

## Phase 24 — 池子不再是一条单人队,和"1 USDC 显示成 0"

phase 23 提交时只跑了测试,没把面板真的打开看一眼。**打开一看,数量是 `0`**——
一笔 1 USDC.e 的转账,签名屏上写着 0,旁边挂着"金额无法链上核验"。
比 phase 22 那次(显示 `1 0x2a22…`)更糟:那次错的是符号,这次错的是**钱数**。

### 追下去:不是解码,是排队

在执行器里给每次 `eth_call` 打上耗时,第一次跑:

```
[probe] Symbol   … done in 4.116s
[probe] Decimals … done in 4.752s
```

而 `decimals` 的 warm 窗口是 **4 秒**。超时那一档的兜底是"18 位小数 + unverified",
于是 `1000000 / 1e18` 被格式化成 **`0`**(`format_token_value` 保留 4 位小数)。
curl 直连同一个 Gnosis 端点是 0.8–1.9 秒。慢的不是链,是我们自己。

在池子里打上每个 POST 的开始/结束,真相很干净——**每一次 START 都正好是上一次 DONE**:

```
[pool]  4.617 post eth_getBalance https://mainnet.optimism.io    START
[pool] 12.619 post eth_getBalance https://mainnet.optimism.io    DONE 8.001s
[pool] 13.209 post eth_getBalance https://bsc-dataseed.binance.org START
[pool] 21.210 post eth_getBalance https://bsc-dataseed.binance.org DONE 8.001s
```

`executor/pool.rs` 的 `drain()` 在**池子线程上原地做**每一个操作,包括那次 HTTP POST
和 `StartBackoff` 的 `thread::sleep`。所以:**进程里任何一次 RPC 都排在其他所有 RPC 后面**。
Optimism 一个 8 秒超时,期间整个钱包一次链都读不了——签名单那条 4 秒预算的
`decimals()` 探针,输给的是一条它根本不关心的链上的余额读。

这不是"慢",是**架构上的单点串行**:池子既是路由权威,又是唯一的执行者。

### 改法:路由留在一根线上,等待搬出去

池子线程仍然是唯一的路由权威(封禁表、EMA、竞速赢家——那正是"一个会话"的意义),
但**会阻塞的三件事交给工作线程**:`JsonRpcPost`、`ProbeChainId`、`StartBackoff`。
一条 channel 同时收调用者的 `Ask` 和工作线程的 `Finished`(mpsc 没有 select,
所以是一条 channel 两种消息);回来的 body 由**池子线程**归档进 `inflight`,
工作线程不碰任何共享可变状态。

- **上限 32 个工作线程**,到顶就退回原地执行——降级成从前的样子,而不是一千个线程。
- 迟到的答案是安全的:`CoreHost::resolve` 的规则 2 就是"没人再问的问题,答案丢掉"。
- **核心的端点竞速这才第一次真的发生**。`rpc_pool` 一直能一次给出多个 POST,
  而壳把它们一个接一个地做——于是"最快的端点"其实是"第一个端点,加上排在它前面的人"。

同一次启动,改完之后:

```
[pool] 3.466–3.472 十二条链的 eth_getBalance 一起 START
[probe] Decimals … done in 622ms   ← 从 2.0–4.7 秒
```

面板上数量变成 **`1`**,"无法核验"的警告消失。整个开机 RPC 从 25 秒压到 5 秒。

### 顺手一个:池子在倒着执行核心的清单

`drain()` 用的是 `Vec::pop`——**从尾巴取**。核心给出多个端点时是**按分数排好序的**,
壳却先去够最差的那一个。结果不算错(接受哪个答案是核心的判决,它点名 URL),
但"先打哪个端点"是路由决定,而这个文件自己的头一句就是它不做路由决定。
改成 `VecDeque` 按序取。

### 测试

一条**不联网**的测试:两条本地链、两个 loopback 上的假 RPC,一个故意慢 1.5 秒。
慢的先发,快的后发,断言快的**在慢的还没回来之前**就答完了。
把 `offload` 临时改成永远拒绝,它会失败(`1.527s`,正好是慢的那条的延迟)——
**证明这条测试真的在测这件事**,而不是两种实现都能通过。

真网四组(`executor::pool` / `relay` / `chain` / `user_op`)与 `wallet::money`
全部照跑:金标 Safe 0.74797 xDAI 读到、封禁仍然跨机器共享、确认页仍然拿到真报价。

desktop **291 / 287**(+1),fmt clean,画廊 36 态全渲染,Windows 类型检查通过。

### 还欠(这一刀照出来的,都要人点)

1. **代币符号仍然显示成地址,phase 23 那条探针实际上等于没生效。**
   日志里符号是**答出来了**的(`USD C.e` 的 UTF-8 就在 payload 里,622ms/642ms
   两条几乎同时回来),但 phase 23 明写了"探针不设门":warm 只等 decimals,
   decimals 一到就**立刻格式化**——晚 20 毫秒的符号只进了缓存,这张单子再也不看它。
   两条探针是**同一次往返一起发出去的**,谁先回来是掷骰子,所以第一次看到某个代币时
   **几乎总是**显示地址。三条路:
   (A) 让符号和 decimals 一起当门(同一次往返,4 秒上限照旧兜底);
   (B) 符号迟到时**重排字段**(但签名屏上的金额行在人读的过程中变化,本身是一类风险);
   (C) 维持现状(只有第二次遇到同一个代币才有符号)。
   我倾向 **A**,但它要推翻 phase 23 自己写下的"探针不设门",所以等你点。
2. **决定不了小数时,`0` 是一句谎话。** 核心在 warm 超时后按 18 位格式化并标
   `unverified`,于是 1 USDC 变成 `0`。四个端共用这条规则(web 移植过来的)。
   一个签名屏宁可说"数量未知",也不该说一个**确凿的错数字**。同样是核心的事,同样等你点。
3. **每次 POST 都新建一个 ureq Agent**(`proxy::agent`),所以每次调用都付一次 TLS 握手,
   而那个函数的注释自己写着"连接复用很重要"。并发之后这件事更值钱了。没动,记账。

## Phase 25 — 创始人点的两件事:符号当门,和"不知道就说不知道"

phase 24 末尾报的两个决定,创始人都点了:**A(符号也当门)** 和 **说「数量未知」**。

### 符号当门 —— 推翻 phase 23 自己写的"探针不设门"

phase 23 写着"慢的符号不该把单子卡住",听起来对,跑起来是空的:两条探针
**是同一次往返一起发出去的**,实测 622ms / 642ms 落地,相差 20 毫秒。
`decimals` 一到就格式化,于是符号只进了缓存——**第一次见到一个代币,几乎必然显示地址**。
"不设门"实际不是"偶尔晚",是"基本没有"。

现在 `Step::AwaitWarm` 拿两个集合,两边都空了才收工。**代价有上限**:一个答得出小数、
却在符号上挂住的代币,最多等到那条本来就有的 4 秒 timer,然后照旧用已知的东西格式化。
下限没动,动的是常见情况。

### `0` 是一句谎话 —— 核心不再算它算不出来的数

`1000000` 用 18 位兜底格式化,`format_token_value` 保留 4 位小数,结果就是 **`0`**:
**一笔 1 USDC 的转账,在签名屏上写着 0。** 小数不可验证 = 数量级不可验证,
核心因此**什么都不说**:`UNKNOWN_AMOUNT`(一个 em dash,就是这个钱包在费用卡上
"没有价格"时用的那个符号),`unverified` 旗标照旧。

壳再把它翻成人话。桌面读 `unverified` + 值以那个破折号开头,渲染 **`amountUnknown`**
——**这一刀唯一新增的语料键**(15 个语言各一句;`gen-i18n.mjs` 里那份"为什么加这个键"
的清单也照规矩续了一条)。**破折号本身是给还没接这个字段的另外三个端的**:
它们照现在的样子渲染 `value`,得到的是"没有数字",而不是一个错数字。

### 测试

核心两条:符号**后到**仍然上单(`500 USDC.e`);小数查不出来时那一行是破折号而**不是** `0.5`
(原来的测试断言的正是 `0.5`,它被改了——**这条 diff 就是这次修的东西本身**)。
桌面一条:未核验的那一行渲染成语料里的词、色调是 caution,而**已核验的那一行原样不动**
(不许改一个核心真算出来的数字)。

vela-core **1,286**(feature `i18n-all,crux,dev-fixtures`),clippy `-D warnings` 干净;
desktop **292 / 288**;wasm 重建 `5d01841e0bb3`(3,728,061 字节),`verify-web` 46,513 例全绿,
`gen-core-types` 两套镜像跑了(**类型没变**,只有 wasm 指纹动),画廊 36 态、Windows 照旧。

### 真机眼见为实

同一个本地页面、同一笔真 `transfer`:**`Amount  1 USDC.e`**。
(phase 22 是 `1 0x2a22…`,phase 24 打开时是 `0 0x2a22…` 挂着"无法核验"。)

### 这一跑又照出一个:**解析途中,面板画的是 mock 的内容**

截图时抓到了一个中间态:抬头是真的(`127.0.0.1:8137` · Gnosis),
**下面的正文却是画稿里那笔 "Swap 0.5 ETH → 1,278.11 USDC · Uniswap V3 Router"**。

原因在 `page.rs::signing_body`:`if !blocks.is_empty()` 才用核心的块。
核心还没解析完(`resolved=false`)、或者解析完但**没有结果**(盲签那一档),
`blocks` 就是空的,于是**画稿的正文留在屏幕上**——挂在一个真请求的抬头下面。

phase 22 修过反过来的那一半(真请求配 mock 的抬头);这是同一个错误的另一半,
而且更糟:**抬头是真的,会让人以为正文也是真的**。归 phase 26。

# 交接:下一个会话从这里开始

**范围:只做 desktop。** 分支 `032-desktop-money-wiring`(叠在 031 → 030 → 029 上,均未合并)。
工作区 `/Volumes/data/production/vela-wallet-native`,七个 phase(1–5、自查的 6/6b,
和把同一把尺子用到隔壁屏幕的 7),**十三个**提交(交接原写「十一个」,实数是 `049617f5..` 的 13)。

## 一句话状态

**桌面发过钱了**(SC-303 达标,2026-09-07:`0x98c8f65c…dfc9`,`receipt.status=0x1`,
Safe 少了 0.011000 逐位对上)。以下是那一推之前的状态,留作背景:**桌面能发钱,只差最后一推**:金标 Safe 真网走到确认页(真持仓、真报价 0.010 xDAI、
滑块已武装),`SlideConfirm` 藏在 `VELA_LIVE_SEND=1` 后面没拉——花真钱是创始人的决定
(SC-303)。固定密钥集签名者在 vela-core(`dev-fixtures`),4337 UserOp 装配在 vela-core
(`user_op.rs`,与 EIP-712 哈希器和 alloy ABI 编码器交叉验证),中继/链读/提交主干、
fee_policy 与 tx_tracker 常驻、send 宿主与七块屏(含批量导入)全接。**A 组全完。**

phase 6/6b 是**对我自己 phase 4/5 的自查**:那两刀写的 live 构造器把核心十六个判断
字段全丢了(余额不够时按钮不动、屏幕不说)。现在一个 `SendNotice` 承载全部拒绝、CTA
三态、手续费币种按不变量⑧不可选、文件读不出与代币存不进都会说话——新增语料键 **0**
个。方法(第二条 grep)写在本文件末尾的「每次接手仍要跑」里,普查判定表在 phase 6b。

## 立刻可跑的闸门

> **别把 `cargo test` 接进管道再用 `&&` 串**(比如 `| tail -3 &&`):`&&` 接的是
> `tail` 的退出码,永远是 0,测试挂了照样报绿。要么原样跑,要么先 `set -o pipefail`。
> phase 8 有一次真实失败就是这么被瞒过去的。

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

基线(**并入 028、走完 phase 12 之后**):desktop **273 passed(feature on)/ 269(off)· 33 ignored**,
vela-core **1,282**,fmt clean,clippy `-D warnings` 无话,gallery 36 态全渲染,
**两种 feature 配置下各 1 个 warning**(`BLE_CHANNEL_SUPPORTED`)。
桌面数字:并树时删掉的 `executor/contact_io.rs` 带走 6 个测试,
phase 8 加 2、9 加 2、10 加 2、11 加 4、12 加 2(外加一条 `#[ignore]` 真网)。
(phase 7 之前 `--tests` 下其实有 3 个 warning,多的两个里一个是真缺陷,见第 6 条教训。)

**动过 `rust/` 就要**:`node rust/scripts/build-web.mjs`(不是 `--check`——指纹一定会动,
要重建入库)→ `verify-web.mjs` → `gen-onboarding-types.mjs --check`。
当前 wasm:`1b6c8ce4be03`,**3,725,860 字节**(032 自己那两次是 3,630,664 只改指纹名;
并入 028 后长了,因为 028 的新事件和拼音首字母表在里面)。

## SC-303:那一推怎么拉(**已拉,2026-09-07**;命令留着,复跑还会再花一次 dust)

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
| 1 | ~~Phase 5 批量导入~~ | **已交付**(phase 5)。**phase 10 补上样张**:`tests/fixtures/payroll-sample.xlsx` 已提交,workbook 那条路以前一次都没在测试里跑过;两个测试从文件一路走到付款行。**仍欠**:真表格实机点一次(要人点文件对话框) |
| 2 | **B 组签名面板**(`clear_signing` `approval_guard` `sign_request`,9,362 行) | 图 DCS1–8 画好、33 个手写场景;`user_op::compute_safe_message_hash` 与 `build_eip1271_signature` 已备好。请求来源仍缺(C 组要 web 引擎) |
| 3 | 真实认证器签一笔发送(USB / caBLE / 平台库) | 本刀没插过钥匙。走的是登录同一条 `passkey::assert` 缝,理论上同路;实机跑一次 |
| 4 | `SendOperation::AddNetwork` | 答 `Error`(移植的 catch 分支)。锁定请求要加网时应走设置向导 |
| 5 | `SimulateCalls` | 桌面没有模拟引擎,答 `None` |
| 6 | 031 留的五件 | **两件已交付**:Windows 日界线(phase 11)、**余额流式**(phase 12,`Answer::Streaming` + 逐链上报,真网验过)。剩三件全部有前置:收藏控件(桌面图里没有星标,**缺图**)、设置页新建/登录账户(**要导航决策**)、扫码(桌面没有相机管线,是新功能不是接线) |
| 7 | Tempo 提交路径 | 已移植(`submit_tempo`)但没在 Tempo 链上跑过 |
| 8 | ⇄ 法币/代币切换控件、多币归集(sweep)选择器、拆分行逐行改额 | 桌面**没画**。phase 6 已把 ⇄ 的拒绝理由说出来了(核心的 `denom_toggle_reason`),但控件本身要图 |
| 9 | ~~设置里加网络向导的 `NetWizardView.{phase,error}`、`NetView.last_added_chain_id`~~ | **已交付**(phase 7):六种状态全说话、对话框按核心的记录关而不是按下就关;新增语料键 0。同一刀顺手修了编译器早就在报的 `AddToken.notice`(phase 6 自己留的) |
| 10b | ~~`NetNetworkRow.explorer_health`~~ | **已交付**(phase 10):同一个徽章位,RPC 传了、浏览器传的是 `None`。一行 |
| 10c | **`PaymentRequestView.can_copy` / `can_save`(要创始人定)** | 收款页是活的,但这道"确认过才允许复制/保存"的门桌面没实现。没自作主张加——Receive 的链上门(issue #14)是被判过时关掉的,这道该不该有是产品判断。见 phase 9 |
| 10d | `FeeView.stale` | 30 秒 TTL 到了没有刷新控件。核心说这是 advisory、提交侧另有硬门;要做得先有图 |
| 10 | `FeedView.toast`(到账庆祝)、`ContactRecipientView` 的信任行、`PaymentRequestView` 的付款链接面 | 都没图/没入口;普查表在 phase 6b |

## 本刀最值得记的五件事

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
5. **接完线要再自查一遍:界面是不是把核心的判断丢了。** phase 4/5 我把七块屏接活了,
   phase 6 一查,`SendView` 十六个判断字段一个没读——最糟的是余额不够时按钮不动、屏幕
   不说,而移植的门偏偏是**按下才拒绝**,那句警告是中间唯一的东西。同类还有手续费币种
   `insufficient`(画成可选=选了必失败)、`file_error`、`save_error`。**语料通常已经有词**
   (这十六处新增键 0 个)。判定要逐条看核心意图:`failed_chain_ids` 未读是对的,因为核心
   给了 `banner_chain_ids`(减去会自愈的限流)。

6. **警告数要按真数字记,别按印象记。** 交接写"1 个既有 warning",`cargo test` 下其实是 3 个;
   多出来的那两个里有一个(`AddToken.notice` 从来没被画)是 phase 6 自己留下的真缺陷,
   编译器指着它说了不知道多少遍。压着不看的警告,下一条真的就藏在它后面。

## ~~028 合并后要立刻做的~~ — **已并、六步已走完(phase 8,2026-09-07)**

> `origin/main` = `61568f22`(PR #186)已并进本分支,`executor/contact_io.rs` 已删,
> 分组字母归核心。详见上面的 **Phase 8**。下面这份原始清单留着当对照记录。
> **注意**:029/030/031 仍未单独合并进 main;本分支现在既含它们也含 028。

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
