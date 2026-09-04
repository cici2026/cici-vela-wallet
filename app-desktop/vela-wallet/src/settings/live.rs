//! The settings screen's display models, built from what the cores decided.
//!
//! The **sibling** of `fixtures.rs`, never its replacement. Both produce the
//! same shapes; the screen picks which one feeds it, and the gallery always
//! picks the fixture. That is what keeps every drawn state reviewable after the
//! real data arrives — and what makes "the galleries are unchanged" something a
//! diff can prove rather than something a reviewer has to eyeball.

use gpui::SharedString;

use crate::settings::SettingsStrings;
use crate::settings::fixtures::{Pill, Tone};
use crate::settings::model::{NetworkRowModel, chain_tint, lettermark};

use vela_core::app::display_currency::CurrencyView;
use vela_core::app::network_admin::{NetProbeHealth, NetServiceHealth, NetView};
use vela_core::l10n::currency::{FiatOptions, format_fiat};

/// The sample figure the 本地化 mock prints beside the currency code.
const SAMPLE: f64 = 1234.56;

/// Symbols for the codes a person can actually reach today.
///
/// Deliberately small. The shell owns the currency catalog — that is the core's
/// division of labour, stated in `display_currency.rs` — but a full catalog is
/// only *useful* once rates exist, because until then no code but USD can be
/// priced at all. It arrives with the rates in spec 031. Unknown codes fall back
/// to the code itself, which `format_fiat` spaces correctly (`CHF 1,234.56`)
/// because CLDR's `currencySpacing` keys off the symbol being alphabetic.
fn symbol_for(code: &str) -> &str {
    match code {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" | "CNY" => "¥",
        "KRW" => "₩",
        "VND" => "₫",
        "INR" => "₹",
        other => other,
    }
}

