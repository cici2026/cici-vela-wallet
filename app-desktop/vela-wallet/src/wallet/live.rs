//! The wallet home's display models, built from what the cores decided.
//!
//! The sibling of `fixtures.rs`, never its replacement — the third of these
//! (settings, contacts, wallet) and the one where the rules bite hardest,
//! because every value here is somebody's money.

use gpui::SharedString;

use vela_core::app::activity_feed::{FeedDirection, FeedItem, FeedRow, FeedTxKind, FeedView};
use vela_core::app::balance_dashboard::{BalanceNotice, BalanceView};
use vela_core::l10n::currency::{FiatOptions, format_fiat};
use vela_core::l10n::number::{NumberPreset, format_token_amount};

use crate::wallet::WalletStrings;
use crate::wallet::fixtures::{
    ActivityKind, ActivityRowModel, AssetRowModel, BALANCE_MASK, BalanceModel, BalanceState,
    ChainRowModel, Fiat, StatusKind,
};

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

    /// Drive the real machine to a settled view, performing every operation
    /// it asks for on this thread.
    ///
    /// `Answer::After` is resolved immediately rather than slept: the only
    /// timer this machine sets is the partial-fetch retry, and a test that
    /// honoured its backoff would spend minutes proving nothing.
    fn settle(address: &str) -> BalanceView {
        use crate::resident::{Answer, Machine};

        let mut host = CoreHost::<BalanceDashboard>::new();
        let mut pending = host.dispatch(BalanceEvent::AccountChanged {
            address: address.to_owned(),
        });
        // A cap, not a timeout: a machine that kept asking would otherwise hang
        // the suite, and 64 operations is far past what one settle needs.
        for _ in 0..64 {
            let Some(next) = pending.pop() else {
                break;
            };
            let result = match BalanceDashboard::perform(&next.operation) {
                Answer::Now(result) | Answer::After(_, result) => result,
                Answer::Blocking(work) => work(),
            };
            pending.extend(host.resolve(next.id, result));
        }
        host.view()
    }

    fn strings() -> WalletStrings {
        WalletStrings::resolve(&crate::loc::Loc::from_env())
    }

    use vela_core::app::activity_feed::{
        ActivityFeed, Event as FeedEvent, FeedItem, FeedRow, FeedView,
    };

    fn feed_with(
        rows: Vec<FeedRow>,
        transactions: Vec<vela_core::app::activity_feed::FeedTxRecord>,
    ) -> FeedView {
        let mut host = CoreHost::<ActivityFeed>::new();
        let _ = host.dispatch(FeedEvent::AccountSwitched {
            address: "0xme".to_owned(),
        });
        FeedView {
            rows,
            transactions,
            ..host.view()
        }
    }

    fn item(id: &str, incoming: bool, value: Option<&str>, symbol: &str) -> FeedItem {
        FeedItem {
            id: id.to_owned(),
            direction: if incoming {
                FeedDirection::In
            } else {
                FeedDirection::Out
            },
            counterparty: Some("0xAbCdEf0000000000000000000000000000000001".to_owned()),
            alias: None,
            value: value.map(str::to_owned),
            symbol: symbol.to_owned(),
            decimals: Some(18),
            usd_value: 0.0,
            chain_id: 100,
            timestamp: 1_756_000_000.0,
            day_start_ms: 0.0,
            tx_hash: None,
            batch: None,
        }
    }

    /// Day headers are dropped for the home preview — the mocks draw a flat
    /// short list there — and the items keep the core's order.
    #[test]
    fn headers_are_dropped_and_items_keep_their_order() {
        let view = feed_with(
            vec![
                FeedRow::Header {
                    id: "day-0".to_owned(),
                    day_start_ms: 0.0,
                    timestamp: 1_756_000_000.0,
                },
                FeedRow::Item {
                    item: item("a", false, Some("2"), "POL"),
                },
                FeedRow::Item {
                    item: item("b", true, Some("120"), "USDT"),
                },
            ],
            Vec::new(),
        );
        let rows = activity_rows(&view, &strings(), false);
        assert_eq!(rows.len(), 2, "the header is not a row here");
        assert_eq!(rows[0].unit, SharedString::from("POL"));
        assert_eq!(rows[1].unit, SharedString::from("USDT"));
    }

    /// The sign is the direction's, and the minus is U+2212 — a hyphen does
    /// not align under a digit, which is why the mocks use the real one.
    #[test]
    fn direction_drives_the_sign() {
        let view = feed_with(
            vec![
                FeedRow::Item {
                    item: item("a", false, Some("2"), "POL"),
                },
                FeedRow::Item {
                    item: item("b", true, Some("120"), "USDT"),
                },
            ],
            Vec::new(),
        );
        let rows = activity_rows(&view, &strings(), false);
        assert_eq!(rows[0].amount, SharedString::from("\u{2212}2"));
        assert!(!rows[0].positive);
        assert_eq!(rows[1].amount, SharedString::from("+120"));
        assert!(rows[1].positive);
    }

    /// `value` is the human amount already. Scaling it by `decimals` would
    /// print every figure 10^18 times too large — and it would look deliberate.
    #[test]
    fn the_amount_is_not_scaled_by_decimals() {
        let view = feed_with(
            vec![FeedRow::Item {
                item: item("a", true, Some("1.5"), "xDAI"),
            }],
            Vec::new(),
        );
        let rows = activity_rows(&view, &strings(), false);
        assert_eq!(rows[0].amount, SharedString::from("+1.5"));
    }

    /// Privacy masks the FIGURE and keeps the unit — H5's rule — and it comes
    /// from the balance view so every money surface masks together.
    #[test]
    fn hiding_masks_the_figure_and_keeps_the_unit() {
        let view = feed_with(
            vec![FeedRow::Item {
                item: item("a", true, Some("120"), "USDT"),
            }],
            Vec::new(),
        );
        let rows = activity_rows(&view, &strings(), true);
        assert_eq!(rows[0].unit, SharedString::from("USDT"), "the unit stays");
        assert!(
            !rows[0].amount.contains("120"),
            "the figure must not survive the mask: {:?}",
            rows[0].amount
        );
    }

    /// A batch with mixed tokens has no sum, and the core says so by sending no
    /// value. Inventing one would be arithmetic nobody asked for.
    #[test]
    fn a_batch_without_a_sum_prints_no_figure() {
        let view = feed_with(
            vec![FeedRow::Item {
                item: item("a", false, None, ""),
            }],
            Vec::new(),
        );
        assert_eq!(
            activity_rows(&view, &strings(), false)[0].amount,
            SharedString::from("")
        );
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

    /// The home strip lists the core's holdings, in the core's order, and says
    /// nothing about a chain nobody holds anything on.
    #[test]
    fn the_home_strips_show_only_what_the_person_holds() {
        crate::executor::storage::tests::with_temp_state("home-strips", || {
            use vela_core::app::balance_dashboard::BalanceToken;
            let token =
                |chain_id: u32, symbol: &str, balance: &str, price: Option<f64>| BalanceToken {
                    chain_id,
                    symbol: symbol.to_owned(),
                    name: symbol.to_owned(),
                    balance: balance.to_owned(),
                    decimals: 18,
                    token_address: None,
                    price_usd: price,
                    spam: false,
                };
            let mut held = view(Some(100.0));
            held.tokens = vec![
                token(1, "ETH", "0.05", Some(2_000.0)),
                token(100, "xDAI", "0.75897", Some(1.0)),
                token(100, "USDC", "12", Some(1.0)),
            ];
            let s = strings();

            let assets = asset_rows(&held, &s, "en-US");
            assert_eq!(assets.len(), 3);
            // The core's order, kept: it already sorted by value.
            assert_eq!(assets[0].ticker, "ETH");
            assert_eq!(assets[0].chain, "Ethereum");
            assert!(matches!(&assets[0].fiat, Fiat::Value(v) if v.as_ref() == "$100.00"));

            let chains = chain_rows(&held, &s);
            // "All networks" plus the TWO chains held on — not the twelve the
            // wallet knows about.
            assert_eq!(chains.len(), 3);
            assert_eq!(chains[0].name, s.all_networks);
            assert_eq!(chains[0].count, 2, "the count is the chains listed");
            assert!(chains[0].dot.is_none(), "all is not a chain");
            assert!(chains[0].selected);
            assert_eq!(chains[1].name, "Ethereum");
            assert_eq!(chains[1].count, 1);
            assert_eq!(chains[2].name, "Gnosis");
            assert_eq!(chains[2].count, 2);

            // Still counting: an empty strip, never somebody else's tokens.
            let counting = view(None);
            assert!(asset_rows(&counting, &s, "en-US").is_empty());
            assert_eq!(chain_rows(&counting, &s).len(), 1, "only the all row");
        });
    }

    /// SC-003's visible half: a chain that could not be reached becomes a chip,
    /// and one that is merely rate-limited does not.
    #[test]
    fn an_unreachable_chain_becomes_a_chip_and_a_rate_limited_one_does_not() {
        crate::executor::storage::tests::with_temp_state("banner-chips", || {
            // Nothing wrong: no banner at all. An empty list is not a banner
            // saying "0 networks unavailable".
            assert!(unreachable_chips(&view(Some(10.0))).is_empty());

            let mut down = view(Some(10.0));
            down.banner_chain_ids = vec![100, 137];
            let chips = unreachable_chips(&down);
            assert_eq!(chips.len(), 2);
            assert_eq!(chips[0].2, "Gnosis");
            assert_eq!(chips[1].2, "Polygon");
            assert_eq!(chips[0].0, "G");
            // The tint is the settings table's, so the chip matches the network
            // row for the same chain.
            assert_eq!(
                chips[0].1,
                crate::settings::model::chain_tint(100)
                    .unwrap_or_else(|| unreachable!("Gnosis has a tint"))
            );

            // A network the person added is named by the name they gave it —
            // which is why these strings are owned rather than `&'static str`.
            let networks = serde_json::json!([
                { "chainId": 7_777_777, "displayName": "My testnet" }
            ]);
            if crate::executor::storage::write_value(
                crate::executor::storage::KEY_CUSTOM_NETWORKS,
                networks,
            )
            .is_err()
            {
                unreachable!("could not seed");
            }
            let mut custom = view(Some(10.0));
            custom.banner_chain_ids = vec![7_777_777];
            assert_eq!(unreachable_chips(&custom)[0].2, "My testnet");
        });
    }

    /// SC-001, end to end: the golden Safe's own money reaches the hero.
    ///
    /// Not a screenshot. This drives the real `balance_dashboard` machine over
    /// the real network and renders the real hero model, so what it proves is
    /// the whole chain — pool routing, the multicall, the price ladder, the
    /// core's total, and the split into the figure the screen draws.
    #[test]
    #[ignore = "reads every chain for a real address"]
    fn the_hero_shows_the_golden_safes_own_money() {
        crate::executor::storage::tests::with_temp_state("hero-live", || {
            crate::executor::chain_tokens::invalidate();
            crate::executor::chainlink::invalidate();
            const GOLDEN: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";
            let view = settle(GOLDEN);
            let model = balance(&view, &strings(), "en-US");
            println!(
                "  hero: {}{}  state={:?} notice={:?}",
                model.integer,
                model
                    .decimals
                    .as_ref()
                    .map_or_else(String::new, |d| format!(".{d}")),
                model.state,
                view.notice
            );

            // The number is REAL, not the fixture and not a skeleton. The
            // fixture total is $1,383.28, and it appearing here would mean the
            // app is showing somebody a stranger's money under their own name.
            assert_eq!(model.state, BalanceState::Normal, "still counting, or zero");
            assert!(
                model.integer.starts_with('$'),
                "no figure at all: {:?}",
                model.integer
            );
            assert_ne!(model.integer.as_ref(), "$1,383", "that is the fixture");
            let usd = view
                .display_total_usd
                .unwrap_or_else(|| unreachable!("no total"));
            // A band, not a figure: the Safe's balance moves on-chain, and a
            // test that goes red when the world changes reports the wrong
            // thing. What must hold is that the total is a plausible amount of
            // money rather than base units or a phantom chain's constant.
            assert!(usd > 0.01 && usd < 1_000.0, "implausible total ${usd}");
        });
    }
}

