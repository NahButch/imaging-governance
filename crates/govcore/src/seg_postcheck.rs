//! Space #2 logic — deterministic segmentation post-checks.
//!
//! Fully implemented in sequencing step 5. This stub keeps the PyO3 surface
//! stable until then.

use crate::report::{Check, CheckStatus, Report};

/// Apply deterministic plausibility gates to a segmentation mask.
pub fn postcheck_segmentation(
    label: &str,
    _mask: &[u8],
    shape: &[usize],
    _spacing_mm: &[f64],
) -> Report {
    let check = Check::new(
        "scaffold.not_implemented",
        "Scaffold placeholder",
        CheckStatus::Warn,
        "seg_postcheck is wired but not yet implemented (sequencing step 5).",
        serde_json::json!({ "label": label, "shape": shape }),
    );
    Report::from_checks("seg_postcheck", vec![check])
}