/// The value the 货币 row shows.
///
/// Two cases, and the split is the core's rule rather than a style choice:
///
/// - **priced** — the code and a sample formatted in it.
/// - **unpriced** (`rate: None`) — the code **alone**. Not the code beside a USD
///   figure wearing its symbol, which is the specific lie `rate: None` exists to
///   prevent: "a rate of 1 is a claim (1 USD = 1 CNY)". Showing `¥1,234.56` for
///   an unpriced JPY would assert an exchange rate nobody obtained.
#[must_use]
pub fn currency_row_value(view: &CurrencyView, locale: &str) -> SharedString {
    match view.rate {
        Some(rate) => {
            let sample = format_fiat(
                SAMPLE * rate,
                &view.code,
                symbol_for(&view.code),
                locale,
                FiatOptions::default(),
            );
            SharedString::from(format!("{} · {sample}", view.code))
        }
        None => SharedString::from(view.code.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core_host::CoreHost;
    use vela_core::app::network_admin::{Event as NetEvent, NetNetworkRow, NetworkAdmin};

    /// A real `NetView` with the rows substituted.
    ///
    /// Built by booting the actual core rather than hand-constructing one: the
    /// wizard, endpoint and provider sub-views have their own invariants, and a
    /// literal I typed would be a guess about them that drifts the first time
    /// they change.
    fn view_with(networks: Vec<NetNetworkRow>) -> NetView {
        let mut host = CoreHost::<NetworkAdmin>::new();
        let _ = host.dispatch(NetEvent::Started);
        NetView {
            loaded: true,
            networks,
            ..host.view()
        }
    }

    fn net_row(
        chain_id: u32,
        name: &str,
        custom: bool,
        health: Option<NetProbeHealth>,
    ) -> NetNetworkRow {
        NetNetworkRow {
            id: format!("chain-{chain_id}"),
            chain_id,
            display_name: name.to_owned(),
            native_symbol: "ETH".to_owned(),
            is_custom: custom,
            rpc_url: String::new(),
            explorer_url: String::new(),
            bundler_url: String::new(),
            rpc_health: health,
            explorer_health: None,
            rpc_chain_mismatch: None,
            rpc_save_deferred: false,
        }
    }

    /// A live Ethereum row is tinted by the same constant the mock is. The
    /// alternative — a second colour table in the live path — is how two
    /// renderings of one network start disagreeing.
    #[test]
    fn a_known_chain_borrows_the_mock_s_brand_colour() {
        let view = view_with(vec![net_row(1, "Ethereum", false, None)]);
        let rows = network_rows(&view);
        let fixture = crate::settings::fixtures::network("ethereum");
        assert_eq!(rows[0].color, fixture.color);
        assert_eq!(rows[0].letter, SharedString::from("E"));
    }

    /// A chain nobody drew gets the neutral, not an invented brand colour.
    #[test]
    fn an_undrawn_chain_gets_the_neutral_tint() {
        let view = view_with(vec![net_row(31_337, "Anvil", true, None)]);
        assert_eq!(network_rows(&view)[0].color, UNTINTED);
    }

    /// Three different core states draw no badge, and the row must not
    /// distinguish them by inventing a zero.
    #[test]
    fn only_a_completed_probe_draws_a_latency_badge() {
        let view = view_with(vec![
            net_row(
                1,
                "Ok",
                false,
                Some(NetProbeHealth::Ok { latency_ms: 182.4 }),
            ),
            net_row(2, "Checking", false, Some(NetProbeHealth::Checking)),
            net_row(3, "Errored", false, Some(NetProbeHealth::Error)),
            net_row(4, "Unprobed", false, None),
        ]);
        let rows = network_rows(&view);
        assert_eq!(rows[0].latency_ms, Some(182), "a measured probe rounds");
        assert_eq!(rows[1].latency_ms, None, "checking is not zero");
        assert_eq!(rows[2].latency_ms, None, "an error is not zero");
        assert_eq!(rows[3].latency_ms, None);
    }

    fn view(code: &str, rate: Option<f64>) -> CurrencyView {
        CurrencyView {
            code: code.to_owned(),
            rate,
            committed: true,
        }
    }

    /// A fresh wallet commits `{USD, 1}`, and the row must then read exactly
    /// what the DST3 mock draws — otherwise going live would silently redraw a
    /// reviewed screen.
    #[test]
    fn a_priced_currency_matches_the_mock() {
        assert_eq!(
            currency_row_value(&view("USD", Some(1.0)), "en"),
            SharedString::from("USD · $1,234.56")
        );
    }

    /// The lie this function exists not to tell. An unpriced JPY must not be
    /// rendered as ¥1,234.56 — that figure is USD, and dressing it in a yen
    /// symbol asserts a rate nobody obtained.
    #[test]
    fn an_unpriced_currency_shows_no_figure_at_all() {
        let value = currency_row_value(&view("JPY", None), "en");
        assert_eq!(value, SharedString::from("JPY"));
        assert!(
            !value.contains("1,234.56"),
            "an unpriced currency must not print a figure it cannot vouch for"
        );
    }

    /// A code with no glyph falls back to itself, and CLDR then separates it
    /// from the digits with a NON-BREAKING space (U+00A0) rather than a plain
    /// one — a currency symbol must not be left at the end of a wrapped line.
    /// Pinned as the literal character it is, because a plain space here would
    /// be a silently different string that only shows up as a bad line break.
    #[test]
    fn an_unknown_code_falls_back_to_itself_with_a_nonbreaking_space() {
        assert_eq!(
            currency_row_value(&view("CHF", Some(1.0)), "en"),
            SharedString::from("CHF · CHF\u{a0}1,234.56")
        );
    }
}

/// A neutral tint for a chain the mocks never drew.
///
/// `#8A8F98` — the design system's muted grey. The alternative is generating a
/// colour from the chain id, which produces a brand-looking colour nobody chose
/// for a network nobody designed.
const UNTINTED: u32 = 0x8A_8F_98;

/// The 网络 rows, from what the core loaded.
///
/// One service endpoint's badge: how it answered, in the words the mocks use.
///
/// `Checking` gets NO pill rather than a neutral one. A grey badge beside a
/// field reads as a verdict, and "we have not asked yet" is not one — the same
/// rule the balance hero applies to a figure it does not have.
#[must_use]
pub fn endpoint_badge(health: &NetServiceHealth, s: &SettingsStrings) -> Option<Pill> {
    match health {
        NetServiceHealth::Checking => None,
        NetServiceHealth::Ok { latency_ms, .. } => {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a measured latency"
            )]
            let ms = latency_ms.max(0.0) as u32;
            Some(crate::settings::fixtures::latency(
                ms,
                (ms >= 1000).then_some(s.network_slow.as_ref()),
            ))
        }
        // Not HTTPS is a REFUSAL to trust, not a slow answer, and it must not
        // wear the same colour as a working endpoint.
        NetServiceHealth::NotHttps => Some(crate::settings::fixtures::pill(
            Tone::Error,
            s.health_https_required.clone(),
        )),
        NetServiceHealth::Unreachable { http_status, .. } => {
            Some(crate::settings::fixtures::pill(
                Tone::Error,
                match http_status {
                    // "HTTP 502" and "Connection failed" are different problems
                    // and lead to different fixes.
                    Some(status) => SharedString::from(format!("HTTP {status}")),
                    None => s.health_offline.clone(),
                },
            ))
        }
        // Reachable, but not the service it must be. A WARNING, not an error:
        // the core does not gate saves on it, so the badge must not look like a
        // refusal.
        NetServiceHealth::InvalidResponse { .. } => Some(crate::settings::fixtures::pill(
            Tone::Warn,
            s.health_invalid.clone(),
        )),
    }
}