/// The chains a person cannot reach right now, as the banner's chips.
///
/// **This is SC-003's visible half.** The fetch already reports an unreachable
/// chain separately from an empty one, and the core already computes which of
/// those deserve a banner. Without this the verdict is correct and invisible,
/// which for the person looking at the screen is the same as absent.
///
/// The core's list, not `failed_chain_ids`: `banner_chain_ids` is failed MINUS
/// rate-limited (invariant ⑦), because a rate limit lifts on its own and a
/// "fix your RPC" banner that nags about one is telling somebody to repair
/// something that is not broken.
#[must_use]
pub fn unreachable_chips(view: &BalanceView) -> Vec<(SharedString, u32, SharedString)> {
    view.banner_chain_ids
        .iter()
        .map(|chain_id| {
            let name = crate::executor::custom_tokens::network_name(*chain_id);
            (
                crate::settings::model::lettermark(&name),
                // The same table the network rows and the activity badges read.
                // A second colour map for the same chains is how one screen's
                // Polygon stops matching another's.
                crate::settings::model::chain_tint(u64::from(*chain_id)).unwrap_or(0x8A_8F_98),
                SharedString::from(name),
            )
        })
        .collect()
}

/// The home's asset strip — the person's holdings, most valuable first.
///
/// The core already sorted and filtered them (`sortAndFilterHoldings`: zero
/// balances dropped, highest value first), so this only renders. Re-sorting
/// here would be a second opinion about which holding matters most.
///
/// Empty while the core is still counting, and empty is right: the strip
/// simply has no rows yet. The hero next to it is already saying "counting" in
/// the one place that can say it without inventing a figure.
#[must_use]
pub fn asset_rows(view: &BalanceView, s: &WalletStrings, locale: &str) -> Vec<AssetRowModel> {
    let unpriced: std::collections::BTreeSet<(u32, String)> = view
        .unpriced_tokens
        .iter()
        .map(|token| {
            (
                token.chain_id,
                token.token_address.clone().unwrap_or_default(),
            )
        })
        .collect();
    view.tokens
        .iter()
        .map(|token| {
            let key = (
                token.chain_id,
                token.token_address.clone().unwrap_or_default(),
            );
            let amount = token.balance.parse::<f64>().unwrap_or(0.0);
            AssetRowModel {
                ticker: SharedString::from(token.symbol.clone()),
                chain: SharedString::from(crate::executor::custom_tokens::network_name(
                    token.chain_id,
                )),
                badge: badge(token.chain_id),
                balance: if view.hidden {
                    SharedString::from(crate::wallet::fixtures::MASK)
                } else {
                    SharedString::from(format_token_amount(amount, NumberPreset::CommaDot, false))
                },
                fiat: if view.hidden {
                    Fiat::Masked
                } else if unpriced.contains(&key) {
                    Fiat::NoPrice(s.no_price.clone())
                } else {
                    Fiat::Value(SharedString::from(format_fiat(
                        amount * token.price_usd.unwrap_or(0.0),
                        "USD",
                        "$",
                        locale,
                        FiatOptions::default(),
                    )))
                },
            }
        })
        .collect()
}

