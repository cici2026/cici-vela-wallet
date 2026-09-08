//! The signing sheet, built from what four machines decided.
//!
//! The **sibling** of `fixtures.rs`, never its replacement: both produce a
//! `SigningModel`, and the panel picks which one feeds it. That is what keeps
//! the 33 drawn scenarios reviewable after real requests arrive, and what
//! makes "the gallery is unchanged" something a diff can prove.
//!
//! Nothing here decides. The intent sentence, the risk grade, which fields
//! exist and whether a confirm may fire are `clear_signing`'s,
//! `approval_guard`'s, `fee_policy`'s and `sign_request`'s answers; this maps
//! them onto blocks and picks the words the corpus already has.

use gpui::SharedString;

use vela_core::app::approval_guard::GuardView;
use vela_core::app::clear_signing::{
    ClearRisk, ClearSignField, ClearSignResult, ClearSigningView, UNKNOWN_AMOUNT,
};
use vela_core::app::fee_policy::FeeView;
use vela_core::app::sign_request::SignView;

use crate::signing::fixtures::{Block, FeeModel};
use crate::signing::{SigningStrings, Tone};

/// The core's risk grade in the drawn vocabulary.
fn tone_of(risk: ClearRisk) -> Tone {
    match risk {
        ClearRisk::Safe => Tone::Success,
        ClearRisk::Normal => Tone::Neutral,
        ClearRisk::Caution => Tone::Caution,
        ClearRisk::Danger => Tone::Danger,
    }
}

/// May the slide fire?
///
/// **Three machines, ANDed**, and the core's own doc says so: `SignView`'s
/// `confirm_gate_open` is "this machine's own approval gate" and "the shell
/// must AND it with `GuardView.confirm_allowed` and
/// `FeeView.confirm_fee_ready`". Taking any one of them alone arms a slide
/// over an unpriced fee, or over an unlimited approval nobody capped — each
/// of which is a signature the person did not agree to.
#[must_use]
pub fn confirm_enabled(sign: &SignView, guard: &GuardView, fee: &FeeView) -> bool {
    sign.confirm_gate_open && guard.confirm_allowed && fee.confirm_fee_ready
}

/// The blocks a resolved request draws, in the order they are read.
///
/// Intent first — what this DOES — then what is wrong with it, then the
/// detail. A warning under the fields is a warning after the decision.
#[must_use]
pub fn blocks(clear: &ClearSigningView, s: &SigningStrings) -> Vec<Block> {
    let Some(result) = clear.result.as_ref() else {
        return Vec::new();
    };
    let mut out = vec![Block::Intent {
        text: SharedString::from(result.intent.clone()),
        tone: tone_of(result.risk),
    }];
    out.extend(warnings(result, s));
    let rows: Vec<crate::signing::fixtures::Row> = result
        .fields
        .iter()
        // `detail` fields are the Advanced section's, not the summary's.
        // Promoting them here would bury the decision in parameters.
        .filter(|field| !field.detail)
        .map(|field| row_of(field, s))
        .collect();
    if !rows.is_empty() {
        out.push(Block::Rows(rows));
    }
    out
}

/// What is wrong with this request, from the core's flags alone.
///
/// Ordered worst-first, because a sheet is read from the top and the burn is
/// the one that cannot be undone.
fn warnings(result: &ClearSignResult, s: &SigningStrings) -> Vec<Block> {
    let mut out = Vec::new();
    if result.to_own_token {
        // Sending a token to its own contract burns it irreversibly.
        out.push(Block::Warning {
            tone: Tone::Danger,
            text: s.warn_token_to_contract.clone(),
        });
    }
    if result.best_effort {
        // Recovered from the 4-byte database and decoded generically: the
        // shape is a guess that parsed, not a descriptor anybody published.
        out.push(Block::Warning {
            tone: Tone::Caution,
            text: s.warn_best_effort.clone(),
        });
    }
    if result.partial {
        // The descriptor declared more fields than resolved. Saying nothing
        // would present an incomplete reading as a complete one.
        out.push(Block::Warning {
            tone: Tone::Caution,
            text: s.warn_verified_abi.clone(),
        });
    }
    if result.fields.iter().any(|field| field.unverified) {
        // An amount rendered with decimals nobody verified is an amount at a
        // magnitude nobody verified.
        out.push(Block::Warning {
            tone: Tone::Caution,
            text: s.warn_unverified_amount.clone(),
        });
    }
    if result.fields.iter().any(|field| field.expired) {
        out.push(Block::Warning {
            tone: Tone::Caution,
            text: s.warn_expired.clone(),
        });
    }
    out
}