/// The tone the field's own border takes.
#[must_use]
pub fn endpoint_tone(health: &NetServiceHealth) -> Option<Tone> {
    match health {
        NetServiceHealth::NotHttps | NetServiceHealth::Unreachable { .. } => Some(Tone::Error),
        NetServiceHealth::Ok { .. } => Some(Tone::Ok),
        NetServiceHealth::Checking | NetServiceHealth::InvalidResponse { .. } => None,
    }
}

/// The core owns which networks exist, in what order, and whether each is
/// custom. This adds only the two things it has no business knowing — a
/// lettermark and a tint — and flattens the probe health to "is there a number".
#[must_use]
pub fn network_rows(view: &NetView) -> Vec<NetworkRowModel> {
    view.networks
        .iter()
        .map(|row| NetworkRowModel {
            id: SharedString::from(row.id.clone()),
            name: SharedString::from(row.display_name.clone()),
            letter: lettermark(&row.display_name),
            color: chain_tint(u64::from(row.chain_id)).unwrap_or(UNTINTED),
            chain_id: u64::from(row.chain_id),
            // A badge is drawn only for a probe that came back with a figure.
            // `Checking` and `Error` are states the core distinguishes and the
            // row draws without a number, exactly as the mock does.
            latency_ms: match row.rpc_health {
                Some(NetProbeHealth::Ok { latency_ms }) => {
                    Some(latency_ms.round().clamp(0.0, f64::from(u32::MAX)) as u32)
                }
                _ => None,
            },
            custom: row.is_custom,
        })
        .collect()
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    fn strings() -> SettingsStrings {
        SettingsStrings::resolve(&crate::loc::Loc::from_env())
    }

    /// Each health state gets its own badge — and "checking" gets none.
    #[test]
    fn a_probe_that_has_not_answered_wears_no_verdict() {
        let s = strings();

        // Not asked yet. A grey badge beside a field reads as a verdict, and
        // this is not one — the same rule the hero applies to a figure it does
        // not have.
        assert!(endpoint_badge(&NetServiceHealth::Checking, &s).is_none());
        assert!(endpoint_tone(&NetServiceHealth::Checking).is_none());

        let ok = endpoint_badge(
            &NetServiceHealth::Ok {
                latency_ms: 62.0,
                rate_count: None,
            },
            &s,
        )
        .unwrap_or_else(|| unreachable!("ok has a badge"));
        assert_eq!(ok.label, "62ms");
        assert!(matches!(ok.tone, Tone::Ok));

        // Over a second the pill says WHY it is amber.
        let slow = endpoint_badge(
            &NetServiceHealth::Ok {
                latency_ms: 1_200.0,
                rate_count: None,
            },
            &s,
        )
        .unwrap_or_else(|| unreachable!("ok has a badge"));
        assert!(matches!(slow.tone, Tone::Warn));
        assert!(slow.label.contains("1.2s"));

        // Not HTTPS is a refusal to trust, not a slow answer.
        let insecure = endpoint_badge(&NetServiceHealth::NotHttps, &s)
            .unwrap_or_else(|| unreachable!("not-https has a badge"));
        assert!(matches!(insecure.tone, Tone::Error));
        assert_eq!(insecure.label, s.health_https_required);

        // "HTTP 502" and "offline" are different problems with different fixes.
        let refused = endpoint_badge(
            &NetServiceHealth::Unreachable {
                http_status: Some(502),
                latency_ms: None,
            },
            &s,
        )
        .unwrap_or_else(|| unreachable!("unreachable has a badge"));
        assert_eq!(refused.label, "HTTP 502");
        let offline = endpoint_badge(
            &NetServiceHealth::Unreachable {
                http_status: None,
                latency_ms: None,
            },
            &s,
        )
        .unwrap_or_else(|| unreachable!("unreachable has a badge"));
        assert_eq!(offline.label, s.health_offline);

        // Reachable but not the right service: a WARNING, because the core does
        // not gate saves on it and the badge must not look like a refusal.
        let wrong = endpoint_badge(&NetServiceHealth::InvalidResponse { latency_ms: 40.0 }, &s)
            .unwrap_or_else(|| unreachable!("invalid has a badge"));
        assert!(matches!(wrong.tone, Tone::Warn));
        assert!(endpoint_tone(&NetServiceHealth::InvalidResponse { latency_ms: 40.0 }).is_none());
    }
}
