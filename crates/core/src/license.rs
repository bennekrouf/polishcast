//! PolishCast Pro: what a subscription licence unlocks, from api0's last answer about it, and
//! the free version's ten exports.
//!
//! Pro comes with a monthly plan bought on mayorana.ch. The plan's licence number
//! (`ABCDE-FGHJK-LMNPQ-RSTUV`, emailed when it is bought) is checked with api0
//! (`GET https://gateway.api0.ai/v1/licence`) when PolishCast starts and then daily; the
//! answer names the plan, the apps it makes Pro, whether it is paid up and the AI credits
//! left. The app keeps the last answer, so Pro lasts [`GRACE`] after the last check that
//! got through. The network and the file are the app's; this is the rule.
//!
//! Without Pro, [`FREE_EXPORTS`] different takes can be exported. Exporting one of them again
//! (after changing a zoom, say) doesn't count again; recording and editing are never limited.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The product id api0 knows PolishCast by. The app was called Small Video when plans were
/// first set up, so the id stays.
pub const PRODUCT: &str = "small-video";

/// How often the licence is checked while PolishCast is open.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// How long Pro lasts without a check getting through (offline, api0 down).
pub const GRACE: Duration = Duration::from_secs(7 * 24 * 3600);

/// How many different takes the free version exports.
pub const FREE_EXPORTS: usize = 10;

const ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A licence number as typed — any case, spaces or dashes — in the form api0 stores it,
/// or `None` if it can't be one.
pub fn normalise(typed: &str) -> Option<String> {
    let chars: String =
        typed.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect();
    if chars.len() != 20 || !chars.chars().all(|c| ALPHABET.contains(c)) {
        return None;
    }
    Some(chars.as_bytes().chunks(5).map(|c| std::str::from_utf8(c).unwrap_or_default()).collect::<Vec<_>>().join("-"))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Credits {
    #[serde(default)]
    pub granted: i64,
    #[serde(default)]
    pub used: i64,
    #[serde(default)]
    pub left: i64,
}

/// api0's answer about a licence (`GET /v1/licence`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(default)]
    pub licence: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub plan: Plan,
    /// The apps the plan makes Pro, by product id; "*" is every app.
    #[serde(default)]
    pub products: Vec<String>,
    /// Paid up. False once the subscription lapses or is cancelled.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    /// When the current month ends (RFC 3339).
    #[serde(default)]
    pub period_end: String,
    #[serde(default)]
    pub credits: Credits,
    #[serde(default)]
    pub checked_at: String,
}

impl Answer {
    /// The plan is paid up and includes PolishCast.
    pub fn unlocks(&self) -> bool {
        self.active && self.products.iter().any(|p| p == PRODUCT || p == "*")
    }

    /// The day the current month ends, `YYYY-MM-DD`.
    pub fn period_end_day(&self) -> &str {
        self.period_end.get(..10).unwrap_or(&self.period_end)
    }

    /// The free allowance an app hands out: AI credits each month, no app made
    /// Pro.
    pub fn is_free(&self) -> bool {
        self.active && self.products.is_empty()
    }

    /// "Renews on …", or "Ends on …" once cancelled.
    pub fn renewal(&self) -> String {
        let verb = if self.cancel_at_period_end { "Ends" } else { "Renews" };
        format!("{verb} on {}", self.period_end_day())
    }
}

/// What the app keeps: the licence number, and api0's last answer with when it came
/// (seconds since 1970, this computer's clock).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub licence: String,
    #[serde(default)]
    pub answer: Option<Answer>,
    #[serde(default)]
    pub checked: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// No licence on this computer.
    Free,
    /// The plan includes PolishCast and is paid up, as of a check within [`GRACE`].
    Pro(Answer),
    /// Checked: the plan has lapsed, or doesn't include PolishCast.
    NotPro(Answer),
    /// Pro at the last check, but no check has got through for [`GRACE`].
    Offline(Answer),
    /// A licence number api0 doesn't know, or one not checked yet.
    Unknown(String),
}

impl Status {
    /// Whether every take can be exported.
    pub fn unlimited(&self) -> bool {
        matches!(self, Status::Pro(_))
    }

    /// api0's answer, when the licence is the free allowance of AI credits.
    pub fn free_credits(&self) -> Option<&Answer> {
        match self {
            Status::NotPro(a) if a.is_free() => Some(a),
            _ => None,
        }
    }

    /// The saved licence number, if any.
    pub fn licence(&self) -> Option<&str> {
        match self {
            Status::Free => None,
            Status::Pro(a) | Status::NotPro(a) | Status::Offline(a) => Some(&a.licence),
            Status::Unknown(l) => Some(l),
        }
    }
}

