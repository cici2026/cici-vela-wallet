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
