//! The one line of status in the header (port of `src/ui/status-line.js`).
//!
//! Busy, error and warning states say what is going on until something
//! replaces them. A plain `ok` message ("SteadyCom calculation cancelled",
//! "Map loaded", the permitted µ range, why a layout preset kept the drawing)
//! is shown for [`NOTICE_MS`] and then gives way to the standing summary.
//! Replaced on the very paint that should show it, the explanation would
//! never reach the screen.

use coxeter_core::js;

use crate::store::Store;
use crate::types::{Status, StatusState};

/// How long an `ok` message stays before the summary returns.
pub const NOTICE_MS: f64 = 6000.0;

/// What the header shows: `state` is `busy`, `error`, `warn`, `notice` or
/// `summary`; `expires_in > 0` when the line has to be repainted after that
/// many milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusLine {
    pub text: String,
    pub title: String,
    pub state: &'static str,
    pub expires_in: f64,
}

/// `statusLine(store, noticeAge)`: `notice_age` is the milliseconds since
/// this status was first painted.
pub fn status_line(store: &Store, notice_age: f64) -> StatusLine {
    status_line_of(store.status(), notice_age, || status_summary(store))
}

/// [`status_line`] for a bare status and a summary.
pub fn status_line_of(s: &Status, notice_age: f64, summary: impl FnOnce() -> String) -> StatusLine {
    let line = |text: &str, title: &str, state, expires_in| StatusLine { text: text.into(), title: title.into(), state, expires_in };
    match s.state {
        StatusState::Busy => line(if s.message.is_empty() { "Working…" } else { &s.message }, "", "busy", 0.0),
        StatusState::Error => line(&s.message, &s.message, "error", 0.0),
        StatusState::Warn if !s.message.is_empty() => line(&s.message, &s.message, "warn", 0.0),
        StatusState::Ok if !s.message.is_empty() && notice_age < NOTICE_MS => {
            line(&s.message, &s.message, "notice", NOTICE_MS - notice_age)
        }
        _ => line(&summary(), "", "summary", 0.0),
    }
}

/// `statusSummary(store)`: species, reactions, objective and LP time.
pub fn status_summary(store: &Store) -> String {
    let mu = store.solution.as_ref().map(|s| s.objective_value).filter(|m| m.is_finite());
    let n = store.community.as_ref().map_or(0, |c| c.merged.reactions.len());
    let species = store.community.as_ref().map_or(0, |c| c.active_species().count());
    format!(
        "{species} species · {n} reactions · objective {} · LP {} ms",
        mu.map_or_else(|| "—".to_string(), |m| js::to_fixed(m, 5)),
        js::number_to_string(store.status().solve_ms)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ok_message_is_shown_for_a_while_before_the_summary_returns() {
        let s = Status { state: StatusState::Ok, message: "The drawing was kept.".into(), solve_ms: 3.0, layout_ms: 0.0 };
        let summary = || "0 species · 0 reactions · objective — · LP 3 ms".to_string();
        let fresh = status_line_of(&s, 0.0, summary);
        assert_eq!(fresh.text, "The drawing was kept.");
        assert_eq!(fresh.expires_in, NOTICE_MS);
        let stale = status_line_of(&s, NOTICE_MS + 1.0, summary);
        assert!(stale.text.contains("species · 0 reactions · objective — · LP 3 ms"));
        assert_eq!(stale.expires_in, 0.0);
        let empty = Status { state: StatusState::Ok, ..Status::default() };
        assert_eq!(status_line_of(&empty, 0.0, summary).state, "summary");
        let warn = Status { state: StatusState::Warn, message: "w".into(), ..Status::default() };
        assert_eq!(status_line_of(&warn, 1e9, summary).text, "w");
        let error = Status { state: StatusState::Error, message: "e".into(), ..Status::default() };
        assert_eq!(status_line_of(&error, 1e9, summary).text, "e");
        let busy = Status { state: StatusState::Busy, ..Status::default() };
        assert_eq!(status_line_of(&busy, 1e9, summary).text, "Working…");
    }
}
