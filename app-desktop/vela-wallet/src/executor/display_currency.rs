//! The only place the `display_currency` machine touches the outside world.
//!
//! Four operations, and the interesting one is the one that answers `None`.
//!
//! ## `resolve_rate` answers `None`, and `None` is not `1`
//!
//! There is no rate source on the desktop until spec 031 wires the RPC pool, so
//! this cut answers "cannot price it right now". The temptation is to answer `1`
//! and move on; the core's own doc explains why that is a defect rather than a
//! default: *"a rate of 1 is a claim (1 USD = 1 CNY)"*, and a fiat-denominated
//! amount multiplied by a defaulted 1 is a real mispayment. The shell's job is
//! to report what it observed. Degrading the display is the core's decision, and
//! it already knows how to make it.
//!
//! ## `read_device_currency` answers `None` too, and that one is a choice
//!
//! Unlike the web — whose core comment says "None on web", because a browser has
//! no region currency — a desktop **does** have one: `Loc::from_env` already
//! resolves `LC_ALL`/`LANG`, and `zh_CN` carries a region. Answering it would be
//! easy and would be wrong today. The core persists a seeded currency **only
//! after a real rate resolves**, precisely because "a seeded currency rendering
//! at the rate-1 fallback (₫78 instead of ₫2,000,000) is strictly worse than
//! staying on USD". With no rate source in this cut, seeding buys a label that
//! cannot commit and resets on the next launch. It goes live in 031, beside the
//! rate, with the region→ISO-4217 table it needs.

use gpui::App;

use vela_core::app::display_currency::{
    CurrencyOperation, CurrencyShellResult, DisplayCurrency, Event,
};

use crate::executor::storage;
use crate::resident::{Answer, Machine};

impl Machine for DisplayCurrency {
    const LABEL: &'static str = "display_currency";

    fn boot_event(_cx: &App) -> Event {
        Event::Refresh
    }

    fn perform(operation: &CurrencyOperation) -> Answer<CurrencyShellResult> {
        match operation {
            // Absent ALWAYS means "the user never chose" — including when the
            // read failed, which is why this cannot surface an error.
            CurrencyOperation::ReadStoredCode => Answer::Now(CurrencyShellResult::StoredCode {
                code: storage::read_value(storage::KEY_DISPLAY_CURRENCY)
                    .ok()
                    .flatten()
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .map(str::to_owned),
            }),

            // Best effort, exactly as every other client's `setCurrency` is: the
            // in-memory choice stays authoritative for this session either way.
            CurrencyOperation::WriteStoredCode { code } => {
                let _ = storage::write_value(
                    storage::KEY_DISPLAY_CURRENCY,
                    serde_json::Value::String(code.clone()),
                );
                Answer::Now(CurrencyShellResult::CodeWritten)
            }

            // live in 031 — see the module note.
            CurrencyOperation::ReadDeviceCurrency => {
                Answer::Now(CurrencyShellResult::DeviceCurrency { code: None })
            }

            // live in 031 — no rate source exists until the pool does. `None`
            // is an observation, not a fallback.
            CurrencyOperation::ResolveRate { code } => {
                Answer::Now(CurrencyShellResult::RateResolved {
                    code: code.clone(),
                    rate: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_host::CoreHost;
    use vela_core::app::display_currency::CurrencyView;

    fn perform(operation: CurrencyOperation) -> CurrencyShellResult {
        match DisplayCurrency::perform(&operation) {
            Answer::Now(result) => result,
            _ => unreachable!("every currency operation is local"),
        }
    }

    /// Drive the whole loop the way the app does — dispatch, perform, resolve,
    /// until quiescent — without a window.
    ///
    /// `resident.rs` adds gpui's scheduling on top of this; the DECISIONS being
    /// exercised here are the same ones, so a rule that breaks breaks here too.
    fn drive(host: &mut CoreHost<DisplayCurrency>, event: Event) -> CurrencyView {
        let mut pending = host.dispatch(event);
        while let Some(next) = pending.pop() {
            let result = match DisplayCurrency::perform(&next.operation) {
                Answer::Now(result) => result,
                _ => unreachable!("every currency operation is local in this cut"),
            };
            pending.extend(host.resolve(next.id, result));
        }
        host.view()
    }

    /// A wallet with no stored preference lands on USD at rate 1 — a real
    /// claim, and the only one that is true by definition.
    #[test]
    fn a_fresh_wallet_commits_usd() {
        storage::tests::with_temp_state("currency-fresh", || {
            let mut host = CoreHost::<DisplayCurrency>::new();
            let view = drive(&mut host, Event::Refresh);
            assert_eq!(view.code, "USD");
            assert_eq!(view.rate, Some(1.0));
            assert!(view.committed, "USD at 1 is committable without a source");
        });
    }

    /// An explicit choice persists immediately — "user choice always wins" —
    /// and comes back after a relaunch. But it comes back UNPRICED, because
    /// this cut has no rate source, and the core says so rather than inventing
    /// a rate to make the pair look complete.
    #[test]
    fn a_chosen_currency_comes_back_unpriced_rather_than_invented() {
        storage::tests::with_temp_state("currency-chosen", || {
            let mut host = CoreHost::<DisplayCurrency>::new();
            drive(&mut host, Event::Refresh);
            let view = drive(
                &mut host,
                Event::UserChose {
                    code: "JPY".to_owned(),
                },
            );
            assert_eq!(view.code, "JPY", "the person's choice must win");
            assert_eq!(
                view.rate, None,
                "no source could price it — and None is not 1"
            );

            // Relaunch: a fresh core over the same directory.
            let mut relaunched = CoreHost::<DisplayCurrency>::new();
            let after = drive(&mut relaunched, Event::Refresh);
            assert_eq!(after.code, "JPY", "the choice did not survive the relaunch");
            assert_eq!(after.rate, None);
        });
    }

    #[test]
    fn a_chosen_currency_survives_a_relaunch() {
        storage::tests::with_temp_state("currency-roundtrip", || {
            assert_eq!(
                perform(CurrencyOperation::ReadStoredCode),
                CurrencyShellResult::StoredCode { code: None },
                "an unwritten preference must read as 'never chose'"
            );

            perform(CurrencyOperation::WriteStoredCode {
                code: "JPY".to_owned(),
            });

            assert_eq!(
                perform(CurrencyOperation::ReadStoredCode),
                CurrencyShellResult::StoredCode {
                    code: Some("JPY".to_owned())
                }
            );
        });
    }

    /// The defect class `app/money.rs` exists to prevent. A shell that answered
    /// `1` here would make the core commit a currency it cannot price, and every
    /// fiat figure downstream would be a claim nobody checked.
    #[test]
    fn an_unpriceable_currency_answers_none_and_never_one() {
        let answer = perform(CurrencyOperation::ResolveRate {
            code: "VND".to_owned(),
        });
        match answer {
            CurrencyShellResult::RateResolved { code, rate } => {
                assert_eq!(code, "VND", "the answer must identify what it priced");
                assert_eq!(rate, None, "no source could price it — and None is not 1");
            }
            other => unreachable!("wrong variant: {other:?}"),
        }
    }

    /// Answered, not skipped. A skipped operation leaves the core waiting
    /// forever, which is the cardinal sin of the executor contract.
    #[test]
    fn the_device_currency_is_answered_rather_than_skipped() {
        assert_eq!(
            perform(CurrencyOperation::ReadDeviceCurrency),
            CurrencyShellResult::DeviceCurrency { code: None }
        );
    }
}