/// The home's network list: every chain the person holds something on, with
/// how many assets are on it, under an "all networks" row.
///
/// Only chains with a holding. A list of twelve networks where eleven say "0"
/// is a list nobody reads — and the eleven are not wrong, they are noise. The
/// count on the "all" row is the number of chains listed, so the two halves of
/// the strip cannot disagree.
#[must_use]
pub fn chain_rows(view: &BalanceView, s: &WalletStrings) -> Vec<ChainRowModel> {
    let mut order: Vec<u32> = Vec::new();
    for token in &view.tokens {
        if !order.contains(&token.chain_id) {
            order.push(token.chain_id);
        }
    }
    let mut rows = vec![ChainRowModel {
        name: s.all_networks.clone(),
        // The neutral dot: "all" is not a chain and must not wear one's colour.
        dot: None,
        count: u32::try_from(order.len()).unwrap_or(u32::MAX),
        selected: true,
    }];
    for chain_id in order {
        rows.push(ChainRowModel {
            name: SharedString::from(crate::executor::custom_tokens::network_name(chain_id)),
            dot: Some(badge(chain_id)),
            count: u32::try_from(
                view.tokens
                    .iter()
                    .filter(|token| token.chain_id == chain_id)
                    .count(),
            )
            .unwrap_or(u32::MAX),
            selected: false,
        });
    }
    rows
}