/// Why a request for free AI credits got no licence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FreeError {
    /// There are no free credits right now: the form is hidden.
    Unavailable,
    /// Anything else, to show as it is.
    Refused(String),
}

/// Whether `email` can be one, before asking api0.
pub fn plausible_email(email: &str) -> bool {
    let email = email.trim();
    match email.split_once('@') {
        Some((user, domain)) => {
            !user.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !email.contains(char::is_whitespace)
        }
        None => false,
    }
}

/// api0's reply to `POST /api/plans/free`, read: the message to show once the
/// licence number is on its way.
pub fn free_reply(status: u16, body: &str) -> Result<String, FreeError> {
    let message =
        serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v["message"].as_str().map(str::to_owned));
    match status {
        200..=299 => Ok(message.unwrap_or_else(|| "Your licence number is on its way to that email.".into())),
        404 => Err(FreeError::Unavailable),
        409 => Err(FreeError::Refused("This email already has a plan: paste its licence number instead.".into())),
        429 => Err(FreeError::Refused("Too many requests: try again in a while.".into())),
        _ => Err(FreeError::Refused(
            message.unwrap_or_else(|| format!("The free AI credits couldn't be requested (api0 answered {status}).")),
        )),
    }
}

/// What `saved` amounts to at `now` (seconds since 1970).
pub fn status_at(saved: Option<&Saved>, now: u64) -> Status {
    let Some(saved) = saved else { return Status::Free };
    let Some(answer) = saved.answer.clone() else { return Status::Unknown(saved.licence.clone()) };
    if !answer.unlocks() {
        Status::NotPro(answer)
    } else if now.saturating_sub(saved.checked) > GRACE.as_secs() {
        Status::Offline(answer)
    } else {
        Status::Pro(answer)
    }
}

/// Whether the last check is old enough for another, at `now`.
pub fn check_due(saved: &Saved, now: u64) -> bool {
    saved.answer.is_none() || now.saturating_sub(saved.checked) >= CHECK_EVERY.as_secs()
}

/// The takes exported so far without Pro, by name (a take's folder name), in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exported {
    #[serde(default)]
    pub takes: Vec<String>,
}

impl Exported {
    /// Whether `take` may be exported without Pro: it already was, or a free export is left.
    pub fn allows(&self, take: &str) -> bool {
        self.takes.iter().any(|t| t == take) || self.takes.len() < FREE_EXPORTS
    }

    /// Free exports not used yet.
    pub fn left(&self) -> usize {
        FREE_EXPORTS.saturating_sub(self.takes.len())
    }

