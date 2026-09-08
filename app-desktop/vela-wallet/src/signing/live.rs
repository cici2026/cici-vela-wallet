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
    ClearBlindTyped, ClearDangerClass, ClearMessageView, ClearRisk, ClearSignField,
    ClearSignResult, ClearSigningView, ClearSiweBinding, ClearSurface, UNKNOWN_AMOUNT,
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

/// What the shell knows about the request that the core does not hand back.
///
/// Only reached on the blind rung, where by definition nothing was decoded:
/// what is still TRUE then is who it goes to and how many bytes nobody could
/// read. Parsed once, by the host, from the same params the machines were told
/// about — re-parsing it here would be a second reading of an untrusted
/// payload, and two readings can disagree.
#[derive(Clone, Default)]
pub struct RequestFacts {
    pub to: Option<String>,
    pub data_bytes: usize,
}

/// The sheet's body, dispatched by the core's own `ClearSurface`.
///
/// **This is the core's dispatch, not the shell's.** Reading `result.is_some()`
/// instead — which this file did until spec 032 phase 26 — collapses five
/// distinct surfaces into "decoded / not decoded", and the caller then had
/// nothing to draw for four of them. The panel filled that hole with the
/// GALLERY's blocks, so a real request from `127.0.0.1` was drawn, under its
/// own true header, as "Swap 0.5 ETH for 1,278.11 USDC · Uniswap V3 Router"
/// for as long as resolution took. Phase 22 fixed the mirror image of this (a
/// mock header over a live request); this half is worse, because a header the
/// reader can verify invites them to trust the body under it.
///
/// `Loading` draws a line rather than nothing for the same reason invariant ⑦
/// exists: a blind view must never flash before the clear one, and an empty
/// body reads as "this transaction does nothing".
#[must_use]
pub fn blocks(clear: &ClearSigningView, facts: &RequestFacts, s: &SigningStrings) -> Vec<Block> {
    match clear.surface {
        ClearSurface::None => Vec::new(),
        ClearSurface::Loading => vec![Block::Sentence {
            text: s.loading.clone(),
            tone: Tone::Neutral,
        }],
        ClearSurface::ClearSign => clear
            .result
            .as_ref()
            .map(|result| result_blocks(result, s))
            .unwrap_or_default(),
        ClearSurface::EthSign | ClearSurface::MessageSign => clear
            .message
            .as_ref()
            .map(|message| message_blocks(message, s))
            .unwrap_or_default(),
        ClearSurface::BlindTypedData => clear
            .blind_typed
            .as_ref()
            .map(|typed| blind_typed_blocks(typed, s))
            .unwrap_or_default(),
        ClearSurface::BlindTransaction => blind_tx_blocks(facts, s),
    }
}