/// The chain tint for an activity badge.
///
/// Read out of `settings::model::chain_tint` — the same table the network rows
/// use, which is the same table the mocks use. A second colour map for the same
/// chains is how one screen's Polygon stops matching another's.
pub(crate) fn badge(chain_id: u32) -> gpui::Hsla {
    gpui::rgb(crate::settings::model::chain_tint(u64::from(chain_id)).unwrap_or(0x8A_8F_98)).into()
}

/// The activity rows the home preview shows.
///
/// **Headers are dropped here, not filtered out of the core.** `FeedView::rows`
/// interleaves day headers with items because the full Activity screen draws
/// them; the home preview is a short flat list and the mocks draw no headings in
/// it. Asking the core for a different shape would move a render decision into
/// the machine.
#[must_use]
pub fn activity_rows(view: &FeedView, s: &WalletStrings, hidden: bool) -> Vec<ActivityRowModel> {
    view.rows
        .iter()
        .filter_map(|row| match row {
            FeedRow::Header { .. } => None,
            FeedRow::Item { item } => Some(activity_row(view, item, s, hidden)),
        })
        .collect()
}

/// What kind of event a row is.
///
/// `FeedItem` carries only a direction; the RECORD carries the kind, and
/// `FeedView::transactions` is the account-scoped record list the core exposes
/// beside the rows. Looking it up there keeps the dApp distinction the mocks
/// draw — a swap is not "sent", and labelling it so loses the one word that
/// explains where the money went.
pub(crate) fn kind_of(view: &FeedView, item: &FeedItem, incoming: bool) -> ActivityKind {
    let record_kind = view
        .transactions
        .iter()
        .find(|record| record.id == item.id)
        .and_then(|record| record.kind);
    match record_kind {
        Some(FeedTxKind::DappTx) => ActivityKind::Dapp,
        // A signature is not money moving, but the home preview has no row for
        // it; treating it as the direction says is the least wrong of the three
        // shapes available, and the full Activity screen draws it properly.
        _ if incoming => ActivityKind::Received,
        _ => ActivityKind::Sent,
    }
}

