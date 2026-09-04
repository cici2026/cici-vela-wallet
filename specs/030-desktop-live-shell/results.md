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
