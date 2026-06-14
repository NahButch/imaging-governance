//! Shared structured-report types used by every governance module.
//!
//! The whole point of `govcore` is that a *deterministic* engine produces an
//! auditable verdict: every module emits the same `Report` shape so the three
//! Spaces render identically and the output is machine-checkable.

use serde::Serialize;

/// Outcome of a single deterministic check. Ordered worst-last so that
/// `max()` over a set of statuses yields the overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckStatus {
    /// Combine two statuses, keeping the more severe one.
    pub fn worse(self, other: CheckStatus) -> CheckStatus {
        self.max(other)
    }
}

/// One deterministic check with human-readable detail and structured evidence.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// Stable machine identifier (e.g. `"geometry.slice_spacing"`).
    pub id: String,
    /// Short human-readable name.
    pub name: String,
    pub status: CheckStatus,
    /// One-line explanation of the outcome.
    pub detail: String,
    /// Arbitrary structured evidence backing the verdict (auditable, not a
    /// black box). Always a JSON object/array, never opaque text.
    pub evidence: serde_json::Value,
}

impl Check {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        status: CheckStatus,
        detail: impl Into<String>,
        evidence: serde_json::Value,
    ) -> Self {
        Check {
            id: id.into(),
            name: name.into(),
            status,
            detail: detail.into(),
            evidence,
        }
    }
}

/// A complete deterministic report. Checks are emitted in a fixed order so the
/// output is byte-stable for identical input.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Module that produced the report (e.g. `"dicom_qc"`).
    pub module: String,
    /// Overall verdict = the worst status across all checks.
    pub verdict: CheckStatus,
    /// One-line summary suitable for a header.
    pub summary: String,
    /// Counts by status, for quick rendering.
    pub counts: Counts,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
}

impl Report {
    /// Build a report from an ordered set of checks, deriving the overall
    /// verdict, counts, and a summary line deterministically.
    pub fn from_checks(module: impl Into<String>, checks: Vec<Check>) -> Self {
        let mut counts = Counts::default();
        let mut verdict = CheckStatus::Pass;
        for c in &checks {
            match c.status {
                CheckStatus::Pass => counts.pass += 1,
                CheckStatus::Warn => counts.warn += 1,
                CheckStatus::Fail => counts.fail += 1,
            }
            verdict = verdict.worse(c.status);
        }
        let summary = match verdict {
            CheckStatus::Pass => format!("PASS — {} checks, all clear", checks.len()),
            CheckStatus::Warn => format!(
                "WARN — {} warning(s), {} pass",
                counts.warn, counts.pass
            ),
            CheckStatus::Fail => format!(
                "FAIL — {} failure(s), {} warning(s), {} pass",
                counts.fail, counts.warn, counts.pass
            ),
        };
        Report {
            module: module.into(),
            verdict,
            summary,
            counts,
            checks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chk(status: CheckStatus) -> Check {
        Check::new("id", "name", status, "detail", json!({}))
    }

    #[test]
    fn status_orders_pass_warn_fail() {
        assert!(CheckStatus::Pass < CheckStatus::Warn);
        assert!(CheckStatus::Warn < CheckStatus::Fail);
        assert_eq!(CheckStatus::Pass.worse(CheckStatus::Fail), CheckStatus::Fail);
        assert_eq!(CheckStatus::Warn.worse(CheckStatus::Pass), CheckStatus::Warn);
        assert_eq!(CheckStatus::Fail.worse(CheckStatus::Warn), CheckStatus::Fail);
    }

    #[test]
    fn verdict_is_worst_with_correct_counts() {
        let r = Report::from_checks(
            "m",
            vec![chk(CheckStatus::Pass), chk(CheckStatus::Warn), chk(CheckStatus::Fail), chk(CheckStatus::Pass)],
        );
        assert_eq!(r.verdict, CheckStatus::Fail);
        assert_eq!(r.counts.pass, 2);
        assert_eq!(r.counts.warn, 1);
        assert_eq!(r.counts.fail, 1);
        assert_eq!(r.module, "m");
    }

    #[test]
    fn all_pass_yields_pass_verdict() {
        let r = Report::from_checks("m", vec![chk(CheckStatus::Pass), chk(CheckStatus::Pass)]);
        assert_eq!(r.verdict, CheckStatus::Pass);
        assert!(r.summary.starts_with("PASS"));
    }

    #[test]
    fn warn_without_fail_yields_warn_verdict() {
        let r = Report::from_checks("m", vec![chk(CheckStatus::Pass), chk(CheckStatus::Warn)]);
        assert_eq!(r.verdict, CheckStatus::Warn);
    }

    #[test]
    fn status_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&CheckStatus::Pass).unwrap(), "\"pass\"");
        assert_eq!(serde_json::to_string(&CheckStatus::Fail).unwrap(), "\"fail\"");
    }
}
