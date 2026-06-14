//! Space #1 logic — deterministic DICOM series QC.
//!
//! Fully implemented in sequencing step 2. This step-1 stub exists only to wire
//! and prove the PyO3 boundary.

use crate::report::{Check, CheckStatus, Report};

/// Validate a DICOM series given the paths to its instances.
pub fn qc_series(paths: &[String]) -> Report {
    let check = Check::new(
        "scaffold.not_implemented",
        "Scaffold placeholder",
        CheckStatus::Warn,
        "dicom_qc is wired but not yet implemented (sequencing step 2).",
        serde_json::json!({ "received_paths": paths.len() }),
    );
    Report::from_checks("dicom_qc", vec![check])
}
