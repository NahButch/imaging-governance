//! Space #1 logic — deterministic DICOM series QC.
//!
//! Given the filesystem paths of a series' instances, this module reads each
//! one with `dicom-rs` and runs an **ordered, deterministic** battery of
//! checks: ingest readability, header completeness, modality/SOP sanity, series
//! uniformity, slice geometry (orientation, monotonic position, uniform
//! spacing), instance-number completeness, and a PHI-leak scan. Every check
//! emits a `{status, detail, evidence}` record; the overall verdict is the
//! worst status. No probabilistic logic — same bytes in, same report out.

use crate::report::{Check, CheckStatus, Report};
use dicom::core::Tag;
use dicom::dictionary_std::tags;
use dicom::object::{open_file, DefaultDicomObject};
use serde_json::json;

/// Direction-cosine equality tolerance for "same orientation".
const ORIENTATION_TOL: f64 = 1e-3;
/// Absolute slice-spacing tolerance (mm) before flagging non-uniform spacing.
const SPACING_ABS_TOL_MM: f64 = 0.05;
/// Relative slice-spacing tolerance (fraction of mean spacing).
const SPACING_REL_TOL: f64 = 0.01;

/// Strong direct identifiers — presence (non-anonymised) fails the PHI gate.
const PHI_DIRECT: &[(Tag, &str)] = &[
    (tags::PATIENT_NAME, "PatientName"),
    (tags::PATIENT_ID, "PatientID"),
    (tags::PATIENT_BIRTH_DATE, "PatientBirthDate"),
    (tags::PATIENT_ADDRESS, "PatientAddress"),
    (tags::PATIENT_TELEPHONE_NUMBERS, "PatientTelephoneNumbers"),
];

/// Indirect / quasi identifiers — presence warns but does not fail.
const PHI_INDIRECT: &[(Tag, &str)] = &[
    (tags::REFERRING_PHYSICIAN_NAME, "ReferringPhysicianName"),
    (tags::INSTITUTION_NAME, "InstitutionName"),
    (tags::INSTITUTION_ADDRESS, "InstitutionAddress"),
    (tags::STUDY_DATE, "StudyDate"),
    (tags::ACCESSION_NUMBER, "AccessionNumber"),
];

/// Placeholder values that count as properly de-identified, not as PHI.
const ANON_PLACEHOLDERS: &[&str] = &["", "ANONYMOUS", "ANON", "NONE", "REMOVED", "DEIDENTIFIED"];

/// Known DICOM modality codes used for the sanity check.
const KNOWN_MODALITIES: &[&str] = &[
    "CT", "MR", "PT", "NM", "US", "XA", "CR", "DX", "MG", "RF", "OT", "SC", "RTSTRUCT", "RTDOSE",
    "RTPLAN", "RTIMAGE", "SEG",
];

