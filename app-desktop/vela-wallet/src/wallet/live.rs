//! The wallet home's display models, built from what the cores decided.
//!
//! The sibling of `fixtures.rs`, never its replacement — the third of these
//! (settings, contacts, wallet) and the one where the rules bite hardest,
//! because every value here is somebody's money.

use gpui::SharedString;

use vela_core::app::balance_dashboard::{BalanceNotice, BalanceView};
use vela_core::l10n::currency::{FiatOptions, format_fiat};

use crate::wallet::WalletStrings;
use crate::wallet::fixtures::{BALANCE_MASK, BalanceModel, BalanceState, StatusKind};

/// The balance hero.
///
/// Four states, and the two that look like edge cases are the ones the core
/// went to trouble over:
///
/// - **hidden** — the fiat value is withheld *by construction*, not masked
///   downstream. The core's invariant ⑧: a leak in one surface defeats the mask
///   everywhere, so `display_total_usd` is already `None` and there is nothing
///   here to accidentally print.
/// - **unknown** — `None` renders a skeleton, **never a fake `$0`** (invariant
///   ②). A wallet that shows zero while it is still counting has told the person
///   their money is gone.
#[must_use]
pub fn balance(view: &BalanceView, s: &WalletStrings, locale: &str) -> BalanceModel {
    let status = if view.refreshing {
        Some((StatusKind::Refreshing, s.balance_stale.clone()))
    } else {
        view.notice.map(|notice| {
            (
                StatusKind::Warning,
                match notice {
                    // Partial and still retrying — the figure is real but not
                    // final, which is a different thing from wrong.
                    BalanceNotice::StillUpdating => s.balance_stale.clone(),
                    BalanceNotice::Unpriced => s.balance_unpriced.clone(),
                },
            )
        })
    };

    if view.hidden {
        return BalanceModel {
            label: s.total_balance.clone(),
            state: BalanceState::Hidden,
            integer: SharedString::from(BALANCE_MASK),
            decimals: None,
            live: None,
            status,
        };
    }

    let Some(usd) = view.display_total_usd else {
        return BalanceModel {
            label: s.total_balance.clone(),
            state: BalanceState::Loading,
            // Not "$0". The core withholds the number until it has one, and the
            // shell must not fill the gap with a figure that reads as an answer.
            integer: SharedString::from(""),
            decimals: None,
            live: None,
            status,
        };
    };

    let (integer, decimals) = split_fiat(usd, locale);
    BalanceModel {
        label: s.total_balance.clone(),
        state: if usd == 0.0 {
            BalanceState::ZeroLive
        } else {
            BalanceState::Normal
        },
        integer,
        decimals,
        live: None,
        status,
    }
}

/// `$1,383.28` → `("$1,383", Some("28"))`.
///
/// The hero draws the minor units smaller than the number — the design
/// language's "subordinated symbols" rule — so the split is a render concern and
/// belongs here rather than in a formatter. Splitting on the LAST `.` is what
/// keeps a locale whose group separator is `.` from being cut in half.
fn split_fiat(usd: f64, locale: &str) -> (SharedString, Option<SharedString>) {
    let formatted = format_fiat(usd, "USD", "$", locale, FiatOptions::default());
    match formatted.rsplit_once('.') {
        Some((whole, minor)) if minor.chars().all(|c| c.is_ascii_digit()) => (
            SharedString::from(whole.to_owned()),
            Some(SharedString::from(minor.to_owned())),
        ),
        // No minor units — a large balance drops them by product rule
        // (`drop_minor_units_above`), and a currency with zero fraction digits
        // never had them.
        _ => (SharedString::from(formatted), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_host::CoreHost;
    use vela_core::app::balance_dashboard::{BalanceDashboard, Event as BalanceEvent};

    /// A real `BalanceView` with the total substituted.
    ///
    /// Taken from a booted core rather than hand-written: this view has a dozen
    /// fields with their own invariants, and a literal I typed would be a guess
    /// about them that drifts the first time one changes. (It already did —
    /// two of my guessed field names did not exist.)
    fn view(usd: Option<f64>) -> BalanceView {
        let mut host = CoreHost::<BalanceDashboard>::new();
        let _ = host.dispatch(BalanceEvent::AccountChanged {
            address: "0xabc".to_owned(),
        });
        BalanceView {
            display_total_usd: usd,
            balance_unknown: usd.is_none(),
            ..host.view()
        }
    }

    fn strings() -> WalletStrings {
        WalletStrings::resolve(&crate::loc::Loc::from_env())
    }

    /// The figure the mock draws, from a real total.
    #[test]
    fn a_known_total_renders_split_for_the_hero() {
        let model = balance(&view(Some(1383.28)), &strings(), "en");
        assert_eq!(model.state, BalanceState::Normal);
        assert_eq!(model.integer, SharedString::from("$1,383"));
        assert_eq!(model.decimals, Some(SharedString::from("28")));
    }

    /// Invariant ②. A wallet that shows $0 while it is still counting has told
    /// somebody their money is gone.
    #[test]
    fn an_unknown_total_is_a_skeleton_and_never_a_zero() {
        let model = balance(&view(None), &strings(), "en");
        assert_eq!(model.state, BalanceState::Loading);
        assert!(
            !model.integer.contains('0'),
            "a skeleton must not print a figure: {:?}",
            model.integer
        );
        assert_eq!(model.decimals, None);
    }

    /// A real zero is a different fact from an unknown one, and gets its own
    /// state — the mocks draw them differently on purpose.
    #[test]
    fn a_real_zero_is_not_the_same_as_unknown() {
        let model = balance(&view(Some(0.0)), &strings(), "en");
        assert_eq!(model.state, BalanceState::ZeroLive);
        assert_eq!(model.integer, SharedString::from("$0"));
    }

    /// Invariant ⑧: the value is withheld by construction. The core already
    /// nulls `display_total_usd` when hidden, so there is nothing here that
    /// could leak — this asserts the shell does not reintroduce it.
    #[test]
    fn hiding_masks_and_carries_no_figure() {
        let mut hidden = view(None);
        hidden.hidden = true;
        let model = balance(&hidden, &strings(), "en");
        assert_eq!(model.state, BalanceState::Hidden);
        assert_eq!(model.integer, SharedString::from(BALANCE_MASK));
        assert_eq!(model.decimals, None);
    }

    /// A locale whose group separator is `.` must not be cut in half.
    #[test]
    fn splitting_survives_a_dot_grouped_locale() {
        let (integer, decimals) = split_fiat(1383.28, "de");
        assert!(
            integer.contains("1.383") || integer.contains("1,383"),
            "the whole part lost its grouping: {integer}"
        );
        assert!(
            decimals.as_ref().is_none_or(|d| d.len() <= 2),
            "the split took too much: {decimals:?}"
        );
    }
}