/// One decoded field as a row, keeping the core's flags as the tone.
///
/// The one substitution: an amount the core could not scale. The core has no
/// words — it emits its em dash and sets `unverified` — and a dash under a
/// warning is honest but silent, so the shell says it in the reader's own
/// language. Only for a `tokenAmount`: `unverified` is set by no other field.
fn row_of(field: &ClearSignField, s: &SigningStrings) -> crate::signing::fixtures::Row {
    let tone = if field.warning {
        Tone::Danger
    } else if field.unverified || field.expired {
        Tone::Caution
    } else {
        Tone::Neutral
    };
    let value = if field.unverified && field.value.starts_with(UNKNOWN_AMOUNT) {
        s.amount_unknown.clone()
    } else {
        SharedString::from(field.value.clone())
    };
    (
        SharedString::from(field.label.clone()),
        value,
        tone,
        // Addresses and raw values read as monospace; a decoded amount does
        // not. The core says which is which by carrying an address.
        field.address.is_some(),
    )
}

/// The fee row, or the line that says there is no fee.
///
/// An off-chain signature costs nothing, and saying "network fee: 0" would
/// invite the reader to look for one.
#[must_use]
pub fn fee_model(clear: &ClearSigningView, fee: &FeeView, s: &SigningStrings) -> FeeModel {
    let off_chain = clear.result.as_ref().is_some_and(|result| {
        result.sign_type != vela_core::app::clear_signing::ClearSignType::Transaction
    });
    if off_chain {
        return FeeModel::OffChain(s.ok_no_network_fee.clone());
    }
    // The send screen's formatter, not a second one: two answers about what a
    // transaction costs, on two screens pricing the same operation, is how
    // they start disagreeing. An unpriced fee renders as its "—" rather than
    // vanishing — a row that is absent reads as "free", and the confirm gate
    // is shut for the same reason.
    FeeModel::OnChain {
        label: s.fee_label.clone(),
        value: SharedString::from(crate::flows::live::fee_text(fee.fee.as_ref())),
        selector: None,
    }
}

/// Who is asking, and on which chain.
///
/// From the REQUEST, never from the fixture. A sheet that names the mock's
/// site while a different one is asking for a signature is not a cosmetic
/// error — it is the one fact the person is being asked to judge, wrong.
///
/// The name is the host itself. Deriving a friendly name from a domain is
/// guessing, and a guess in this position is how a look-alike domain gets to
/// present itself as the real thing; the drawings' pretty names come from a
/// dApp identity the request does not carry yet.
#[must_use]
pub fn dapp_identity(origin: &str) -> (SharedString, SharedString, SharedString) {
    let host = origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(origin);
    let letter = host
        .chars()
        .find(char::is_ascii_alphanumeric)
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string());
    (
        SharedString::from(host.to_owned()),
        SharedString::from(host.to_owned()),
        SharedString::from(letter),
    )
}

