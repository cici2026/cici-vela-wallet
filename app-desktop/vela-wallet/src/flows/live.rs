//! The flow panels' display models, built from what the cores decided.
//!
//! The sibling of `flows/fixtures.rs`, never its replacement — the same split
//! `wallet`, `settings` and `contacts` already have. A signed-in person gets
//! these; the gallery and an unsigned window get the mocks, because a mock is a
//! picture of the design and this is somebody's money.
//!
//! ## Three panels, three cores
//!
//! - **Assets** (DT1L / DT4L) reads `BalanceView.tokens` — the core's own
//!   USD-sorted holdings, with `unpriced_tokens` deciding which rows say so.
//! - **Activity** (DA1L) reads `FeedView.rows`, headers included: the full
//!   panel draws day headings, which is exactly the shape the core produces and
//!   the home preview throws away.
//! - **Receive** (DR1L / DR2L) reads the person's real address and the networks
//!   the wallet actually knows, custom ones included.

use gpui::{Hsla, SharedString};

use vela_core::app::activity_feed::{FeedRow, FeedTxStatus, FeedView};
use vela_core::app::balance_dashboard::{BalanceToken, BalanceView};
use vela_core::app::network_admin::BUILTIN_CHAINS;
use vela_core::app::payment_request::PaymentRequestView;
use vela_core::app::receive_watch::ReceiveWatchView;
use vela_core::l10n::currency::{FiatOptions, format_fiat};
use vela_core::l10n::datetime::{Civil, TimePreset, format_time};
use vela_core::l10n::number::{NumberPreset, format_token_amount};

use crate::flows::FlowStrings;
use crate::flows::fixtures::{
    AddressCard, AssetsEmpty, AssetsPanel, DepositEntry as FlowDeposit, FactLead, FactRow,
    HistoryGroup, NetworkRow, ReceiveList, ReceiveQr, StatusChip, StatusTone, TokenMark,
    address_lines,
};
use crate::wallet::fixtures::{AssetRowModel, Fiat, MASK};

/// A chain's colour, from the one table every surface reads.
fn tint(chain_id: u32) -> Hsla {
    gpui::rgb(crate::settings::model::chain_tint(u64::from(chain_id)).unwrap_or(0x8A_8F_98)).into()
}

/// A chain's name, from the one place that derives it.
fn chain_name(chain_id: u32) -> String {
    crate::executor::custom_tokens::network_name(chain_id)
}

// ---------------------------------------------------------------------------
// Assets — DT1L / DT4L
// ---------------------------------------------------------------------------