pub(crate) fn activity_row(
    view: &FeedView,
    item: &FeedItem,
    s: &WalletStrings,
    hidden: bool,
) -> ActivityRowModel {
    let incoming = item.direction == FeedDirection::In;
    let kind = kind_of(view, item, incoming);

    // Who it was with: the resolved alias if the core has one, else a shortened
    // address, else nothing — a batch row has no single counterparty.
    let who = item
        .alias
        .clone()
        .or_else(|| item.counterparty.as_ref().map(|a| shorten_address(a)));
    let subtitle = who.map_or_else(
        || SharedString::from(""),
        |name| {
            SharedString::from(crate::wallet::fill(
                if incoming { &s.from_name } else { &s.to_name },
                "name",
                &name,
            ))
        },
    );

    ActivityRowModel {
        kind,
        title: match kind {
            ActivityKind::Sent => s.label_sent.clone(),
            ActivityKind::Received => s.label_received.clone(),
            ActivityKind::Dapp => s.label_dapp.clone(),
        },
        subtitle,
        // Privacy masks the FIGURE and keeps the unit — H5's rule, and the same
        // mask the hero uses, because a leak in one surface defeats it
        // everywhere (the core's invariant ④ on the balance side).
        amount: if hidden {
            SharedString::from(crate::wallet::fixtures::MASK)
        } else {
            amount_text(item, incoming)
        },
        unit: SharedString::from(item.symbol.clone()),
        positive: incoming,
        badge: badge(item.chain_id),
    }
}

/// `+120` / `−2`. The minus is U+2212, not a hyphen — the mocks use it and it
/// is what aligns under a digit.
/// The same signed amount the feed row shows, mask included.
///
/// Privacy masks the FIGURE and keeps the unit, on every surface together —
/// the detail panel is one of them, and reading a second flag is how one ends
/// up out of step.
pub(crate) fn amount_text_of(item: &FeedItem, incoming: bool, hidden: bool) -> SharedString {
    if hidden {
        return SharedString::from(crate::wallet::fixtures::MASK);
    }
    SharedString::from(format!("{} {}", amount_text(item, incoming), item.symbol))
}

pub(crate) fn amount_text(item: &FeedItem, incoming: bool) -> SharedString {
    let Some(value) = item.value.as_deref() else {
        // A multi-select batch has mixed tokens and no sum; the core says so by
        // sending no value, and inventing one here would be arithmetic nobody
        // asked for.
        return SharedString::from("");
    };
    // `value` is the HUMAN amount already — "1.5", not raw wei. The web renders
    // it with `trimBalance(item.value)` and the core sums it with `parseFloat`,
    // neither of which scales by `decimals`. Scaling here would print every
    // amount 10^18 times too large, and it would look deliberate.
    let Ok(amount) = value.parse::<f64>() else {
        return SharedString::from("");
    };
    let formatted = format_token_amount(amount, NumberPreset::CommaDot, false);
    SharedString::from(format!(
        "{}{formatted}",
        if incoming { "+" } else { "\u{2212}" }
    ))
}

pub(crate) fn shorten_address(address: &str) -> String {
    if address.len() <= 14 {
        return address.to_owned();
    }
    format!("{}…{}", &address[..6], &address[address.len() - 4..])
}
