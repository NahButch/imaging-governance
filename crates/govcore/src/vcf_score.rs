//! Space #3 logic — deterministic VCF parsing + weighted-evidence scoring.
//!
//! Fully implemented in sequencing step 6. This stub keeps the PyO3 surface
//! stable until then.

use crate::report::{Check, CheckStatus, Report};

/// Parse and deterministically score the variants in a VCF document.
pub fn score_vcf(vcf_text: &str) -> Report {
    let check = Check::new(
        "scaffold.not_implemented",
        "Scaffold placeholder",
        CheckStatus::Warn,
        "vcf_score is wired but not yet implemented (sequencing step 6).",
        serde_json::json!({ "input_bytes": vcf_text.len() }),
    );
    Report::from_checks("vcf_score", vec![check])
}
