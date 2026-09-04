# Results — 030 desktop-live-shell

Delivery ledger in the 019 format. Baselines first, because "did this grow?" has no
answer after the fact.

## Baselines — recorded 2026-09-04, branch point `dede5a4c` (tip of `029-native-repair`)

**Stacked on 029, not on `main`, deliberately.** Both cuts heavily edit
`app-desktop/vela-wallet/src/wallet/page.rs` — 029 added `Section::Explore` and a
379-line render body, 030 adds the live settings and contacts bindings to the same
file. Branching from `main` would guarantee a conflict, and 030's gate is the desktop
CI job 029 introduced.

### Desktop source at the branch point

| File | Lines |
|---|---|
| `src/**/*.rs` (57 files) | **32,050** |
| `wallet/page.rs` | **4,172** ⚠ |
| `settings/fixtures.rs` | 564 |
| `settings/components.rs` | 874 |
| `contacts/fixtures.rs` | 509 |
| `contacts/components.rs` | 564 |
| `executor/storage.rs` | 479 |
| `executor/mod.rs` | 391 |
| `core_host.rs` | 157 |
| `session.rs` | 230 |

⚠ `page.rs` is the risk this cut has to manage: it is 4,172 lines and every phase
touches it. The model/live seam exists precisely to keep the *decisions* out of it —
if a phase's `page.rs` delta exceeds ~400 lines, the panel wiring splits into
`settings/panels.rs`, for which `flows/panels.rs` (1,230 lines) is the precedent.

### Test baseline

`cargo test`: **93 passed · 0 failed · 5 ignored** (93 compiled; the 5 ignored need
hardware or the live registry). Inherited from 029, which took it from 88.

### The three machines (core side, already written and tested)

| Machine | Lines | Operations | Events |
|---|---|---|---|
| `network_admin` | 2,900 | 15 | 21 |
| `contacts` | 1,385 | 7 | 11 |
| `display_currency` | 413 | 4 | 3 |

**4,698 lines of rules this cut does not write.** The desktop links them directly —
no bridge, no JSON — so the entire cost is executors and rendering.

### Port provenance — web 024, at `f9bcb278`

| Source | Lines |
|---|---|
| `settings/core/network-admin-executor.ts` | 568 |
| `settings/live.ts` | 404 |
| `contacts/core/contacts-executor.ts` | 276 |
| `contacts/live.ts` | 216 |
| `settings/core/network-admin.svelte.ts` | 87 |
| `settings/core/currency-executor.ts` | 73 |
| `settings/core/currency.svelte.ts` | 67 |
| `services/storage.ts` | 95 |

Not ported, and the reason is the whole shape of this cut: `core/effect-loop.ts` (126)
and `core/json-shell.ts` (44) have **no desktop counterpart** — `CoreHost<A>` already
does that job in-process, in Rust enums, with no serialization at all.

## Phase 1 — the road, proven by the smallest machine

`src/resident.rs` (the generic gpui host), `executor/storage.rs` (+7 keys, generic
accessors, and a fixed bug), `executor/display_currency.rs`, `settings/live.rs`, and
the 货币 row bound to the core.

| Gate | Result |
|---|---|
| `cargo test` | ✅ **104 passed · 0 failed · 5 ignored** (baseline 93) |
| `cargo fmt --all --check` | ✅ clean |
| warnings | ✅ **10** — exactly the inherited baseline, none added |
| `scripts/sweep-gallery.sh` | ✅ every state rendered |
| `*fixtures.rs` diff | ✅ **empty** — not one constant touched |
| `rust/` diff | ✅ empty |

### Why the road ships with a machine rather than alone

The plan had phase 1 land the plumbing behaviour-neutrally. Done literally that adds
**18 dead-code warnings** for one commit — nothing consumes a road nobody drives on —
and a phase boundary whose artifact is a warning spike is a bad boundary. So the road
ships proven by `display_currency`: 4 operations, 413 lines of core, the smallest of
the three. Everything is live, and the warning count came back to the baseline 10.

**This moves the SC-004 probe to `contacts`, and costs nothing.** The probe's power is
being *third*, not being smallest — it measures whether the road was paved, and any
third machine measures that. It arguably improves: `network_admin` is now second, and
it is the one machine here that must do HTTP, so if the road needs changing that is
discovered at machine two rather than hidden behind an easy third.

### The bug the storage extension had to fix first

`save_registry_endpoint` wrote `json!({ "registry": url })` — a **whole-value write**
to `vela.serviceEndpoints`, under a **desktop-only field name**. Two defects, both
live the moment `network_admin` shares that key:

1. The write is destructive. Saving the index endpoint would erase
   `ethereumDataURL` / `bundlerServiceURL` / `fiatRatesURL`, and saving those would
   erase the index — a self-hosted stack quietly un-configuring itself.
2. Every other client spells it **`passkeyIndexURL`** (web's `services/endpoints.ts`,
   the Expo client). The desktop's "registry" *is* the passkey index —
   `executor/registry.rs` defaults to `p256-index-v2.getvela.app`. So a record written
   on desktop was invisible everywhere else.

Now: `merge_value` writes field-by-field, reads prefer `passkeyIndexURL` and fall back
to the legacy name, and a save drops the legacy field in the same write so the two
cannot disagree. Three tests pin it, including the pre-existing sign-out scope test,
which still passes unchanged.

### What `rate: null` bought, concretely

`settings/live.rs::currency_row_value` has two cases and the second is the point. A
priced currency renders `USD · $1,234.56` — byte-identical to what the DST3 mock
draws, so going live does not silently redraw a reviewed screen. An **unpriced** one
renders the code **alone**: not the code beside a USD figure wearing its symbol.
Rendering `¥1,234.56` for an unpriced JPY would assert an exchange rate nobody
obtained, which is exactly the claim `rate: None` exists to refuse. There is a test
whose only job is that the string contains no figure.

Two smaller things the tests pinned rather than assumed:
- CLDR separates a fallback alphabetic symbol from its digits with a **non-breaking
  space** (U+00A0), not a plain one. My first assertion used a plain space and failed;
  the character is now pinned literally, because the difference is invisible until it
  shows up as a bad line break.
- `read_device_currency` answers `None` **by choice**, not by inability. A desktop has
  a region (`Loc::from_env` resolves `LC_ALL`/`LANG`), unlike the browser the core's
  comment was written about. But the core persists a seed only after a real rate
  resolves, because "a seeded currency rendering at the rate-1 fallback (₫78 instead
  of ₫2,000,000) is strictly worse than staying on USD" — so with no rate source this
  cut, seeding buys a label that cannot commit. Owed to 031, with its region table.
