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
//!
//! The rule every mayorana app shares is `desktop_kit::licence`, re-exported here; what is
//! PolishCast's own is the product id and the free version's exports.

pub use desktop_kit::licence::{
    check_due, free_reply, normalise, plausible_email, reconcile, shared_from, Answer, Credits, FreeError, Plan, Saved,
    Shared, Status, CHECK_EVERY, GRACE,
};
use serde::{Deserialize, Serialize};

/// The product id api0 knows PolishCast by. The app was called Small Video when plans were
/// first set up, so the id stays.
pub const PRODUCT: &str = "small-video";

/// How many different takes the free version exports.
pub const FREE_EXPORTS: usize = 10;

/// What `saved` amounts to for PolishCast at `now` (seconds since 1970).
pub fn status_at(saved: Option<&Saved>, now: u64) -> Status {
    desktop_kit::licence::status_at(saved, now, PRODUCT)
}

/// What PolishCast shows of a licence status.
pub trait StatusExt {
    /// What the window title adds after "PolishCast <version>".
    fn title_badge(&self) -> &'static str;
}

impl StatusExt for Status {
    fn title_badge(&self) -> &'static str {
        self.title_badge_for(PRODUCT)
    }
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