    /// Counts a finished export of `take`; again for the same take changes nothing. Returns
    /// whether anything changed.
    pub fn record(&mut self, take: &str) -> bool {
        if self.takes.iter().any(|t| t == take) {
            return false;
        }
        self.takes.push(take.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 3600;

    fn answer(products: &[&str], active: bool) -> Answer {
        Answer {
            licence: "ABCDE-FGHJK-LMNPQ-RSTUV".into(),
            email: "anna@studio.ch".into(),
            plan: Plan { id: "pro".into(), name: "Pro".into() },
            products: products.iter().map(|p| p.to_string()).collect(),
            active,
            period_end: "2026-11-21T10:00:00Z".into(),
            ..Answer::default()
        }
    }

    fn saved(answer: Answer, checked: u64) -> Saved {
        Saved { licence: answer.licence.clone(), answer: Some(answer), checked }
    }

    #[test]
    fn licence_numbers_are_read_in_any_case_with_spaces_or_dashes() {
        let want = Some("ABCDE-FGHJK-LMNPQ-RSTUV".to_string());
        assert_eq!(normalise("ABCDE-FGHJK-LMNPQ-RSTUV"), want);
        assert_eq!(normalise(" abcde fghjk\nlmnpq-rstuv "), want);
        assert_eq!(normalise("ABCDEFGHJKLMNPQRSTUV"), want);
        assert_eq!(normalise("ABCDE-FGHJK-LMNPQ"), None);
        // 0, 1, I and O look alike, so licence numbers never use them.
        assert_eq!(normalise("ABCDE-FGHJK-LMNPQ-RSTU0"), None);
        assert_eq!(normalise("ABCDE-FGHJK-LMNPQ-RSTUI"), None);
    }

    #[test]
    fn pro_needs_an_active_plan_that_lists_polishcast_or_every_app() {
        assert!(answer(&["small-video"], true).unlocks());
        assert!(answer(&["gitagent", "*"], true).unlocks());
        assert!(!answer(&["gitagent", "jobscout"], true).unlocks());
        assert!(!answer(&["small-video"], false).unlocks());

        let now = 1_000 * DAY;
        assert!(status_at(Some(&saved(answer(&["*"], true), now)), now).unlimited());
        assert!(matches!(status_at(Some(&saved(answer(&["small-video"], false), now)), now), Status::NotPro(_)));
        assert!(matches!(status_at(Some(&saved(answer(&["jobscout"], true), now)), now), Status::NotPro(_)));
        assert_eq!(status_at(None, now), Status::Free);
        let unchecked = Saved { licence: "ABCDE-FGHJK-LMNPQ-RSTUV".into(), ..Saved::default() };
        assert!(!status_at(Some(&unchecked), now).unlimited());
    }

    #[test]
    fn a_free_licence_gives_ai_credits_but_not_pro() {
        let now = 1_000 * DAY;
        let mut free = answer(&[], true);
        free.credits.left = 50;
        let status = status_at(Some(&saved(free, now)), now);
        assert!(!status.unlimited());
        assert!(matches!(status, Status::NotPro(_)));
        assert_eq!(status.free_credits().map(|a| a.credits.left), Some(50));
        // A paid plan, or a lapsed free one, isn't the free allowance.
        let paid = status_at(Some(&saved(answer(&["small-video"], true), now)), now);
        assert!(paid.free_credits().is_none());
        assert!(status_at(Some(&saved(answer(&[], false), now)), now).free_credits().is_none());
    }

    #[test]
    fn api0s_reply_to_a_free_credits_request_is_read() {
        assert_eq!(
            free_reply(200, r#"{"success":true,"emailed":true,"message":"On its way."}"#),
            Ok("On its way.".into())
        );
        assert_eq!(
            free_reply(404, r#"{"message":"There are no free credits right now"}"#),
            Err(FreeError::Unavailable)
        );
        assert!(matches!(free_reply(409, "{}"), Err(FreeError::Refused(m)) if m.contains("already has a plan")));
        assert_eq!(
            free_reply(400, r#"{"message":"Enter the email"}"#),
            Err(FreeError::Refused("Enter the email".into()))
        );
        assert!(plausible_email(" dev@example.ch "));
        assert!(!plausible_email("dev@example"));
        assert!(!plausible_email("example.ch"));
    }

    #[test]
    fn pro_lasts_seven_days_after_the_last_check_that_got_through() {
        let checked = 1_000 * DAY;
        let s = saved(answer(&["small-video"], true), checked);
        assert!(status_at(Some(&s), checked + 6 * DAY).unlimited());
        assert!(status_at(Some(&s), checked + 7 * DAY).unlimited());
        let late = status_at(Some(&s), checked + 7 * DAY + 1);
        assert!(matches!(late, Status::Offline(_)));
        assert!(!late.unlimited());
        // A clock set back doesn't take Pro away.
        assert!(status_at(Some(&s), checked - DAY).unlimited());
    }

    #[test]
    fn the_licence_is_checked_again_after_a_day() {
        let checked = 1_000 * DAY;
        let s = saved(answer(&["small-video"], true), checked);
        assert!(!check_due(&s, checked + DAY - 1));
        assert!(check_due(&s, checked + DAY));
        assert!(check_due(&Saved { licence: s.licence.clone(), ..Saved::default() }, checked));
    }

    #[test]
    fn api0s_answer_is_read() {
        let json = r#"{"object":"licence","licence":"ABCDE-FGHJK-LMNPQ-RSTUV","email":"anna@studio.ch",
            "plan":{"id":"studio","name":"Studio"},"products":["small-video"],"active":true,
            "status":"active","cancel_at_period_end":false,"period_end":"2026-11-21T10:00:00Z",
            "credits":{"granted":500,"used":120,"left":380},"checked_at":"2026-10-21T10:00:00Z"}"#;
        let a: Answer = serde_json::from_str(json).unwrap();
        assert!(a.unlocks());
        assert_eq!(a.plan.name, "Studio");
        assert_eq!(a.credits.left, 380);
        assert_eq!(a.renewal(), "Renews on 2026-11-21");
    }

    #[test]
    fn ten_different_takes_export_free_and_exporting_one_again_costs_nothing() {
        let mut exported = Exported::default();
        for i in 0..FREE_EXPORTS {
            let take = format!("2026-10-0{} 10.00", i);
            assert!(exported.allows(&take));
            assert!(exported.record(&take));
        }
        assert_eq!(exported.left(), 0);
        assert!(exported.allows("2026-10-03 10.00"), "a take already exported can be exported again");
        assert!(!exported.record("2026-10-03 10.00"));
        assert!(!exported.allows("2026-10-09 18.30"), "an eleventh take needs Pro");
    }
}