/// A message the person is asked to sign, in the core's own classification.
///
/// The payload is shown as the core prepared it: `decoded_text` when the bytes
/// are readable, the short hex preview when they are not. Nothing here decodes
/// anything — a second reading of the payload is a second answer to "what am I
/// signing", and only one of them would be on screen.
fn message_blocks(message: &ClearMessageView, s: &SigningStrings) -> Vec<Block> {
    let signing_in = message.siwe.is_some();
    let danger = matches!(
        message.danger_class,
        ClearDangerClass::EthSign | ClearDangerClass::SiwePhish
    );
    let mut out = vec![Block::Intent {
        text: if signing_in {
            s.intent_sign_in.clone()
        } else {
            s.intent_message.clone()
        },
        tone: if danger { Tone::Danger } else { Tone::Neutral },
    }];

    if message.danger_class == ClearDangerClass::EthSign {
        // `eth_sign` signs an opaque digest: there is no text to read, and the
        // sentence says so before the digest is shown.
        out.push(Block::Sentence {
            text: s.body_eth_sign.clone(),
            tone: Tone::Danger,
        });
    }
    if let Some(text) = message.decoded_text.as_ref().filter(|t| !t.is_empty()) {
        out.push(Block::Sentence {
            text: SharedString::from(text.clone()),
            tone: Tone::Neutral,
        });
    }
    if let Some(preview) = message.binary_preview.as_ref() {
        out.push(Block::Code {
            lines: vec![SharedString::from(preview.clone())],
            note: None,
        });
    }
    if message.non_printable {
        out.push(Block::Warning {
            tone: Tone::Caution,
            text: SharedString::from(s.warn_hex_message.clone()),
        });
    }

    if let Some(siwe) = message.siwe.as_ref() {
        let mut rows = vec![(
            s.label_siwe_site.clone(),
            // The host the check RAN ON, never a prettier one: the core's own
            // field doc says showing a different string is how a lookalike
            // slips past.
            SharedString::from(
                siwe.domain_host
                    .clone()
                    .unwrap_or_else(|| siwe.domain.clone()),
            ),
            Tone::Neutral,
            false,
        )];
        if let Some(statement) = siwe.statement.as_ref() {
            rows.push((
                s.label_siwe_statement.clone(),
                SharedString::from(statement.clone()),
                Tone::Neutral,
                false,
            ));
        }
        if let Some(uri) = siwe.uri.as_ref() {
            rows.push((
                s.label_siwe_origin.clone(),
                SharedString::from(uri.clone()),
                Tone::Neutral,
                false,
            ));
        }
        out.push(Block::Rows(rows));
        match siwe_binding(message) {
            // Only a proven match is asserted. `Unknown` says nothing, which
            // is the fail-safe side: an unparseable authority is not evidence
            // of phishing and must not be sold as evidence of safety.
            Some(true) => out.push(Block::Positive(SharedString::from(s.ok_siwe.clone()))),
            Some(false) => out.push(Block::Warning {
                tone: Tone::Danger,
                text: SharedString::from(s.warn_siwe_mismatch.clone()),
            }),
            None => {}
        }
    }

    if message.danger_class == ClearDangerClass::EthSign {
        out.push(Block::Warning {
            tone: Tone::Danger,
            text: s.warn_eth_sign.clone(),
        });
    }
    out
}

fn siwe_binding(message: &ClearMessageView) -> Option<bool> {
    match message.binding? {
        ClearSiweBinding::Ok => Some(true),
        ClearSiweBinding::Mismatch => Some(false),
        ClearSiweBinding::Unknown => None,
    }
}

/// Typed data nobody published a descriptor for: the payload's own projection.
///
/// The core takes the first five `message` entries in payload order and the
/// domain; this draws them and says, in the corpus's words, that no descriptor
/// explained them.
fn blind_typed_blocks(typed: &ClearBlindTyped, s: &SigningStrings) -> Vec<Block> {
    let mut out = vec![
        Block::Intent {
            text: typed
                .primary_type
                .clone()
                .map_or_else(|| s.intent_typed_data.clone(), SharedString::from),
            tone: Tone::Caution,
        },
        Block::Warning {
            tone: Tone::Caution,
            text: s.warn_blind_typed.clone(),
        },
    ];
    if typed.has_domain {
        out.push(Block::Party {
            label: s.label_typed_domain.clone(),
            name: typed
                .domain_name
                .clone()
                .map_or_else(|| s.tag_unverified.clone(), SharedString::from),
            address: typed.verifying_contract.clone().map(SharedString::from),
            badge: None,
        });
    }
    let rows: Vec<crate::signing::fixtures::Row> = typed
        .fields
        .iter()
        .map(|field| {
            (
                SharedString::from(field.key.clone()),
                SharedString::from(field.value.clone()),
                Tone::Neutral,
                true,
            )
        })
        .collect();
    if !rows.is_empty() {
        out.push(Block::Rows(rows));
    }
    out
}