/// One holding as a row.
///
/// The fiat column has three states and they are not interchangeable:
/// a value, an explicit "no price", and the privacy mask. A token whose price
/// nobody could find must say so rather than show `$0.00` — the core keeps
/// `unpriced_tokens` for precisely this, and rendering it as zero would tell
/// somebody their holding is worthless.
fn asset_row(
    token: &BalanceToken,
    unpriced: bool,
    hidden: bool,
    wallet: &crate::wallet::WalletStrings,
    locale: &str,
) -> AssetRowModel {
    let amount = token.balance.parse::<f64>().unwrap_or(0.0);
    AssetRowModel {
        ticker: SharedString::from(token.symbol.clone()),
        chain: SharedString::from(chain_name(token.chain_id)),
        badge: tint(token.chain_id),
        balance: if hidden {
            SharedString::from(MASK)
        } else {
            SharedString::from(format_token_amount(amount, NumberPreset::CommaDot, false))
        },
        fiat: if hidden {
            Fiat::Masked
        } else if unpriced {
            // The same sentence the balance detail already says. Resolving a
            // second key for one wording is how two surfaces start disagreeing
            // about what "no price" is called.
            Fiat::NoPrice(wallet.no_price.clone())
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
}

/// DT1L, or DT4L when there is nothing to list.
///
/// **The empty state is only shown once the core has ruled.** While the count
/// is in flight (`holdings_loading`, or a balance the core calls unknown) the
/// panel shows an empty list and no guided-empty body — telling somebody their
/// wallet is empty while it is still being read is the assets-panel version of
/// the fake `$0` the hero refuses.
#[must_use]
pub fn assets(
    view: &BalanceView,
    s: &FlowStrings,
    wallet: &crate::wallet::WalletStrings,
    locale: &str,
) -> AssetsPanel {
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

    let rows: Vec<AssetRowModel> = view
        .tokens
        .iter()
        .map(|token| {
            let key = (
                token.chain_id,
                token.token_address.clone().unwrap_or_default(),
            );
            asset_row(token, unpriced.contains(&key), view.hidden, wallet, locale)
        })
        .collect();

    let settled = !view.holdings_loading && !view.balance_unknown;
    AssetsPanel {
        // The chain filter's dots: the chains this person actually holds on,
        // in the order the core sorted them. A filter offering chains with
        // nothing on them is a filter that does nothing.
        filter: Some((
            chain_dots(&view.tokens),
            s.pill_all.clone(),
            s.assets_add.clone(),
        )),
        search_placeholder: s.assets_search.clone(),
        rows: rows.clone(),
        add_by_address: s.add_by_address.clone(),
        empty: (rows.is_empty() && settled).then(|| AssetsEmpty {
            title: s.assets_empty_title.clone(),
            caption: s.assets_empty_caption.clone(),
            cta: s.add_token_title.clone(),
            hint_title: s.not_showing_title.clone(),
            hint_body: s.not_showing_body.clone(),
        }),
    }
}

/// Up to three chain dots for the filter pill, deduped in holdings order.
fn chain_dots(tokens: &[BalanceToken]) -> Vec<Hsla> {
    let mut seen = Vec::new();
    for token in tokens {
        if !seen.contains(&token.chain_id) {
            seen.push(token.chain_id);
        }
        if seen.len() == 3 {
            break;
        }
    }
    seen.into_iter().map(tint).collect()
}

// ---------------------------------------------------------------------------
// Activity — DA1L
// ---------------------------------------------------------------------------

/// The full history, grouped by day.
///
/// Unlike the home preview this KEEPS the core's headers: `FeedView::rows`
/// interleaves them because this panel is what they were computed for. The
/// label is the shell's, because "Today" is a fact about the reader's clock and
/// `vela-core` ships no timezone database.
#[must_use]
pub fn history(
    view: &FeedView,
    s: &FlowStrings,
    wallet: &crate::wallet::WalletStrings,
    hidden: bool,
) -> Vec<HistoryGroup> {
    let mut groups: Vec<HistoryGroup> = Vec::new();
    for row in &view.rows {
        match row {
            FeedRow::Header { day_start_ms, .. } => groups.push(HistoryGroup {
                label: day_label(*day_start_ms, s),
                rows: Vec::new(),
            }),
            FeedRow::Item { item } => {
                let model = crate::wallet::live::activity_row(view, item, wallet, hidden);
                // A feed that opened with an item rather than a header is not a
                // shape the core produces, but drawing the row is better than
                // dropping somebody's transaction over a missing heading.
                if let Some(group) = groups.last_mut() {
                    group.rows.push(model);
                } else {
                    groups.push(HistoryGroup {
                        label: s.today.clone(),
                        rows: vec![model],
                    });
                }
            }
        }
    }
    // A day header the core emitted with nothing under it would draw as a
    // heading over blank space.
    groups.retain(|group| !group.rows.is_empty());
    groups
}

/// The transaction ids the history panel draws, in render order.
///
/// The page binds one listener per id and `panels::history` hands them out in
/// the same walk, so row N's listener opens row N's transaction. Two walks that
/// could disagree would open the wrong record, which on a money screen is worse
/// than opening nothing.
#[must_use]
pub fn history_ids(view: &FeedView) -> Vec<String> {
    let mut seen_header = false;
    let mut ids = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for row in &view.rows {
        match row {
            FeedRow::Header { .. } => {
                // The previous group's rows are kept only if it had any — the
                // same retain `history` applies, walked the same way.
                ids.append(&mut pending);
                seen_header = true;
            }
            FeedRow::Item { item } => {
                if seen_header {
                    pending.push(item.id.clone());
                } else {
                    ids.push(item.id.clone());
                }
            }
        }
    }
    ids.append(&mut pending);
    ids
}

/// "Today" / "Yesterday" / the date.
///
/// Compared against the shell's own local day boundary, which is the same
/// function the executor stamps records with — asking two different questions
/// about which day it is here is how a row lands under the wrong heading.
fn day_label(day_start_ms: f64, s: &FlowStrings) -> SharedString {
    let today = crate::executor::day_start_ms(crate::executor::now_ms());
    const DAY_MS: f64 = 86_400_000.0;
    if (day_start_ms - today).abs() < DAY_MS / 2.0 {
        return s.today.clone();
    }
    if (day_start_ms - (today - DAY_MS)).abs() < DAY_MS / 2.0 {
        return s.yesterday.clone();
    }
    #[allow(clippy::cast_possible_truncation, reason = "an epoch in ms")]
    let civil = Civil::from_unix_millis(day_start_ms as i64, 0);
    SharedString::from(vela_core::l10n::datetime::format_date(
        &civil,
        vela_core::l10n::datetime::DatePreset::Iso,
    ))
}

/// One transaction, in detail — DA2L / DA3L.
///
/// `None` when the id names nothing: a row can be deleted while its panel is
/// open, and drawing a stale detail over a record that no longer exists is
/// worse than closing the column.
#[must_use]
pub fn tx_detail(
    view: &FeedView,
    id: &str,
    s: &FlowStrings,
    hidden: bool,
    locale: &str,
) -> Option<crate::flows::fixtures::TxDetail> {
    let item = view.rows.iter().find_map(|row| match row {
        FeedRow::Item { item } if item.id == id => Some(item),
        _ => None,
    })?;
    let incoming = item.direction == vela_core::app::activity_feed::FeedDirection::In;
    let record = view.transactions.iter().find(|record| record.id == item.id);

    let mut facts = Vec::new();
    // Who it was with. The identicon is seeded by the ADDRESS even when a name
    // is known — the avatar is how somebody checks the name is on the address
    // they meant, so seeding it from the name would defeat its purpose.
    if let Some(counterparty) = item.counterparty.as_ref() {
        let named = item.alias.clone();
        facts.push(FactRow {
            label: if incoming {
                s.detail_from.clone()
            } else {
                s.detail_to.clone()
            },
            value: SharedString::from(
                named
                    .clone()
                    .unwrap_or_else(|| crate::wallet::live::shorten_address(counterparty)),
            ),
            lead: FactLead::Identicon(SharedString::from(counterparty.clone())),
            // A name is prose; an address is a string somebody compares
            // character by character, and that needs the mono face.
            mono: named.is_none(),
            copyable: true,
        });
    }
    facts.push(FactRow {
        label: s.detail_chain.clone(),
        value: SharedString::from(chain_name(item.chain_id)),
        lead: FactLead::Token(TokenMark {
            ticker: SharedString::from(item.symbol.clone()),
            badge: tint(item.chain_id),
        }),
        mono: false,
        copyable: false,
    });
    facts.push(FactRow {
        label: s.detail_date.clone(),
        value: SharedString::from(stamp(item.timestamp, s, locale)),
        lead: FactLead::None,
        mono: false,
        copyable: false,
    });
    // Only if there IS one. An empty hash row on an off-chain signature invites
    // "which transaction?" — the same reason the mock omits the contract row on
    // a native transfer.
    if let Some(hash) = item.tx_hash.as_ref().filter(|hash| !hash.is_empty()) {
        facts.push(FactRow {
            label: s.detail_hash.clone(),
            value: SharedString::from(hash.clone()),
            lead: FactLead::None,
            mono: true,
            copyable: true,
        });
    }

    let status = record.map_or(FeedTxStatus::Confirmed, |record| record.status);
    Some(crate::flows::fixtures::TxDetail {
        title: SharedString::from(crate::wallet::fill(
            if incoming {
                &s.tx_label_received
            } else {
                &s.tx_label_sent
            },
            "symbol",
            &item.symbol,
        )),
        status: StatusChip {
            text: match status {
                FeedTxStatus::Confirmed => s.status_confirmed.clone(),
                // A pending or failed transfer must not wear the confirmed
                // chip. The words are the feed's, which already has them.
                FeedTxStatus::Pending => s.status_pending.clone(),
                FeedTxStatus::Failed => s.status_failed.clone(),
            },
            tone: match status {
                FeedTxStatus::Confirmed => StatusTone::Success,
                FeedTxStatus::Pending => StatusTone::Info,
                FeedTxStatus::Failed => StatusTone::Error,
            },
        },
        amount: crate::wallet::live::amount_text_of(item, incoming, hidden),
        // The core already valued it, stablecoin fallback and all. `0` means
        // unknown rather than free, so it shows nothing instead of `$0.00`.
        fiat: if hidden || item.usd_value <= 0.0 {
            SharedString::from("")
        } else {
            SharedString::from(format_fiat(
                item.usd_value,
                "USD",
                "$",
                locale,
                FiatOptions::default(),
            ))
        },
        positive: incoming,
        facts,
        view_on_explorer: s.view_on_explorer.clone(),
    })
}

/// A transaction's wall clock: "Today 11:20", "Yesterday 14:02", or the date.
fn stamp(timestamp_sec: f64, s: &FlowStrings, locale: &str) -> String {
    let epoch_ms = timestamp_sec * 1000.0;
    let civil = crate::executor::local_civil(epoch_ms);
    let clock = format_time(&civil, TimePreset::H24, locale);
    let day = day_label(crate::executor::day_start_ms(epoch_ms), s);
    format!("{day} {clock}")
}

// ---------------------------------------------------------------------------
// Receive — DR1L / DR2L
// ---------------------------------------------------------------------------

/// Every network this wallet can be paid on, each showing the person's own
/// address.
///
/// One address across every EVM chain is the whole point of the Safe: the rows
/// differ by network, never by address, and a person who reads two of them must
/// see the same string twice.
#[must_use]
pub fn receive_list(address: &str, s: &FlowStrings) -> ReceiveList {
    let rows: Vec<NetworkRow> = receivable_chains()
        .into_iter()
        .map(|(chain_id, symbol)| NetworkRow {
            name: SharedString::from(chain_name(chain_id)),
            code: SharedString::from(symbol),
            badge: tint(chain_id),
            address: SharedString::from(shorten(address)),
        })
        .collect();
    ReceiveList {
        subtitle: SharedString::from(crate::wallet::fill(
            &s.networks_line,
            "count",
            &rows.len().to_string(),
        )),
        search_placeholder: s.receive_search.clone(),
        rows,
    }
}

/// One network's QR, with the person's real address under it.
#[must_use]
pub fn receive_qr(
    address: &str,
    name: &str,
    chain_id: u32,
    watch: &ReceiveWatchView,
    pay: &PaymentRequestView,
    s: &FlowStrings,
    locale: &str,
) -> ReceiveQr {
    let network = chain_name(chain_id);
    let symbol = receivable_chains()
        .into_iter()
        .find(|(id, _)| *id == chain_id)
        .map_or_else(|| network.clone(), |(_, symbol)| symbol);
    let lines = address_lines(address);
    ReceiveQr {
        title: SharedString::from(crate::wallet::fill(
            &s.qr_title_network,
            "network",
            &network,
        )),
        // The contract line is DR3L's — an ASSET's QR names the token it is
        // for. A network QR has no contract, and inventing one would put a
        // token address under a code that is not for a token.
        contract: None,
        account: AddressCard {
            name: SharedString::from(name.to_owned()),
            // The identicon seed is the ADDRESS, not the name: two accounts a
            // person named the same thing must not draw the same avatar, and an
            // avatar is how somebody checks they are looking at the right one.
            seed: SharedString::from(address.to_owned()),
            lines: (SharedString::from(lines.0), SharedString::from(lines.1)),
        },
        // WHAT the code says is the core's decision, not this file's: in
        // address mode `qr_value` is the bare recipient, and in request mode it
        // is the EIP-681 URI with the amount in it. Encoding the address here
        // would work today and silently ignore an amount the moment the request
        // builder lands.
        qr_payload: (!pay.qr_value.is_empty()).then(|| SharedString::from(pay.qr_value.clone())),
        centre: TokenMark {
            ticker: SharedString::from(symbol),
            badge: tint(chain_id),
        },
        warning: s.warning_reminder.clone(),
        save_image: s.save_image.clone(),
        view_on_explorer: s.view_on_explorer.clone(),
        deposits: deposits(watch, locale),
    }
}

/// What landed while this code was open.
///
/// The core decides WHAT counts as a deposit — the baseline, the comparison,
/// the debounce. This turns its verdict into words: the local wall clock, the
/// amount with its sign, and the network and value beside it.
///
/// `detected` gates the section rather than `deposits.is_empty()`, because they
/// are the core's two separate answers and only the first means "announce
/// this". A list with nothing in it is not a celebration.
fn deposits(view: &ReceiveWatchView, locale: &str) -> Vec<FlowDeposit> {
    if !view.detected {
        return Vec::new();
    }
    view.deposits
        .iter()
        .map(|entry| FlowDeposit {
            time: SharedString::from(format_time(
                &crate::executor::local_civil(entry.at_epoch_ms),
                TimePreset::H24,
                locale,
            )),
            rows: entry
                .items
                .iter()
                .map(|item| {
                    (
                        SharedString::from(format!(
                            "+{} {}",
                            format_token_amount(item.amount, NumberPreset::CommaDot, false),
                            item.symbol
                        )),
                        SharedString::from(match item.usd {
                            // An unpriced arrival still says which chain it came
                            // in on. Printing `$0.00` beside it would be the
                            // assets panel's mistake on a happier screen.
                            Some(usd) => format!(
                                "{}  {}",
                                chain_name(item.chain_id),
                                format_fiat(usd, "USD", "$", locale, FiatOptions::default())
                            ),
                            None => chain_name(item.chain_id),
                        }),
                    )
                })
                .collect(),
        })
        .collect()
}

/// The chains a payment can arrive on: the built-ins plus whatever the person
/// added. The same list the balance fetch reads, for the same reason — a
/// network nobody can be paid on is a network nobody has.
pub fn receivable_chains() -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = BUILTIN_CHAINS
        .iter()
        .map(|chain| (chain.chain_id, chain.native_symbol.to_owned()))
        .collect();
    if let Ok(Some(serde_json::Value::Array(items))) =
        crate::executor::storage::read_value(crate::executor::storage::KEY_CUSTOM_NETWORKS)
    {
        for item in items {
            let Some(chain_id) = item
                .get("chainId")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
            else {
                continue;
            };
            if out.iter().any(|(id, _)| *id == chain_id) {
                continue;
            }
            out.push((
                chain_id,
                item.get("nativeSymbol")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            ));
        }
    }
    out
}

fn shorten(address: &str) -> String {
    if address.len() <= 14 {
        return address.to_owned();
    }
    format!("{}…{}", &address[..6], &address[address.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_host::CoreHost;
    use vela_core::app::balance_dashboard::{BalanceDashboard, Event as BalanceEvent};

    fn strings() -> FlowStrings {
        FlowStrings::resolve(&crate::loc::Loc::from_env())
    }

    fn wallet_strings() -> crate::wallet::WalletStrings {
        crate::wallet::WalletStrings::resolve(&crate::loc::Loc::from_env())
    }

    /// A real `PaymentRequestView`, from a booted core.
    fn pay_view() -> PaymentRequestView {
        use vela_core::app::payment_request::{Event as PayEvent, PaymentRequest};
        let mut host = CoreHost::<PaymentRequest>::new();
        let _ = host.dispatch(PayEvent::Start {
            account: "0xabc".to_owned(),
            recipient: "0xabc".to_owned(),
            base_url: "https://getvela.app".to_owned(),
        });
        host.view()
    }

    /// A real `BalanceView`, taken from a booted core rather than hand-written.
    fn view() -> BalanceView {
        let mut host = CoreHost::<BalanceDashboard>::new();
        let _ = host.dispatch(BalanceEvent::AccountChanged {
            address: "0xabc".to_owned(),
        });
        host.view()
    }

    fn token(chain_id: u32, symbol: &str, balance: &str, price: Option<f64>) -> BalanceToken {
        BalanceToken {
            chain_id,
            symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            balance: balance.to_owned(),
            decimals: 18,
            token_address: None,
            price_usd: price,
            spam: false,
        }
    }

    /// A holding renders its amount and its value; one nobody could price says
    /// so instead of showing zero.
    #[test]
    fn an_unpriced_holding_says_so_rather_than_showing_nothing() {
        let mut view = view();
        view.tokens = vec![
            token(100, "xDAI", "0.75897", Some(1.0)),
            token(143, "MON", "12.5", None),
        ];
        view.unpriced_tokens = vec![token(143, "MON", "12.5", None)];

        let panel = assets(&view, &strings(), &wallet_strings(), "en-US");
        assert_eq!(panel.rows.len(), 2);
        assert_eq!(panel.rows[0].ticker, "xDAI");
        assert_eq!(panel.rows[0].chain, "Gnosis");
        assert!(matches!(panel.rows[0].fiat, Fiat::Value(_)));
        // The one the core could not price. `$0.00` here would tell somebody
        // their holding is worthless.
        assert!(
            matches!(panel.rows[1].fiat, Fiat::NoPrice(_)),
            "an unpriced holding must not render as a value"
        );
        assert!(panel.empty.is_none(), "there are rows");
    }

    /// Privacy masks the figure AND the value, on this surface as on the hero.
    #[test]
    fn hiding_the_balance_hides_it_here_too() {
        let mut view = view();
        view.tokens = vec![token(100, "xDAI", "0.75897", Some(1.0))];
        view.hidden = true;

        let panel = assets(&view, &strings(), &wallet_strings(), "en-US");
        assert_eq!(panel.rows[0].balance, MASK);
        assert!(matches!(panel.rows[0].fiat, Fiat::Masked));
        // The unit survives — H5's rule. The figure is what goes.
        assert_eq!(panel.rows[0].ticker, "xDAI");
    }

    /// An empty wallet and a wallet still being counted are different screens.
    #[test]
    fn the_guided_empty_waits_until_the_core_has_ruled() {
        let mut counting = view();
        counting.tokens = Vec::new();
        counting.balance_unknown = true;
        assert!(
            assets(&counting, &strings(), &wallet_strings(), "en-US")
                .empty
                .is_none(),
            "still counting: no 'your wallet is empty'"
        );

        let mut loading = view();
        loading.tokens = Vec::new();
        loading.balance_unknown = false;
        loading.holdings_loading = true;
        assert!(
            assets(&loading, &strings(), &wallet_strings(), "en-US")
                .empty
                .is_none()
        );

        let mut settled = view();
        settled.tokens = Vec::new();
        settled.balance_unknown = false;
        settled.holdings_loading = false;
        assert!(
            assets(&settled, &strings(), &wallet_strings(), "en-US")
                .empty
                .is_some(),
            "the core ruled: genuinely empty"
        );
    }

    /// The filter dots are the chains this person actually holds on.
    #[test]
    fn the_chain_filter_offers_only_chains_with_something_on_them() {
        let mut view = view();
        view.tokens = vec![
            token(100, "xDAI", "1", Some(1.0)),
            token(100, "USDC", "1", Some(1.0)),
            token(1, "ETH", "1", Some(2000.0)),
            token(56, "BNB", "1", Some(700.0)),
            token(137, "POL", "1", Some(0.4)),
        ];
        let panel = assets(&view, &strings(), &wallet_strings(), "en-US");
        let dots = panel
            .filter
            .as_ref()
            .map(|(dots, _, _)| dots.len())
            .unwrap_or_default();
        // Deduped by chain (Gnosis appears twice) and capped at three.
        assert_eq!(dots, 3);
    }

    /// Every network shows the SAME address — that is what a Safe is.
    #[test]
    fn every_receive_row_shows_one_address() {
        crate::executor::storage::tests::with_temp_state("flows-receive", || {
            const ADDR: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";
            let list = receive_list(ADDR, &strings());
            assert_eq!(list.rows.len(), BUILTIN_CHAINS.len());
            let first = list.rows[0].address.clone();
            for row in &list.rows {
                assert_eq!(row.address, first, "one address, every chain");
            }
            assert!(list.subtitle.contains(&BUILTIN_CHAINS.len().to_string()));

            // A network the person added joins the list, named by their name.
            let networks = serde_json::json!([
                { "chainId": 7_777_777, "nativeSymbol": "TST", "displayName": "My testnet" }
            ]);
            if crate::executor::storage::write_value(
                crate::executor::storage::KEY_CUSTOM_NETWORKS,
                networks,
            )
            .is_err()
            {
                unreachable!("could not seed");
            }
            let list = receive_list(ADDR, &strings());
            assert_eq!(list.rows.len(), BUILTIN_CHAINS.len() + 1);
            assert!(list.rows.iter().any(|row| row.name == "My testnet"));
        });
    }

    /// The QR card carries the real address, and the identicon is seeded by it.
    #[test]
    fn the_qr_card_is_seeded_by_the_address_not_the_name() {
        crate::executor::storage::tests::with_temp_state("flows-qr", || {
            const ADDR: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";
            let quiet = ReceiveWatchView {
                detected: false,
                deposits: Vec::new(),
            };
            let mut pay = pay_view();
            pay.qr_value = ADDR.to_owned();
            let qr = receive_qr(
                ADDR,
                "Everyday wallet",
                100,
                &quiet,
                &pay,
                &strings(),
                "en-US",
            );
            assert_eq!(qr.account.name, "Everyday wallet");
            assert_eq!(qr.account.seed, ADDR, "two same-named accounts must differ");
            // The two halves rejoin into the address the person will paste.
            let joined = format!("{}{}", qr.account.lines.0, qr.account.lines.1);
            assert_eq!(joined, ADDR);
            assert!(qr.title.contains("Gnosis"));
            assert_eq!(qr.centre.ticker, "xDAI");
            // A network code is not a token code: no contract line here.
            assert_eq!(qr.contract, None);
        });
    }

    /// A transaction's detail, and the row order the listeners are bound in.
    #[test]
    fn a_transaction_opens_its_own_detail_and_the_row_order_matches() {
        crate::executor::storage::tests::with_temp_state("flows-tx-detail", || {
            use vela_core::app::activity_feed::{
                ActivityFeed, Event as FeedEvent, FeedDirection, FeedItem, FeedTxRecord,
            };

            let mut host = CoreHost::<ActivityFeed>::new();
            let _ = host.dispatch(FeedEvent::AccountSwitched {
                address: "0xme".to_owned(),
            });
            let today = crate::executor::day_start_ms(crate::executor::now_ms());
            let item = |id: &str, incoming: bool| FeedItem {
                id: id.to_owned(),
                direction: if incoming {
                    FeedDirection::In
                } else {
                    FeedDirection::Out
                },
                counterparty: Some("0xAbCdEf0000000000000000000000000000000001".to_owned()),
                alias: None,
                value: Some("1.5".to_owned()),
                symbol: "xDAI".to_owned(),
                decimals: Some(18),
                usd_value: 1.5,
                chain_id: 100,
                timestamp: today / 1000.0 + 3600.0,
                day_start_ms: today,
                tx_hash: Some("0xdead".to_owned()),
                batch: None,
            };
            let view = FeedView {
                rows: vec![
                    FeedRow::Header {
                        id: "day-0".to_owned(),
                        day_start_ms: today,
                        timestamp: today / 1000.0,
                    },
                    FeedRow::Item {
                        item: item("a", true),
                    },
                    FeedRow::Item {
                        item: item("b", false),
                    },
                ],
                transactions: vec![FeedTxRecord {
                    id: "b".to_owned(),
                    user_op_hash: String::new(),
                    tx_hash: "0xdead".to_owned(),
                    from: "0xme".to_owned(),
                    to: "0xAbCdEf0000000000000000000000000000000001".to_owned(),
                    to_name: None,
                    value: "1.5".to_owned(),
                    symbol: "xDAI".to_owned(),
                    decimals: 18,
                    logo_urls: None,
                    chain_id: 100,
                    timestamp: today / 1000.0 + 3600.0,
                    day_start_ms: today,
                    status: FeedTxStatus::Pending,
                    kind: None,
                    usd: None,
                }],
                ..host.view()
            };

            // The listeners are bound in the order the rows draw.
            assert_eq!(history_ids(&view), vec!["a".to_owned(), "b".to_owned()]);

            let s = strings();
            let received = tx_detail(&view, "a", &s, false, "en-US")
                .unwrap_or_else(|| unreachable!("row a exists"));
            assert!(received.positive);
            assert_eq!(received.amount, "+1.5 xDAI");
            assert_eq!(received.fiat, "$1.50");
            // No stored record for "a", so the status is the confirmed default
            // rather than a guess at something worse.
            assert_eq!(received.status.text, s.status_confirmed);
            // From (not To) for a receipt, plus chain, date and hash.
            assert_eq!(received.facts[0].label, s.detail_from);
            assert_eq!(received.facts[1].label, s.detail_chain);
            assert_eq!(received.facts[2].label, s.detail_date);
            assert_eq!(received.facts[3].label, s.detail_hash);
            assert!(
                received.facts[3].mono,
                "a hash is compared character by character"
            );

            // The sent one, whose stored record says pending — it must NOT
            // wear the confirmed chip.
            let sent = tx_detail(&view, "b", &s, false, "en-US")
                .unwrap_or_else(|| unreachable!("row b exists"));
            assert!(!sent.positive);
            assert_eq!(sent.facts[0].label, s.detail_to);
            assert_eq!(sent.status.text, s.status_pending);
            assert!(matches!(sent.status.tone, StatusTone::Info));

            // Privacy masks the figure here as everywhere.
            let hidden = tx_detail(&view, "a", &s, true, "en-US")
                .unwrap_or_else(|| unreachable!("row a exists"));
            assert_eq!(hidden.amount, crate::wallet::fixtures::MASK);
            assert_eq!(hidden.fiat, "");

            // A row that no longer exists has no detail — the panel closes
            // rather than showing a stale one.
            assert!(tx_detail(&view, "gone", &s, false, "en-US").is_none());
        });
    }

    /// The QR encodes what the CORE says, and a live one is never the demo
    /// pattern.
    #[test]
    fn the_code_carries_the_cores_payload_not_the_shells_guess() {
        crate::executor::storage::tests::with_temp_state("flows-qr-payload", || {
            const ADDR: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";
            let quiet = ReceiveWatchView {
                detected: false,
                deposits: Vec::new(),
            };

            // A booted `payment_request` in address mode answers the recipient.
            let mut pay = pay_view();
            pay.qr_value = ADDR.to_owned();
            let qr = receive_qr(ADDR, "Me", 100, &quiet, &pay, &strings(), "en-US");
            assert_eq!(qr.qr_payload.as_deref(), Some(ADDR));

            // Request mode puts an EIP-681 URI in the same field, and this file
            // must forward it rather than re-deriving the address.
            pay.qr_value = "ethereum:0x88cC@100?value=1.5e18".to_owned();
            let request = receive_qr(ADDR, "Me", 100, &quiet, &pay, &strings(), "en-US");
            assert_eq!(
                request.qr_payload.as_deref(),
                Some("ethereum:0x88cC@100?value=1.5e18")
            );

            // Before the core has ruled there is nothing to encode, and drawing
            // a decorative code on a screen meant to be scanned is the failure
            // this field exists to end.
            pay.qr_value = String::new();
            let unruled = receive_qr(ADDR, "Me", 100, &quiet, &pay, &strings(), "en-US");
            assert_eq!(unruled.qr_payload, None);
        });
    }

    /// A deposit is announced when the core says one landed — and only then.
    #[test]
    fn an_arrival_is_announced_only_when_the_core_says_one_landed() {
        use vela_core::app::receive_watch::{DepositEntry, DepositItem};

        let entry = DepositEntry {
            // 2026-09-04T12:34:56Z, shifted into whatever zone this machine is
            // in — the assertion is on shape, not on a clock the test cannot
            // know.
            at_epoch_ms: 1_788_525_296_000.0,
            items: vec![
                DepositItem {
                    symbol: "xDAI".to_owned(),
                    amount: 1.5,
                    chain_id: 100,
                    usd: Some(1.5),
                },
                DepositItem {
                    symbol: "MON".to_owned(),
                    amount: 12.0,
                    chain_id: 143,
                    usd: None,
                },
            ],
        };

        // A list the core has NOT called detected is not a celebration.
        let quiet = ReceiveWatchView {
            detected: false,
            deposits: vec![entry.clone()],
        };
        assert!(
            deposits(&quiet, "en-US").is_empty(),
            "undetected must not announce"
        );

        let landed = ReceiveWatchView {
            detected: true,
            deposits: vec![entry],
        };
        let announced = deposits(&landed, "en-US");
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].rows.len(), 2);
        assert_eq!(announced[0].rows[0].0, "+1.5 xDAI");
        assert!(announced[0].rows[0].1.starts_with("Gnosis"));
        assert!(announced[0].rows[0].1.contains("$1.50"));
        // An unpriced arrival still says which chain it came in on; `$0.00`
        // beside it would be the assets panel's mistake on a happier screen.
        assert_eq!(announced[0].rows[1].0, "+12 MON");
        assert_eq!(announced[0].rows[1].1, "Monad");
        // The time is a wall clock, not an epoch.
        assert!(
            announced[0].time.contains(':') && announced[0].time.len() <= 8,
            "not a clock: {}",
            announced[0].time
        );
    }

    /// The assets panel, end to end, against the golden Safe.
    ///
    /// The hero says one number; this is the screen that has to justify it. A
    /// total nobody can break down is a total nobody can check.
    #[test]
    #[ignore = "reads every chain for a real address"]
    fn the_assets_panel_lists_what_the_hero_totals() {
        use crate::resident::{Answer, Machine};

        crate::executor::storage::tests::with_temp_state("flows-assets-live", || {
            crate::executor::chain_tokens::invalidate();
            crate::executor::chainlink::invalidate();
            const GOLDEN: &str = "0x88cCA0EeDbF2C4426110bbFc998F048689266894";

            let mut host = CoreHost::<BalanceDashboard>::new();
            let mut pending = host.dispatch(BalanceEvent::AccountChanged {
                address: GOLDEN.to_owned(),
            });
            for _ in 0..64 {
                let Some(next) = pending.pop() else { break };
                let result = match BalanceDashboard::perform(&next.operation) {
                    Answer::Now(result) | Answer::After(_, result) => result,
                    Answer::Blocking(work) => work(),
                };
                pending.extend(host.resolve(next.id, result));
            }
            let view = host.view();
            let panel = assets(&view, &strings(), &wallet_strings(), "en-US");

            for row in &panel.rows {
                println!(
                    "    {:>10} {:<8} {:<12} {}",
                    row.balance,
                    row.ticker,
                    row.chain,
                    match &row.fiat {
                        Fiat::Value(v) => v.to_string(),
                        Fiat::NoPrice(v) => v.to_string(),
                        Fiat::Masked => "•••".to_owned(),
                    }
                );
            }

            assert!(
                !panel.rows.is_empty(),
                "the hero has a total; this has rows"
            );
            assert!(
                panel.empty.is_none(),
                "a funded wallet must not be told it is empty"
            );
            let gnosis = panel
                .rows
                .iter()
                .find(|row| row.chain == "Gnosis" && row.ticker == "xDAI")
                .unwrap_or_else(|| unreachable!("Gnosis xDAI is missing"));
            // The row's own figure, not base units: the same bug the hero test
            // pins, one surface further out.
            let amount: f64 = gnosis
                .balance
                .replace(',', "")
                .parse()
                .unwrap_or_else(|_| unreachable!("not an amount: {}", gnosis.balance));
            assert!(amount > 0.0 && amount < 1_000.0, "implausible: {amount}");
            assert!(matches!(gnosis.fiat, Fiat::Value(_)), "xDAI is priced");

            // And the total the hero shows is the sum of what this lists.
            let listed: f64 = view
                .tokens
                .iter()
                .map(vela_core::app::balance_dashboard::token_usd_value)
                .sum();
            let hero = view
                .display_total_usd
                .unwrap_or_else(|| unreachable!("no total"));
            assert!(
                (listed - hero).abs() < 0.01,
                "the panel lists {listed} and the hero says {hero}"
            );
        });
    }

    /// Day headers become headings, and a heading with nothing under it is not
    /// drawn.
    #[test]
    fn the_history_keeps_the_headers_the_home_preview_drops() {
        use vela_core::app::activity_feed::{
            ActivityFeed, Event as FeedEvent, FeedDirection, FeedItem,
        };

        let mut host = CoreHost::<ActivityFeed>::new();
        let _ = host.dispatch(FeedEvent::AccountSwitched {
            address: "0xme".to_owned(),
        });
        let today = crate::executor::day_start_ms(crate::executor::now_ms());
        let item = |id: &str, day: f64| FeedItem {
            id: id.to_owned(),
            direction: FeedDirection::In,
            counterparty: Some("0xAbCdEf0000000000000000000000000000000001".to_owned()),
            alias: None,
            value: Some("1.5".to_owned()),
            symbol: "xDAI".to_owned(),
            decimals: Some(18),
            usd_value: 0.0,
            chain_id: 100,
            timestamp: day / 1000.0,
            day_start_ms: day,
            tx_hash: None,
            batch: None,
        };
        let view = FeedView {
            rows: vec![
                FeedRow::Header {
                    id: "day-0".to_owned(),
                    day_start_ms: today,
                    timestamp: today / 1000.0,
                },
                FeedRow::Item {
                    item: item("a", today),
                },
                FeedRow::Header {
                    id: "day-1".to_owned(),
                    day_start_ms: today - 86_400_000.0,
                    timestamp: (today - 86_400_000.0) / 1000.0,
                },
                FeedRow::Item {
                    item: item("b", today - 86_400_000.0),
                },
                // A heading the core emitted with nothing under it.
                FeedRow::Header {
                    id: "day-9".to_owned(),
                    day_start_ms: today - 9.0 * 86_400_000.0,
                    timestamp: (today - 9.0 * 86_400_000.0) / 1000.0,
                },
            ],
            ..host.view()
        };

        let s = strings();
        let groups = history(&view, &s, &wallet_strings(), false);
        assert_eq!(groups.len(), 2, "the empty heading is not drawn");
        assert_eq!(groups[0].label, s.today);
        assert_eq!(groups[1].label, s.yesterday);
        assert_eq!(groups[0].rows.len(), 1);
        assert_eq!(groups[0].rows[0].amount, "+1.5");

        // Hidden masks every figure here, as on the hero and the home preview.
        let hidden = history(&view, &s, &wallet_strings(), true);
        assert_eq!(hidden[0].rows[0].amount, crate::wallet::fixtures::MASK);
        assert_eq!(hidden[0].rows[0].unit, "xDAI");
    }
}
