# Results — 029 native-repair

Delivery ledger in the 019 format: baselines recorded **before** any change (the
question "did this grow?" has no answer after the fact), then one section per
phase, then the SC verdict table, consolidated deviations, and carried debts.

## Baselines — recorded 2026-09-04, branch point `f9bcb278` (`origin/main`)

### The defect, measured

| Platform | Unreachable source | Dead tests | Evidence |
|---|---|---|---|
| desktop | **3,571 lines** (`src/explore/` 925 + `src/signing/` 2,646) | 5 `#[test]`, never compiled | `main.rs` declares 22 mods; `src/` holds 24 dirs; the `diff` is exactly `explore`, `signing` |
| Android | **4,353 lines** (`feature/explore/` + `feature/signing/`) | 0 (fixture tests only, which do run) | `VelaDestinations` declares 10 routes; none is `EXPLORE` |
| iOS | **1,771 lines** (`Features/Explore/` + `Features/Signing/`) | 0 (fixture tests only, which do run) | `PageOverride.Page` has 7 cases (`RootView.swift:384`); `ExploreScreen(` never instantiated |
| **total** | **9,695 lines** | | |

### Test baselines

| Platform | Static count | Runtime baseline | Reconciliation |
|---|---|---|---|
| desktop | 100 `#[test]` | **88 passed · 0 failed · 5 ignored** (93 compiled, macOS, 2.44s) | 100 − 5 unreachable − 2 `#[cfg(target_os = "linux")]` in `executor/proxy.rs:308,317` = 93 |
| Android | 99 `@Test` | *(pending first CI run)* | |
| iOS | 130 `@Test` (Swift Testing macro, not `func test`) | *(pending first CI run)* | |

The 5 desktop `#[ignore]`d tests need hardware or the live registry:
`a_plugged_in_key_answers_get_info`, `the_deployed_registry_answers_its_health_probe`,
`an_unknown_public_key_is_simply_unregistered`, `register_then_assert`,
`excluded_credential_is_refused`.

### Recovery source, verified

| Check | Result |
|---|---|
| `969bf8fc` exists and carries the wiring | ✅ its `main.rs` declares `mod explore; mod signing; mod intro;` |
| The 8 explore/signing source dirs vs `main` | ✅ **byte-identical** — `git diff --stat main 020-intro-carousel -- <dir>` is empty for each |
| `git merge` is safe? | ❌ **no** — `git diff --stat main 020-intro-carousel -- app-desktop app-android app-ios` = 97 files, 3,775 ins / **26,229 del** (branch predates 021/023; a merge deletes iOS Flows and Settings) |
| `git apply --3way` is safe? | ❌ **no** — `git show 969bf8fc -- .../wallet/page.rs \| git apply --check --3way -` → *"Applied patch … with conflicts"* (`page.rs` was 2,082 lines then, 3,746 now) |
| Therefore | hand re-apply, using `969bf8fc` as a **specification** (FR-004) |

### CI baseline

`.github/workflows/ci.yml`: 5 jobs (`app`, `web`, `site`, `rust`, `rust-macos`).
**0 occurrences of `gradlew`. 0 of `xcodebuild`.** The only `app-desktop` reference
is line 241, `check-windows.sh`, which type-checks the C-free `vela-passkey-win`
crate *standalone* — its own header documents having missed exactly this class of
bug once before ("left the Windows path unlinked, with this gate green throughout").
The desktop app crate is not a `rust/` workspace member, so `cargo test --workspace`
cannot reach it either.

Blockers found for the CI work itself:
- **iOS has no shared scheme.** `app-ios/VelaWallet/VelaWallet.xcodeproj/xcshareddata/xcschemes/`
  does not exist, so `xcodebuild -scheme VelaWallet` cannot resolve today.
- **gpui is an unpinned git dependency** (`gpui = { git = ".../zed" }`, no `rev`/`tag`).
  Reproducible only via the committed `Cargo.lock`; the first `cargo update` moves
  desktop to an arbitrary Zed commit.

## Phase 1 — the guards, red (in progress)

`scripts/check-native-reachability.mjs` — static, no toolchain, runs in the existing
`app` job in well under a second. Against the unmodified tree it exits 1 and reports:

```
desktop: 2 module(s) on disk that src/main.rs never declares, so rustc never
         compiles them: explore, signing
android: 2 feature package(s) no VelaNavHost destination reaches: explore, signing
ios: 2 Features/ folder(s) RootView.swift never instantiates: Explore, Signing
```

**`EXEMPT` ships empty, and that is a finding.** Every candidate exemption was
checked and turned out to be genuinely reachable — desktop `ui`/`ctap`/`executor`
are all declared in `main.rs`; iOS `Gallery` is instantiated at `RootView.swift:70`.
With zero exemptions the guard reports exactly the two real orphans per platform and
no false positives. A guard that ships pre-populated with exemptions nobody needs is
a guard the next orphan hides behind.