/// The words on the slide, as the core graded them.
///
/// `Confirm` is never "Approve" — the core's own note says that verb belongs
/// only to an actual token approval, which is `approval_guard`'s surface. The
/// mock said "Confirm swap" over a plain transfer because a fixture cannot
/// know what it is confirming; this does.
#[must_use]
pub fn confirm_label(clear: &ClearSigningView, s: &SigningStrings) -> SharedString {
    use vela_core::app::clear_signing::ClearConfirm;
    let action = match &clear.confirm {
        ClearConfirm::Sign => return s.sign_label.clone(),
        ClearConfirm::Confirm => None,
        // The intent travels as a canonical English key; the shell localizes
        // the ones it has words for and shows the neutral verb for the rest,
        // which is better than showing an English key to somebody reading
        // Chinese.
        ClearConfirm::ConfirmIntent { intent } => match intent.as_str() {
            "send" => Some(s.confirm_send.clone()),
            "swap" => Some(s.confirm_swap.clone()),
            "deposit" => Some(s.confirm_deposit.clone()),
            "withdraw" => Some(s.confirm_withdraw.clone()),
            _ => None,
        },
    };
    match action {
        Some(action) => SharedString::from(format!("{} · {action}", s.slide_to_confirm)),
        None => SharedString::from(format!("{} · {}", s.slide_to_confirm, s.confirm_plain)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use vela_core::app::clear_signing::{ClearFieldRole, ClearSignType};

    fn strings() -> SigningStrings {
        SigningStrings::resolve(&crate::loc::Loc::from_env())
    }

    fn field(label: &str, value: &str) -> ClearSignField {
        ClearSignField {
            label: label.to_owned(),
            value: value.to_owned(),
            format: String::new(),
            token_address: None,
            warning: false,
            unverified: false,
            role: ClearFieldRole::Generic,
            detail: false,
            expired: false,
            address: None,
            usd_value: None,
        }
    }

    /// An amount the core could not scale reads as words, not as a dash and
    /// not as a number.
    ///
    /// The core stopped printing a number it cannot compute (spec 032 phase
    /// 25 — 1 USDC came out as "0"), and the dash it emits instead is correct
    /// but silent. This row is the one a person is asked to judge, so the
    /// shell spends a word on it. A verified amount is untouched: nothing here
    /// may rewrite a number the core did compute.
    #[test]
    fn an_amount_with_unverified_decimals_says_so_in_words() {
        let s = strings();
        let mut unknown = field("Amount", &format!("{UNKNOWN_AMOUNT} USDC.e"));
        unknown.unverified = true;
        let known = field("Amount", "500 USDC.e");

        let blocks = blocks(&view(result(vec![unknown, known])), &s);
        let rows = blocks
            .iter()
            .find_map(|block| match block {
                Block::Rows(rows) => Some(rows.clone()),
                _ => None,
            })
            .unwrap_or_else(|| unreachable!("the fields are drawn as rows"));

        assert_eq!(rows[0].1, s.amount_unknown, "the dash was left to speak");
        assert_ne!(
            rows[0].2,
            Tone::Neutral,
            "an unverified amount is a caution"
        );
        assert_eq!(rows[1].1, SharedString::from("500 USDC.e"));
        assert!(
            !s.amount_unknown.is_empty() && !s.amount_unknown.contains('.'),
            "the corpus key resolved to a phrase, not an echoed key"
        );
    }

    fn result(fields: Vec<ClearSignField>) -> ClearSignResult {
        ClearSignResult {
            intent: "Send 1 ETH".to_owned(),
            contract_name: None,
            owner: None,
            fields,
            risk: ClearRisk::Normal,
            contract_address: None,
            verified: true,
            sign_type: ClearSignType::Transaction,
            partial: false,
            best_effort: false,
            to_own_token: false,
        }
    }

    fn view(result: ClearSignResult) -> ClearSigningView {
        let mut host =
            crate::core_host::CoreHost::<vela_core::app::clear_signing::ClearSigning>::new();
        let _ = host.dispatch(vela_core::app::clear_signing::Event::Cleared);
        ClearSigningView {
            resolved: true,
            result: Some(result),
            ..host.view()
        }
    }

    /// The slide is three machines' answer, ANDed.
    ///
    /// The core's own doc says so, and each one alone is a different way to
    /// arm a signature nobody agreed to: without the fee's, over a price
    /// nobody has; without the guard's, over an unlimited approval nobody
    /// capped.
    #[test]
    fn the_confirm_needs_all_three_machines() {
        let mut sign =
            crate::core_host::CoreHost::<vela_core::app::sign_request::SignRequest>::new().view();
        let mut guard =
            crate::core_host::CoreHost::<vela_core::app::approval_guard::ApprovalGuard>::new()
                .view();
        let mut fee =
            crate::core_host::CoreHost::<vela_core::app::fee_policy::FeePolicy>::new().view();

        sign.confirm_gate_open = true;
        guard.confirm_allowed = true;
        fee.confirm_fee_ready = true;
        assert!(confirm_enabled(&sign, &guard, &fee));

        for drop_one in 0..3 {
            let (mut s, mut g, mut f) = (sign.clone(), guard.clone(), fee.clone());
            match drop_one {
                0 => s.confirm_gate_open = false,
                1 => g.confirm_allowed = false,
                _ => f.confirm_fee_ready = false,
            }
            assert!(
                !confirm_enabled(&s, &g, &f),
                "any one machine withholding shuts the slide ({drop_one})"
            );
        }
    }

    /// The detail fields belong to Advanced, not to the summary.
    ///
    /// Promoting them would bury the decision — what this DOES — under the
    /// parameters it does it with.
    #[test]
    fn advanced_fields_stay_out_of_the_summary() {
        let mut detail = field("calldata", "0xabcd");
        detail.detail = true;
        let blocks = blocks(
            &view(result(vec![field("To", "0xbbb"), detail])),
            &strings(),
        );
        let rows = blocks
            .iter()
            .find_map(|block| match block {
                Block::Rows(rows) => Some(rows),
                _ => None,
            })
            .unwrap_or_else(|| unreachable!("a decoded request has rows"));
        assert_eq!(rows.len(), 1, "only the summary field: {rows:?}");
        assert_eq!(rows[0].0, "To");
    }

    /// Every flag the core raises reaches the screen, worst first.
    #[test]
    fn the_cores_flags_each_become_a_warning() {
        let s = strings();
        let mut burn = result(vec![field("To", "0xbbb")]);
        burn.to_own_token = true;
        burn.best_effort = true;
        let blocks = blocks(&view(burn), &s);
        let warnings: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Warning { tone, text } => Some((*tone, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        // The irreversible one is read first.
        assert_eq!(
            warnings[0],
            (Tone::Danger, s.warn_token_to_contract.clone())
        );
        assert_eq!(warnings[1].0, Tone::Caution);
    }

    /// An unresolved request draws no blocks — never an empty intent that
    /// would read as "this does nothing".
    #[test]
    fn nothing_decoded_draws_nothing() {
        let host = crate::core_host::CoreHost::<vela_core::app::clear_signing::ClearSigning>::new();
        assert!(blocks(&host.view(), &strings()).is_empty());
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    /// The header names the ORIGIN, and does not dress it up.
    ///
    /// Deriving a friendly name from a domain is guessing, and a guess here is
    /// how `uniswap-app.com` gets to present itself as Uniswap. The one fact
    /// the person is being asked to judge is who is asking, so it is shown
    /// exactly as the transport reported it.
    #[test]
    fn the_header_shows_the_origin_verbatim() {
        let (name, host, letter) = dapp_identity("https://app.uniswap.org/swap?x=1");
        assert_eq!(host, "app.uniswap.org");
        assert_eq!(name, host, "no invented display name");
        assert_eq!(letter, "A");

        // A look-alike stays a look-alike on screen.
        let (name, _, _) = dapp_identity("https://uniswap-app.com");
        assert_eq!(name, "uniswap-app.com");

        // Local pages and odd origins do not panic and do not go blank.
        let (name, _, letter) = dapp_identity("http://127.0.0.1:8137/");
        assert_eq!(name, "127.0.0.1:8137");
        assert_eq!(letter, "1");
        assert_eq!(dapp_identity("").2, "?");
    }
}