/// Per-instance extracted view of a DICOM file (or a parse error).
struct Instance {
    path: String,
    parse_error: Option<String>,
    sop_class_uid: Option<String>,
    sop_instance_uid: Option<String>,
    series_uid: Option<String>,
    study_uid: Option<String>,
    modality: Option<String>,
    instance_number: Option<i64>,
    rows: Option<i64>,
    columns: Option<i64>,
    bits_allocated: Option<i64>,
    samples_per_pixel: Option<i64>,
    pixel_data_len: Option<usize>,
    frame_of_ref: Option<String>,
    position: Option<[f64; 3]>,
    orientation: Option<[f64; 6]>,
    /// PHI tags found populated: (field name, severity "direct"|"indirect", masked preview).
    phi_hits: Vec<(String, &'static str, String)>,
    /// Count of populated private (odd-group) data elements.
    private_tag_count: usize,
}

/// Trim trailing NULs/spaces; return `None` for empty.
fn clean(s: &str) -> Option<String> {
    let t = s.trim().trim_end_matches('\0').trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn str_of(obj: &DefaultDicomObject, tag: Tag) -> Option<String> {
    obj.element(tag).ok().and_then(|e| e.to_str().ok()).and_then(|s| clean(&s))
}

fn int_of(obj: &DefaultDicomObject, tag: Tag) -> Option<i64> {
    // IS / US / SS — read as string then parse to stay VR-agnostic and lossless.
    if let Some(s) = str_of(obj, tag) {
        if let Ok(v) = s.parse::<i64>() {
            return Some(v);
        }
    }
    obj.element(tag).ok().and_then(|e| e.to_int::<i64>().ok())
}

fn floats_of(obj: &DefaultDicomObject, tag: Tag) -> Option<Vec<f64>> {
    obj.element(tag).ok().and_then(|e| e.value().to_multi_float64().ok())
}

fn mask_preview(s: &str) -> String {
    let n = s.chars().count();
    let head: String = s.chars().take(1).collect();
    format!("{head}*** (len {n})")
}

/// Read one file into an [`Instance`], capturing a parse error rather than
/// panicking on malformed input.
fn read_instance(path: &str) -> Instance {
    let mut inst = Instance {
        path: path.to_string(),
        parse_error: None,
        sop_class_uid: None,
        sop_instance_uid: None,
        series_uid: None,
        study_uid: None,
        modality: None,
        instance_number: None,
        rows: None,
        columns: None,
        bits_allocated: None,
        samples_per_pixel: None,
        pixel_data_len: None,
        frame_of_ref: None,
        position: None,
        orientation: None,
        phi_hits: Vec::new(),
        private_tag_count: 0,
    };

    let obj = match open_file(path) {
        Ok(o) => o,
        Err(e) => {
            inst.parse_error = Some(e.to_string());
            return inst;
        }
    };

    inst.sop_class_uid = str_of(&obj, tags::SOP_CLASS_UID);
    inst.sop_instance_uid = str_of(&obj, tags::SOP_INSTANCE_UID);
    inst.series_uid = str_of(&obj, tags::SERIES_INSTANCE_UID);
    inst.study_uid = str_of(&obj, tags::STUDY_INSTANCE_UID);
    inst.modality = str_of(&obj, tags::MODALITY);
    inst.instance_number = int_of(&obj, tags::INSTANCE_NUMBER);
    inst.rows = int_of(&obj, tags::ROWS);
    inst.columns = int_of(&obj, tags::COLUMNS);
    inst.bits_allocated = int_of(&obj, tags::BITS_ALLOCATED);
    inst.samples_per_pixel = int_of(&obj, tags::SAMPLES_PER_PIXEL);
    inst.frame_of_ref = str_of(&obj, tags::FRAME_OF_REFERENCE_UID);
    inst.pixel_data_len = obj
        .element(tags::PIXEL_DATA)
        .ok()
        .and_then(|e| e.to_bytes().ok())
        .map(|b| b.len());

    if let Some(v) = floats_of(&obj, tags::IMAGE_POSITION_PATIENT) {
        if v.len() == 3 {
            inst.position = Some([v[0], v[1], v[2]]);
        }
    }
    if let Some(v) = floats_of(&obj, tags::IMAGE_ORIENTATION_PATIENT) {
        if v.len() == 6 {
            inst.orientation = Some([v[0], v[1], v[2], v[3], v[4], v[5]]);
        }
    }

    for (tag, name) in PHI_DIRECT {
        if let Some(val) = str_of(&obj, *tag) {
            if !ANON_PLACEHOLDERS.iter().any(|p| p.eq_ignore_ascii_case(&val)) {
                inst.phi_hits.push((name.to_string(), "direct", mask_preview(&val)));
            }
        }
    }
    for (tag, name) in PHI_INDIRECT {
        if let Some(val) = str_of(&obj, *tag) {
            if !ANON_PLACEHOLDERS.iter().any(|p| p.eq_ignore_ascii_case(&val)) {
                inst.phi_hits.push((name.to_string(), "indirect", mask_preview(&val)));
            }
        }
    }

    // Private data elements live in odd-numbered groups (>= 0x0009).
    for elem in obj.iter() {
        let g = elem.header().tag.group();
        if g % 2 == 1 && g >= 0x0009 {
            inst.private_tag_count += 1;
        }
    }

    inst
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// ---------------------------------------------------------------------------
// Checks. Each consumes the parsed instances and returns one ordered Check.
// ---------------------------------------------------------------------------

fn check_ingest(instances: &[Instance]) -> Check {
    let unreadable: Vec<_> = instances
        .iter()
        .filter(|i| i.parse_error.is_some())
        .map(|i| json!({ "path": i.path, "error": i.parse_error }))
        .collect();
    let readable = instances.len() - unreadable.len();
    let (status, detail) = if instances.is_empty() {
        (CheckStatus::Fail, "No files supplied to QC.".to_string())
    } else if readable == 0 {
        (CheckStatus::Fail, "No files were parseable as DICOM.".to_string())
    } else if !unreadable.is_empty() {
        (
            CheckStatus::Fail,
            format!("{} of {} file(s) failed to parse.", unreadable.len(), instances.len()),
        )
    } else {
        (CheckStatus::Pass, format!("All {readable} file(s) parsed as DICOM."))
    };
    Check::new(
        "ingest.readable",
        "Ingest / readability",
        status,
        detail,
        json!({ "total": instances.len(), "readable": readable, "unreadable": unreadable }),
    )
}

fn check_required_tags(readable: &[&Instance]) -> Check {
    let required: &[(&str, fn(&Instance) -> bool)] = &[
        ("SOPClassUID", |i| i.sop_class_uid.is_some()),
        ("SOPInstanceUID", |i| i.sop_instance_uid.is_some()),
        ("SeriesInstanceUID", |i| i.series_uid.is_some()),
        ("StudyInstanceUID", |i| i.study_uid.is_some()),
        ("Modality", |i| i.modality.is_some()),
        ("Rows", |i| i.rows.is_some()),
        ("Columns", |i| i.columns.is_some()),
    ];
    let mut missing = Vec::new();
    for inst in readable {
        let absent: Vec<&str> = required.iter().filter(|(_, f)| !f(inst)).map(|(n, _)| *n).collect();
        if !absent.is_empty() {
            missing.push(json!({ "path": inst.path, "missing": absent }));
        }
    }
    let (status, detail) = if missing.is_empty() {
        (CheckStatus::Pass, "All required header tags present on every instance.".to_string())
    } else {
        (CheckStatus::Fail, format!("{} instance(s) missing required tags.", missing.len()))
    };
    Check::new(
        "header.required_tags",
        "Required header tags",
        status,
        detail,
        json!({ "instances_with_gaps": missing }),
    )
}

fn check_modality_sop(readable: &[&Instance]) -> Check {
    let mut modalities: Vec<String> = readable.iter().filter_map(|i| i.modality.clone()).collect();
    modalities.sort();
    modalities.dedup();
    let unknown: Vec<&String> = modalities.iter().filter(|m| !KNOWN_MODALITIES.contains(&m.as_str())).collect();
    let missing_sop = readable.iter().filter(|i| i.sop_class_uid.is_none()).count();

    let (status, detail) = if !unknown.is_empty() {
        (CheckStatus::Warn, format!("Unrecognised modality code(s): {unknown:?}"))
    } else if modalities.len() > 1 {
        (CheckStatus::Warn, format!("Mixed modalities in one series: {modalities:?}"))
    } else if missing_sop > 0 {
        (CheckStatus::Warn, format!("{missing_sop} instance(s) lack SOPClassUID."))
    } else {
        (CheckStatus::Pass, format!("Modality {modalities:?} recognised; SOP class present."))
    };
    Check::new(
        "modality.sop_sanity",
        "Modality / SOP class sanity",
        status,
        detail,
        json!({ "modalities": modalities, "unknown": unknown, "instances_missing_sop_class": missing_sop }),
    )
}

fn check_series_uniformity(readable: &[&Instance]) -> Check {
    let mut series: Vec<String> = readable.iter().filter_map(|i| i.series_uid.clone()).collect();
    series.sort();
    series.dedup();
    let mut studies: Vec<String> = readable.iter().filter_map(|i| i.study_uid.clone()).collect();
    studies.sort();
    studies.dedup();

    let (status, detail) = if series.len() > 1 {
        (CheckStatus::Fail, format!("{} distinct SeriesInstanceUIDs — not a single series.", series.len()))
    } else if studies.len() > 1 {
        (CheckStatus::Fail, format!("{} distinct StudyInstanceUIDs.", studies.len()))
    } else if series.is_empty() {
        (CheckStatus::Warn, "No SeriesInstanceUID present to verify uniformity.".to_string())
    } else {
        (CheckStatus::Pass, "All instances share one series and study.".to_string())
    };
    Check::new(
        "series.uniformity",
        "Series uniformity",
        status,
        detail,
        json!({ "distinct_series": series.len(), "distinct_studies": studies.len() }),
    )
}

fn check_orientation(geom: &[&Instance]) -> Check {
    let oriented: Vec<[f64; 6]> = geom.iter().filter_map(|i| i.orientation).collect();
    if oriented.len() < 2 {
        return Check::new(
            "geometry.orientation",
            "Slice orientation consistency",
            CheckStatus::Pass,
            "Fewer than two oriented slices — orientation consistency not applicable.".to_string(),
            json!({ "oriented_slices": oriented.len() }),
        );
    }
    let reference = oriented[0];
    let mut max_dev = 0.0_f64;
    for o in &oriented[1..] {
        for k in 0..6 {
            max_dev = max_dev.max((o[k] - reference[k]).abs());
        }
    }
    let (status, detail) = if max_dev <= ORIENTATION_TOL {
        (CheckStatus::Pass, "ImageOrientationPatient consistent across all slices.".to_string())
    } else {
        (
            CheckStatus::Fail,
            format!("ImageOrientationPatient varies across slices (max deviation {max_dev:.4})."),
        )
    };
    Check::new(
        "geometry.orientation",
        "Slice orientation consistency",
        status,
        detail,
        json!({ "max_cosine_deviation": max_dev, "tolerance": ORIENTATION_TOL, "reference": reference }),
    )
}

/// Build the per-slice (instance_number, signed distance along slice normal)
/// list, ordered by instance number. Returns `None` if geometry is unavailable.
fn ordered_projections(geom: &[&Instance]) -> Option<Vec<(i64, f64)>> {
    let reference = geom.iter().find_map(|i| i.orientation)?;
    let row = [reference[0], reference[1], reference[2]];
    let col = [reference[3], reference[4], reference[5]];
    let normal = cross(row, col);
    let mut out: Vec<(i64, f64)> = geom
        .iter()
        .filter_map(|i| Some((i.instance_number?, dot(i.position?, normal))))
        .collect();
    if out.len() < 2 {
        return None;
    }
    out.sort_by_key(|(n, _)| *n);
    Some(out)
}

fn check_position_monotonic(geom: &[&Instance]) -> Check {
    let proj = match ordered_projections(geom) {
        Some(p) => p,
        None => {
            return Check::new(
                "geometry.position_monotonic",
                "Slice position monotonicity",
                CheckStatus::Pass,
                "Insufficient position/orientation/instance-number data — not applicable."
                    .to_string(),
                json!({ "evaluable_slices": 0 }),
            )
        }
    };
    let diffs: Vec<f64> = proj.windows(2).map(|w| w[1].1 - w[0].1).collect();
    let any_pos = diffs.iter().any(|d| *d > 1e-6);
    let any_neg = diffs.iter().any(|d| *d < -1e-6);
    let any_zero = diffs.iter().any(|d| d.abs() <= 1e-6);

    let (status, detail) = if any_pos && any_neg {
        (CheckStatus::Fail, "Slice position reverses direction when ordered by instance number.".to_string())
    } else if any_zero {
        (CheckStatus::Fail, "Duplicate / co-located slice positions detected.".to_string())
    } else {
        (CheckStatus::Pass, "Slice positions advance monotonically with instance number.".to_string())
    };
    Check::new(
        "geometry.position_monotonic",
        "Slice position monotonicity",
        status,
        detail,
        json!({ "ordered_projection_mm": proj.iter().map(|(_, d)| d).collect::<Vec<_>>() }),
    )
}

fn check_slice_spacing(geom: &[&Instance]) -> Check {
    let proj = match ordered_projections(geom) {
        Some(p) => p,
        None => {
            return Check::new(
                "geometry.slice_spacing",
                "Uniform slice spacing",
                CheckStatus::Pass,
                "Insufficient geometry to evaluate slice spacing — not applicable.".to_string(),
                json!({ "evaluable_slices": 0 }),
            )
        }
    };
    let spacings: Vec<f64> = proj.windows(2).map(|w| (w[1].1 - w[0].1).abs()).collect();
    let mean = spacings.iter().sum::<f64>() / spacings.len() as f64;
    let min = spacings.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = spacings.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let spread = max - min;
    let tol = SPACING_ABS_TOL_MM.max(SPACING_REL_TOL * mean);

    let (status, detail) = if spread <= tol {
        (CheckStatus::Pass, format!("Uniform spacing ~{mean:.3} mm (spread {spread:.4} mm)."))
    } else {
        (
            CheckStatus::Warn,
            format!("Non-uniform slice spacing: {min:.3}–{max:.3} mm (spread {spread:.4} mm > tol {tol:.4})."),
        )
    };
    Check::new(
        "geometry.slice_spacing",
        "Uniform slice spacing",
        status,
        detail,
        json!({ "mean_mm": mean, "min_mm": min, "max_mm": max, "spread_mm": spread, "tolerance_mm": tol, "spacings_mm": spacings }),
    )
}

fn check_completeness(readable: &[&Instance]) -> Check {
    let mut numbers: Vec<i64> = readable.iter().filter_map(|i| i.instance_number).collect();
    if numbers.len() < 2 {
        return Check::new(
            "completeness.instance_numbers",
            "Series completeness",
            CheckStatus::Pass,
            "Fewer than two numbered instances — completeness not applicable.".to_string(),
            json!({ "numbered_instances": numbers.len() }),
        );
    }
    numbers.sort();
    let first = *numbers.first().unwrap();
    let last = *numbers.last().unwrap();
    let expected = (last - first + 1) as usize;
    let duplicates = numbers.len() - {
        let mut u = numbers.clone();
        u.dedup();
        u.len()
    };
    let mut gaps = Vec::new();
    let mut present = numbers.clone();
    present.dedup();
    let present_set: std::collections::BTreeSet<i64> = present.into_iter().collect();
    for n in first..=last {
        if !present_set.contains(&n) {
            gaps.push(n);
        }
    }
    let (status, detail) = if gaps.is_empty() && duplicates == 0 {
        (CheckStatus::Pass, format!("Instance numbers {first}–{last} contiguous, no gaps."))
    } else if !gaps.is_empty() {
        (CheckStatus::Fail, format!("{} missing instance number(s) in range {first}–{last}.", gaps.len()))
    } else {
        (CheckStatus::Warn, format!("{duplicates} duplicate instance number(s)."))
    };
    Check::new(
        "completeness.instance_numbers",
        "Series completeness",
        status,
        detail,
        json!({ "first": first, "last": last, "expected": expected, "present": numbers.len(), "missing": gaps, "duplicates": duplicates }),
    )
}

fn check_pixeldata(readable: &[&Instance]) -> Check {
    let mut mismatches = Vec::new();
    let mut missing = 0usize;
    let mut checked = 0usize;
    for inst in readable {
        let (Some(rows), Some(cols)) = (inst.rows, inst.columns) else {
            continue; // can't assess without dimensions (header check owns that)
        };
        let bits = inst.bits_allocated.unwrap_or(16);
        let samples = inst.samples_per_pixel.unwrap_or(1);
        let expected = (rows * cols * samples * bits / 8) as usize;
        match inst.pixel_data_len {
            None => missing += 1,
            Some(actual) => {
                checked += 1;
                // Encapsulated/compressed pixel data is shorter than the raw
                // size; only flag when stored *larger* or grossly truncated.
                if actual != expected && actual < expected {
                    mismatches.push(json!({
                        "path": inst.path, "expected_bytes": expected, "actual_bytes": actual
                    }));
                }
            }
        }
    }
    let (status, detail) = if !mismatches.is_empty() {
        (CheckStatus::Fail, format!("{} instance(s) have PixelData shorter than Rows×Cols×bits imply.", mismatches.len()))
    } else if missing > 0 {
        (CheckStatus::Warn, format!("{missing} instance(s) declare image dimensions but carry no PixelData."))
    } else if checked == 0 {
        (CheckStatus::Pass, "No pixel data to assess.".to_string())
    } else {
        (CheckStatus::Pass, format!("PixelData length consistent with image dimensions on {checked} instance(s)."))
    };
    Check::new(
        "pixeldata.consistency",
        "Pixel data consistency",
        status,
        detail,
        json!({ "checked": checked, "missing_pixeldata": missing, "mismatches": mismatches }),
    )
}

fn check_frame_of_reference(readable: &[&Instance]) -> Check {
    let present: Vec<&String> = readable.iter().filter_map(|i| i.frame_of_ref.as_ref()).collect();
    let mut distinct: Vec<&String> = present.clone();
    distinct.sort();
    distinct.dedup();
    let (status, detail) = if distinct.len() > 1 {
        (CheckStatus::Fail, format!("{} distinct FrameOfReferenceUIDs — slices are not in one spatial frame.", distinct.len()))
    } else if !present.is_empty() && present.len() != readable.len() {
        (CheckStatus::Warn, format!("FrameOfReferenceUID present on {}/{} instances only.", present.len(), readable.len()))
    } else if present.is_empty() {
        (CheckStatus::Pass, "No FrameOfReferenceUID present (uniformly absent).".to_string())
    } else {
        (CheckStatus::Pass, "All instances share one FrameOfReferenceUID.".to_string())
    };
    Check::new(
        "geometry.frame_of_reference",
        "Frame-of-reference consistency",
        status,
        detail,
        json!({ "present": present.len(), "total": readable.len(), "distinct": distinct.len() }),
    )
}

fn check_phi(readable: &[&Instance]) -> Check {
    let mut direct = Vec::new();
    let mut indirect = Vec::new();
    let mut private_total = 0usize;
    for inst in readable {
        private_total += inst.private_tag_count;
        for (field, sev, preview) in &inst.phi_hits {
            let entry = json!({ "path": inst.path, "field": field, "preview": preview });
            if *sev == "direct" {
                direct.push(entry);
            } else {
                indirect.push(entry);
            }
        }
    }
    let (status, detail) = if !direct.is_empty() {
        (
            CheckStatus::Fail,
            format!("{} populated direct-identifier PHI tag(s) found — series is NOT de-identified.", direct.len()),
        )
    } else if !indirect.is_empty() || private_total > 0 {
        (
            CheckStatus::Warn,
            format!(
                "{} quasi-identifier tag(s) and {} private tag(s) present — review before release.",
                indirect.len(),
                private_total
            ),
        )
    } else {
        (CheckStatus::Pass, "No residual PHI detected in scanned identifier tags.".to_string())
    };
    Check::new(
        "anonymization.phi",
        "PHI-leak scan",
        status,
        detail,
        json!({ "direct_identifiers": direct, "quasi_identifiers": indirect, "private_tags": private_total }),
    )
}

/// Validate a DICOM series given the paths to its instances and return a
/// deterministic structured [`Report`].
pub fn qc_series(paths: &[String]) -> Report {
    let instances: Vec<Instance> = paths.iter().map(|p| read_instance(p)).collect();
    let readable: Vec<&Instance> = instances.iter().filter(|i| i.parse_error.is_none()).collect();

    let mut checks = vec![check_ingest(&instances)];
    if !readable.is_empty() {
        checks.push(check_required_tags(&readable));
        checks.push(check_pixeldata(&readable));
        checks.push(check_modality_sop(&readable));
        checks.push(check_series_uniformity(&readable));
        checks.push(check_frame_of_reference(&readable));
        checks.push(check_orientation(&readable));
        checks.push(check_position_monotonic(&readable));
        checks.push(check_slice_spacing(&readable));
        checks.push(check_completeness(&readable));
        checks.push(check_phi(&readable));
    }
    Report::from_checks("dicom_qc", checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom::core::{DataElement, PrimitiveValue, VR};
    use dicom::dictionary_std::uids;
    use dicom::object::{FileMetaTableBuilder, InMemDicomObject};
    use std::path::Path;

    const CT_SOP: &str = "1.2.840.10008.5.1.4.1.1.2";

    fn ds(values: &[f64]) -> String {
        values.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join("\\")
    }

    /// Write a CT instance with sensible defaults, mutated by `build`.
    fn make_file(dir: &Path, name: &str, build: impl FnOnce(&mut InMemDicomObject)) -> String {
        let mut obj = InMemDicomObject::new_empty();
        obj.put(DataElement::new(tags::SOP_CLASS_UID, VR::UI, CT_SOP));
        obj.put(DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, format!("1.2.3.{name}")));
        obj.put(DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, "1.2.3.100"));
        obj.put(DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, "1.2.3.10"));
        obj.put(DataElement::new(tags::MODALITY, VR::CS, "CT"));
        // 8x8, 16-bit, 1 sample -> 128 bytes of pixel data (consistent by default).
        obj.put(DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(8_u16)));
        obj.put(DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(8_u16)));
        obj.put(DataElement::new(tags::BITS_ALLOCATED, VR::US, PrimitiveValue::from(16_u16)));
        obj.put(DataElement::new(tags::SAMPLES_PER_PIXEL, VR::US, PrimitiveValue::from(1_u16)));
        obj.put(DataElement::new(tags::PIXEL_DATA, VR::OB, PrimitiveValue::from(vec![0_u8; 128])));
        obj.put(DataElement::new(
            tags::IMAGE_ORIENTATION_PATIENT,
            VR::DS,
            ds(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
        ));
        build(&mut obj);
        let file_obj = obj
            .with_meta(
                FileMetaTableBuilder::new()
                    .transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN)
                    .media_storage_sop_class_uid(CT_SOP)
                    .implementation_class_uid("1.2.3.4.5"),
            )
            .expect("build file meta");
        let path = dir.join(name);
        file_obj.write_to_file(&path).expect("write dicom");
        path.to_string_lossy().into_owned()
    }

    /// A clean, well-formed 4-slice axial CT series.
    fn clean_series(dir: &Path) -> Vec<String> {
        (1..=4)
            .map(|n| {
                make_file(dir, &format!("clean_{n}.dcm"), |o| {
                    o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, n.to_string()));
                    o.put(DataElement::new(
                        tags::IMAGE_POSITION_PATIENT,
                        VR::DS,
                        ds(&[0.0, 0.0, (n - 1) as f64 * 2.0]),
                    ));
                })
            })
            .collect()
    }

    fn status_of<'a>(report: &'a Report, id: &str) -> &'a CheckStatus {
        &report.checks.iter().find(|c| c.id == id).expect("check present").status
    }

    #[test]
    fn clean_series_passes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = clean_series(dir.path());
        let report = qc_series(&paths);
        assert_eq!(report.verdict, CheckStatus::Pass, "report: {report:?}");
        assert_eq!(report.module, "dicom_qc");
    }

    #[test]
    fn unreadable_file_fails_ingest() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("notdicom.dcm");
        std::fs::write(&bad, b"this is not a dicom file").unwrap();
        let report = qc_series(&[bad.to_string_lossy().into_owned()]);
        assert_eq!(*status_of(&report, "ingest.readable"), CheckStatus::Fail);
        assert_eq!(report.verdict, CheckStatus::Fail);
    }

    #[test]
    fn missing_slice_fails_completeness() {
        let dir = tempfile::tempdir().unwrap();
        // Slices 1,2,4 — instance 3 missing.
        let paths: Vec<String> = [1, 2, 4]
            .iter()
            .map(|&n| {
                make_file(dir.path(), &format!("g_{n}.dcm"), |o| {
                    o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, n.to_string()));
                    o.put(DataElement::new(
                        tags::IMAGE_POSITION_PATIENT,
                        VR::DS,
                        ds(&[0.0, 0.0, (n - 1) as f64 * 2.0]),
                    ));
                })
            })
            .collect();
        let report = qc_series(&paths);
        assert_eq!(*status_of(&report, "completeness.instance_numbers"), CheckStatus::Fail);
    }

    #[test]
    fn inconsistent_orientation_fails_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for n in 1..=3 {
            paths.push(make_file(dir.path(), &format!("o_{n}.dcm"), |o| {
                o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, n.to_string()));
                o.put(DataElement::new(
                    tags::IMAGE_POSITION_PATIENT,
                    VR::DS,
                    ds(&[0.0, 0.0, (n - 1) as f64 * 2.0]),
                ));
                if n == 3 {
                    // Tilt the last slice's orientation.
                    o.put(DataElement::new(
                        tags::IMAGE_ORIENTATION_PATIENT,
                        VR::DS,
                        ds(&[1.0, 0.0, 0.0, 0.0, 0.7071, 0.7071]),
                    ));
                }
            }));
        }
        let report = qc_series(&paths);
        assert_eq!(*status_of(&report, "geometry.orientation"), CheckStatus::Fail);
    }

    #[test]
    fn nonuniform_spacing_warns() {
        let dir = tempfile::tempdir().unwrap();
        let positions = [0.0, 2.0, 4.0, 9.0]; // last gap is 5mm, not 2mm
        let paths: Vec<String> = positions
            .iter()
            .enumerate()
            .map(|(i, &z)| {
                let n = i + 1;
                make_file(dir.path(), &format!("s_{n}.dcm"), |o| {
                    o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, n.to_string()));
                    o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, z])));
                })
            })
            .collect();
        let report = qc_series(&paths);
        assert_eq!(*status_of(&report, "geometry.slice_spacing"), CheckStatus::Warn);
    }

    #[test]
    fn residual_phi_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_file(dir.path(), "phi.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
            o.put(DataElement::new(tags::PATIENT_NAME, VR::PN, "Doe^Jane"));
            o.put(DataElement::new(tags::PATIENT_ID, VR::LO, "MRN-00112233"));
        });
        let report = qc_series(&[path]);
        assert_eq!(*status_of(&report, "anonymization.phi"), CheckStatus::Fail);
    }

    #[test]
    fn anonymised_placeholder_not_flagged_as_phi() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_file(dir.path(), "anon.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
            o.put(DataElement::new(tags::PATIENT_NAME, VR::PN, "ANONYMOUS"));
        });
        let report = qc_series(&[path]);
        assert_ne!(*status_of(&report, "anonymization.phi"), CheckStatus::Fail);
    }

    #[test]
    fn split_series_fails_uniformity() {
        let dir = tempfile::tempdir().unwrap();
        let a = make_file(dir.path(), "a.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
        });
        let b = make_file(dir.path(), "b.dcm", |o| {
            o.put(DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, "1.2.3.999"));
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "2"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 2.0])));
        });
        let report = qc_series(&[a, b]);
        assert_eq!(*status_of(&report, "series.uniformity"), CheckStatus::Fail);
    }

    #[test]
    fn truncated_pixeldata_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_file(dir.path(), "px.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
            // 8x8x16-bit implies 128 bytes; supply only 64.
            o.put(DataElement::new(tags::PIXEL_DATA, VR::OB, PrimitiveValue::from(vec![0_u8; 64])));
        });
        let report = qc_series(&[path]);
        assert_eq!(*status_of(&report, "pixeldata.consistency"), CheckStatus::Fail);
    }

    #[test]
    fn mixed_frame_of_reference_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = make_file(dir.path(), "fa.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
            o.put(DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, "1.2.3.500"));
        });
        let b = make_file(dir.path(), "fb.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "2"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 2.0])));
            o.put(DataElement::new(tags::FRAME_OF_REFERENCE_UID, VR::UI, "1.2.3.501"));
        });
        let report = qc_series(&[a, b]);
        assert_eq!(*status_of(&report, "geometry.frame_of_reference"), CheckStatus::Fail);
    }

    #[test]
    fn empty_input_fails_ingest() {
        let report = qc_series(&[]);
        assert_eq!(*status_of(&report, "ingest.readable"), CheckStatus::Fail);
        assert_eq!(report.verdict, CheckStatus::Fail);
    }

    #[test]
    fn unknown_modality_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_file(dir.path(), "mod.dcm", |o| {
            o.put(DataElement::new(tags::MODALITY, VR::CS, "ZZ"));
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
        });
        let report = qc_series(&[path]);
        assert_eq!(*status_of(&report, "modality.sop_sanity"), CheckStatus::Warn);
    }

    #[test]
    fn quasi_identifier_only_warns_not_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_file(dir.path(), "inst.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
            o.put(DataElement::new(tags::INSTITUTION_NAME, VR::LO, "General Hospital"));
        });
        let report = qc_series(&[path]);
        assert_eq!(*status_of(&report, "anonymization.phi"), CheckStatus::Warn);
    }

    #[test]
    fn duplicate_instance_numbers_warn() {
        let dir = tempfile::tempdir().unwrap();
        let a = make_file(dir.path(), "d1.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
        });
        let b = make_file(dir.path(), "d2.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 2.0])));
        });
        let report = qc_series(&[a, b]);
        assert_eq!(*status_of(&report, "completeness.instance_numbers"), CheckStatus::Warn);
    }

    #[test]
    fn missing_pixeldata_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_file(dir.path(), "nopx.dcm", |o| {
            o.put(DataElement::new(tags::INSTANCE_NUMBER, VR::IS, "1"));
            o.put(DataElement::new(tags::IMAGE_POSITION_PATIENT, VR::DS, ds(&[0.0, 0.0, 0.0])));
            o.remove_element(tags::PIXEL_DATA); // dimensions present, pixels absent
        });
        let report = qc_series(&[path]);
        assert_eq!(*status_of(&report, "pixeldata.consistency"), CheckStatus::Warn);
    }
}