/// The bottom rung: a transaction nothing could read.
///
/// Two facts and no invention — how many bytes were not decoded, and who they
/// go to. The amount is deliberately absent: scaling a value is what the core
/// does for every other rung, and a number this file composed on its own would
/// be a second authority on "how much" (recorded as a gap in phase 26).
fn blind_tx_blocks(facts: &RequestFacts, s: &SigningStrings) -> Vec<Block> {
    let mut out = vec![
        Block::Intent {
            text: s.intent_contract_call.clone(),
            tone: Tone::Caution,
        },
        Block::Warning {
            tone: Tone::Caution,
            text: SharedString::from(crate::signing::fill(
                &s.warn_blind_decode,
                &[("bytes", &facts.data_bytes.to_string())],
            )),
        },
    ];
    if let Some(to) = facts.to.as_ref() {
        out.push(Block::Party {
            label: s.label_interacting.clone(),
            name: s.tag_unverified.clone(),
            address: Some(SharedString::from(to.clone())),
            badge: Some((s.tag_unverified.clone(), Tone::Caution)),
        });
    }
    out
}

/// The blocks a decoded request draws, in the order they are read.
///
/// Intent first — what this DOES — then what is wrong with it, then the
/// detail. A warning under the fields is a warning after the decision.
fn result_blocks(result: &ClearSignResult, s: &SigningStrings) -> Vec<Block> {
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

        let blocks = blocks(
            &view(result(vec![unknown, known])),
            &RequestFacts::default(),
            &s,
        );
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
            // The surface a decoded request is presented on. The core picks it
            // and the sheet follows it, so a fixture that set only `result`
            // would be testing a state the core never produces.
            surface: ClearSurface::ClearSign,
            result: Some(result),
            ..host.view()
        }
    }

    fn surfaced(surface: ClearSurface, view: ClearSigningView) -> ClearSigningView {
        ClearSigningView { surface, ..view }
    }

    fn pristine() -> ClearSigningView {
        crate::core_host::CoreHost::<vela_core::app::clear_signing::ClearSigning>::new().view()
    }

    fn message(danger: ClearDangerClass) -> ClearMessageView {
        ClearMessageView {
            payload: "0xdead".to_owned(),
            is_hex: true,
            decoded_text: Some("Sign in to Example".to_owned()),
            binary_preview: None,
            non_printable: false,
            siwe: None,
            binding: None,
            danger_class: danger,
        }
    }

    /// Every surface the core can present draws SOMETHING.
    ///
    /// This is the guard on the defect phase 26 fixed. The panel used to keep
    /// the gallery's blocks whenever the live builder returned nothing, so a
    /// real request wore a drawn swap under its own true header. The panel no
    /// longer has that fallback — which means an empty answer here is now a
    /// blank sheet, and a blank sheet reads as "this does nothing". Any
    /// surface that stops drawing must fail here first.
    #[test]
    fn every_surface_the_core_can_present_draws_something() {
        let s = strings();
        let facts = RequestFacts {
            to: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned()),
            data_bytes: 196,
        };
        let cases: Vec<(ClearSurface, ClearSigningView)> = vec![
            (ClearSurface::Loading, pristine()),
            (
                ClearSurface::ClearSign,
                view(result(vec![field("Amount", "1 USDC.e")])),
            ),
            (
                ClearSurface::MessageSign,
                ClearSigningView {
                    message: Some(message(ClearDangerClass::Plain)),
                    ..pristine()
                },
            ),
            (
                ClearSurface::EthSign,
                ClearSigningView {
                    message: Some(message(ClearDangerClass::EthSign)),
                    ..pristine()
                },
            ),
            (
                ClearSurface::BlindTypedData,
                ClearSigningView {
                    blind_typed: Some(ClearBlindTyped {
                        primary_type: Some("Permit".to_owned()),
                        has_domain: true,
                        domain_name: Some("Example".to_owned()),
                        verifying_contract: Some("0xcccc".to_owned()),
                        fields: vec![vela_core::app::clear_signing::ClearBlindField {
                            key: "spender".to_owned(),
                            value: "0xdddd".to_owned(),
                        }],
                    }),
                    ..pristine()
                },
            ),
            (ClearSurface::BlindTransaction, pristine()),
        ];
        for (surface, base) in cases {
            let drawn = blocks(&surfaced(surface, base), &facts, &s);
            assert!(
                !drawn.is_empty(),
                "{surface:?} drew nothing — the sheet would be blank"
            );
        }
        // The one surface that is meant to be empty: the core presenting
        // nothing at all. Drawing something here would be the shell inventing
        // a request.
        assert!(blocks(&surfaced(ClearSurface::None, pristine()), &facts, &s).is_empty());
    }

    /// The blind rung says the two things that are still true, and no more.
    #[test]
    fn a_blind_transaction_says_only_what_is_true_about_it() {
        let s = strings();
        let facts = RequestFacts {
            to: Some("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned()),
            data_bytes: 196,
        };
        let drawn = blocks(
            &surfaced(ClearSurface::BlindTransaction, pristine()),
            &facts,
            &s,
        );
        let warned = drawn.iter().any(|block| match block {
            Block::Warning { text, .. } => text.contains("196"),
            _ => false,
        });
        assert!(warned, "the byte count nobody could read is the warning");
        let named = drawn.iter().any(|block| match block {
            Block::Party { address, .. } => address.as_ref().is_some_and(|a| a.contains("0xbbbb")),
            _ => false,
        });
        assert!(named, "who it goes to is still known");
        // No amount: scaling a value is the core's job on every other rung,
        // and a number composed here would be a second authority on "how much".
        assert!(
            !drawn
                .iter()
                .any(|block| matches!(block, Block::Amount { .. })),
            "the shell invented an amount"
        );
    }

    /// `eth_sign` is the hard-warning surface, never the calm message view —
    /// and an unknown SIWE binding asserts nothing in either direction.
    #[test]
    fn the_message_surfaces_keep_the_cores_classification() {
        let s = strings();
        let facts = RequestFacts::default();

        let hard = blocks(
            &surfaced(
                ClearSurface::EthSign,
                ClearSigningView {
                    message: Some(message(ClearDangerClass::EthSign)),
                    ..pristine()
                },
            ),
            &facts,
            &s,
        );
        assert!(
            hard.iter().any(|block| matches!(
                block,
                Block::Warning {
                    tone: Tone::Danger,
                    ..
                }
            )),
            "eth_sign drew no danger warning"
        );

        let mut unknown_binding = message(ClearDangerClass::SiweOk);
        unknown_binding.siwe = Some(vela_core::app::clear_signing::ClearSiweFields {
            domain: "example.com".to_owned(),
            domain_host: Some("example.com".to_owned()),
            address: None,
            statement: Some("Sign in".to_owned()),
            uri: None,
            chain_id: None,
            nonce: None,
        });
        unknown_binding.binding = Some(ClearSiweBinding::Unknown);
        let calm = blocks(
            &surfaced(
                ClearSurface::MessageSign,
                ClearSigningView {
                    message: Some(unknown_binding),
                    ..pristine()
                },
            ),
            &facts,
            &s,
        );
        assert!(
            !calm.iter().any(|block| matches!(block, Block::Positive(_))),
            "an unparseable authority was sold as a verified match"
        );
        assert!(
            !calm.iter().any(|block| matches!(
                block,
                Block::Warning {
                    tone: Tone::Danger,
                    ..
                }
            )),
            "an unknown binding is not evidence of phishing either"
        );
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
            &RequestFacts::default(),
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
        let blocks = blocks(&view(burn), &RequestFacts::default(), &s);
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

    /// A pristine machine — nothing presented at all — draws nothing.
    ///
    /// Written before phase 26 as "nothing decoded draws nothing", which is no
    /// longer the same sentence: a request that decodes to nothing is the
    /// BlindTransaction surface and it draws the blind rung. What draws
    /// nothing is `ClearSurface::None`, which is the core saying it has not
    /// been given a request to present.
    #[test]
    fn a_machine_with_no_request_draws_nothing() {
        let host = crate::core_host::CoreHost::<vela_core::app::clear_signing::ClearSigning>::new();
        assert!(blocks(&host.view(), &RequestFacts::default(), &strings()).is_empty());
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
