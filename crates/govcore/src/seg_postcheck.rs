//! Space #2 logic — deterministic segmentation post-checks.
//!
//! A probabilistic segmentation model proposes a label mask; this module
//! disposes by running pure plausibility gates over it and emitting a verdict
//! plus a binary **gate decision** (`gated`) telling downstream code whether the
//! mask is safe to use. No learned parameters — same mask in, same gate out.
//!
//! Mask layout: `mask` is a flattened label volume in row-major `[z, y, x]`
//! order; `shape` is `[z, y, x]` voxel counts; `spacing_mm` is the physical
//! voxel spacing `[z, y, x]` in millimetres. Foreground = any non-zero label.

use crate::report::{Check, CheckStatus, Report};
use serde_json::json;

/// Fraction of foreground that may sit on the outermost voxel plane before we
/// warn / fail. A well-placed structure touches the field edge ~0%, so even a
/// few percent of border contact is a meaningful out-of-field signal.
const BORDER_WARN_FRAC: f64 = 0.03;
const BORDER_FAIL_FRAC: f64 = 0.10;
/// Largest-component share of foreground below which the mask looks fragmented.
const FRAG_WARN_FRAC: f64 = 0.98;
const FRAG_FAIL_FRAC: f64 = 0.90;
/// Bounding-box fill below which a solid structure looks implausibly diffuse.
const COMPACT_WARN_FRAC: f64 = 0.15;
const COMPACT_FAIL_FRAC: f64 = 0.05;

/// Plausible volume ranges (millilitres) for a labeled structure. Deliberately
/// lenient — the point is to catch order-of-magnitude implausibility.
fn volume_bounds_ml(label: &str) -> Option<(f64, f64)> {
    let l = label.trim().to_ascii_lowercase();
    let b = match l.as_str() {
        "liver" => (800.0, 2500.0),
        "spleen" => (50.0, 500.0),
        "kidney" | "kidney_left" | "kidney_right" => (80.0, 400.0),
        "lung" | "lung_left" | "lung_right" => (1000.0, 4000.0),
        "heart" => (450.0, 1000.0),
        "pancreas" => (40.0, 180.0),
        "bladder" => (50.0, 700.0),
        "tumor" | "lesion" | "nodule" | "mass" => (0.05, 1000.0),
        _ => return None,
    };
    Some(b)
}

/// Connected-component analysis result over the foreground (6-connectivity).
struct Components {
    count: usize,
    largest: usize,
    total_foreground: usize,
}

fn connected_components(mask: &[u8], z: usize, y: usize, x: usize) -> Components {
    let idx = |zz: usize, yy: usize, xx: usize| (zz * y + yy) * x + xx;
    let mut visited = vec![false; mask.len()];
    let mut count = 0;
    let mut largest = 0;
    let mut total = 0;
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();

    for zz in 0..z {
        for yy in 0..y {
            for xx in 0..x {
                let i = idx(zz, yy, xx);
                if mask[i] == 0 {
                    continue;
                }
                total += 1;
                if visited[i] {
                    continue;
                }
                // Flood the component containing this foreground voxel.
                count += 1;
                let mut size = 0;
                visited[i] = true;
                stack.push((zz, yy, xx));
                while let Some((cz, cy, cx)) = stack.pop() {
                    size += 1;
                    let push = |nz: usize, ny: usize, nx: usize, st: &mut Vec<_>, vis: &mut Vec<bool>| {
                        let j = idx(nz, ny, nx);
                        if mask[j] != 0 && !vis[j] {
                            vis[j] = true;
                            st.push((nz, ny, nx));
                        }
                    };
                    if cz > 0 { push(cz - 1, cy, cx, &mut stack, &mut visited); }
                    if cz + 1 < z { push(cz + 1, cy, cx, &mut stack, &mut visited); }
                    if cy > 0 { push(cz, cy - 1, cx, &mut stack, &mut visited); }
                    if cy + 1 < y { push(cz, cy + 1, cx, &mut stack, &mut visited); }
                    if cx > 0 { push(cz, cy, cx - 1, &mut stack, &mut visited); }
                    if cx + 1 < x { push(cz, cy, cx + 1, &mut stack, &mut visited); }
                }
                largest = largest.max(size);
            }
        }
    }
    Components { count, largest, total_foreground: total }
}

fn border_foreground(mask: &[u8], z: usize, y: usize, x: usize) -> usize {
    let idx = |zz: usize, yy: usize, xx: usize| (zz * y + yy) * x + xx;
    let mut on_border = 0;
    for zz in 0..z {
        for yy in 0..y {
            for xx in 0..x {
                if mask[idx(zz, yy, xx)] == 0 {
                    continue;
                }
                if zz == 0 || zz + 1 == z || yy == 0 || yy + 1 == y || xx == 0 || xx + 1 == x {
                    on_border += 1;
                }
            }
        }
    }
    on_border
}

/// Fraction of the foreground's axis-aligned bounding box that is foreground,
/// plus the bounding-box dimensions `[dz, dy, dx]`. A compact solid fills a
/// large share of its box; scattered or thin masks fill very little.
fn bbox_fill(mask: &[u8], z: usize, y: usize, x: usize) -> (f64, [usize; 3]) {
    let idx = |zz: usize, yy: usize, xx: usize| (zz * y + yy) * x + xx;
    let (mut z0, mut y0, mut x0) = (usize::MAX, usize::MAX, usize::MAX);
    let (mut z1, mut y1, mut x1) = (0usize, 0usize, 0usize);
    let mut fg = 0usize;
    for zz in 0..z {
        for yy in 0..y {
            for xx in 0..x {
                if mask[idx(zz, yy, xx)] != 0 {
                    fg += 1;
                    z0 = z0.min(zz); y0 = y0.min(yy); x0 = x0.min(xx);
                    z1 = z1.max(zz); y1 = y1.max(yy); x1 = x1.max(xx);
                }
            }
        }
    }
    if fg == 0 {
        return (0.0, [0, 0, 0]);
    }
    let dims = [z1 - z0 + 1, y1 - y0 + 1, x1 - x0 + 1];
    let bbox_vol = dims[0] * dims[1] * dims[2];
    (fg as f64 / bbox_vol as f64, dims)
}

/// Apply deterministic plausibility gates to a segmentation mask.
pub fn postcheck_segmentation(label: &str, mask: &[u8], shape: &[usize], spacing_mm: &[f64]) -> Report {
    // ---- shape validity (fail-fast) ----
    if shape.len() != 3 || spacing_mm.len() != 3 {
        let c = Check::new(
            "shape.valid",
            "Mask shape validity",
            CheckStatus::Fail,
            "shape and spacing_mm must both be 3-element [z, y, x].".to_string(),
            json!({ "shape_len": shape.len(), "spacing_len": spacing_mm.len() }),
        );
        return Report::from_checks("seg_postcheck", with_gate(vec![c]));
    }
    let (z, y, x) = (shape[0], shape[1], shape[2]);
    if z * y * x != mask.len() || mask.is_empty() {
        let c = Check::new(
            "shape.valid",
            "Mask shape validity",
            CheckStatus::Fail,
            format!("shape {z}x{y}x{x} = {} does not match mask length {}.", z * y * x, mask.len()),
            json!({ "shape": [z, y, x], "mask_len": mask.len() }),
        );
        return Report::from_checks("seg_postcheck", with_gate(vec![c]));
    }

    let voxel_vol_mm3 = spacing_mm[0] * spacing_mm[1] * spacing_mm[2];
    let foreground = mask.iter().filter(|&&v| v != 0).count();
    let total = mask.len();
    let volume_ml = foreground as f64 * voxel_vol_mm3 / 1000.0;

    let mut checks = Vec::new();
    checks.push(Check::new(
        "shape.valid",
        "Mask shape validity",
        CheckStatus::Pass,
        format!("Mask is a valid {z}x{y}x{x} volume ({total} voxels)."),
        json!({ "shape": [z, y, x], "voxel_volume_mm3": voxel_vol_mm3 }),
    ));

    // ---- label consistency ----
    let mut distinct: Vec<u8> = mask.iter().copied().collect();
    distinct.sort_unstable();
    distinct.dedup();
    let multi_label = distinct.iter().any(|&v| v > 1);
    let (lstatus, ldetail) = if foreground == 0 {
        (CheckStatus::Fail, "Mask is empty — no foreground voxels.".to_string())
    } else if foreground == total {
        (CheckStatus::Fail, "Entire volume is labeled foreground — implausible.".to_string())
    } else if multi_label {
        (CheckStatus::Warn, format!("Mask carries multiple label values {distinct:?}; gate expects binary."))
    } else {
        (CheckStatus::Pass, format!("Binary mask: {foreground}/{total} voxels foreground."))
    };
    checks.push(Check::new(
        "label.consistency",
        "Label consistency",
        lstatus,
        ldetail,
        json!({ "distinct_labels": distinct, "foreground": foreground, "total": total }),
    ));

    // ---- volume plausibility ----
    let (vstatus, vdetail, bounds_json) = match volume_bounds_ml(label) {
        _ if foreground == 0 => (
            CheckStatus::Fail,
            "No volume to assess (empty mask).".to_string(),
            json!(null),
        ),
        Some((lo, hi)) => {
            if volume_ml < lo {
                (CheckStatus::Fail, format!("{volume_ml:.3} mL is below the plausible {lo}–{hi} mL for '{label}'."), json!([lo, hi]))
            } else if volume_ml > hi {
                (CheckStatus::Fail, format!("{volume_ml:.3} mL exceeds the plausible {lo}–{hi} mL for '{label}'."), json!([lo, hi]))
            } else {
                (CheckStatus::Pass, format!("{volume_ml:.3} mL within plausible {lo}–{hi} mL for '{label}'."), json!([lo, hi]))
            }
        }
        None => (
            CheckStatus::Warn,
            format!("No anatomical volume bounds known for label '{label}' ({volume_ml:.3} mL)."),
            json!(null),
        ),
    };
    checks.push(Check::new(
        "volume.plausibility",
        "Volume plausibility",
        vstatus,
        vdetail,
        json!({ "label": label, "volume_ml": volume_ml, "bounds_ml": bounds_json, "voxel_volume_mm3": voxel_vol_mm3 }),
    ));

    // ---- connectivity / fragmentation ----
    let comp = connected_components(mask, z, y, x);
    let largest_frac = if comp.total_foreground > 0 {
        comp.largest as f64 / comp.total_foreground as f64
    } else {
        0.0
    };
    let (cstatus, cdetail) = if foreground == 0 {
        (CheckStatus::Fail, "No components (empty mask).".to_string())
    } else if comp.count == 1 || largest_frac >= FRAG_WARN_FRAC {
        (CheckStatus::Pass, format!("{} component(s); dominant component holds {:.1}% of foreground.", comp.count, largest_frac * 100.0))
    } else if largest_frac < FRAG_FAIL_FRAC {
        (CheckStatus::Fail, format!("Fragmented: {} components, largest only {:.1}% of foreground.", comp.count, largest_frac * 100.0))
    } else {
        (CheckStatus::Warn, format!("{} components; dominant holds {:.1}% — minor fragments present.", comp.count, largest_frac * 100.0))
    };
    checks.push(Check::new(
        "connectivity.fragments",
        "Connectivity / fragment sanity",
        cstatus,
        cdetail,
        json!({ "components": comp.count, "largest_voxels": comp.largest, "largest_fraction": largest_frac }),
    ));

    // ---- boundary / out-of-field ----
    let border = border_foreground(mask, z, y, x);
    let border_frac = if foreground > 0 { border as f64 / foreground as f64 } else { 0.0 };
    let (bstatus, bdetail) = if foreground == 0 {
        (CheckStatus::Pass, "No foreground to assess for clipping.".to_string())
    } else if border_frac > BORDER_FAIL_FRAC {
        (CheckStatus::Fail, format!("{:.1}% of foreground touches the volume border — structure is clipped / out of field.", border_frac * 100.0))
    } else if border_frac > BORDER_WARN_FRAC {
        (CheckStatus::Warn, format!("{:.1}% of foreground touches the border — possible truncation.", border_frac * 100.0))
    } else {
        (CheckStatus::Pass, format!("Only {:.1}% of foreground touches the border — within field.", border_frac * 100.0))
    };
    checks.push(Check::new(
        "boundary.out_of_field",
        "Boundary / out-of-field",
        bstatus,
        bdetail,
        json!({ "border_voxels": border, "border_fraction": border_frac, "warn_at": BORDER_WARN_FRAC, "fail_at": BORDER_FAIL_FRAC }),
    ));

    // ---- compactness (bounding-box fill) ----
    let (fill, bbox) = bbox_fill(mask, z, y, x);
    let (cpstatus, cpdetail) = if foreground == 0 {
        (CheckStatus::Pass, "No foreground to assess for compactness.".to_string())
    } else if fill < COMPACT_FAIL_FRAC {
        (CheckStatus::Fail, format!("Fills only {:.1}% of its bounding box — implausibly diffuse for a solid structure.", fill * 100.0))
    } else if fill < COMPACT_WARN_FRAC {
        (CheckStatus::Warn, format!("Fills {:.1}% of its bounding box — sparse/elongated shape.", fill * 100.0))
    } else {
        (CheckStatus::Pass, format!("Fills {:.1}% of its bounding box — compact.", fill * 100.0))
    };
    checks.push(Check::new(
        "compactness.bbox_fill",
        "Shape compactness",
        cpstatus,
        cpdetail,
        json!({ "bbox_fill_fraction": fill, "bounding_box": bbox, "warn_at": COMPACT_WARN_FRAC, "fail_at": COMPACT_FAIL_FRAC }),
    ));

    Report::from_checks("seg_postcheck", with_gate(checks))
}

/// Append the terminal gate-decision check. `gated = true` iff nothing failed,
/// i.e. the mask is safe to pass downstream.
fn with_gate(mut checks: Vec<Check>) -> Vec<Check> {
    let worst = checks.iter().map(|c| c.status).max().unwrap_or(CheckStatus::Pass);
    let gated = worst != CheckStatus::Fail;
    let (status, detail) = if gated {
        (worst, "Mask GATED THROUGH — passes deterministic plausibility for downstream use.".to_string())
    } else {
        (CheckStatus::Fail, "Mask BLOCKED — failed a deterministic plausibility gate.".to_string())
    };
    checks.push(Check::new(
        "gate.decision",
        "Gate decision",
        status,
        detail,
        json!({ "gated": gated }),
    ));
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `z*y*x` mask with a solid foreground box `[z0,z1) x [y0,y1) x [x0,x1)`.
    fn box_mask(
        (z, y, x): (usize, usize, usize),
        (z0, z1): (usize, usize),
        (y0, y1): (usize, usize),
        (x0, x1): (usize, usize),
        value: u8,
    ) -> Vec<u8> {
        let mut m = vec![0u8; z * y * x];
        for zz in z0..z1 {
            for yy in y0..y1 {
                for xx in x0..x1 {
                    m[(zz * y + yy) * x + xx] = value;
                }
            }
        }
        m
    }

    fn check<'a>(r: &'a Report, id: &str) -> &'a Check {
        r.checks.iter().find(|c| c.id == id).expect("check present")
    }

    fn gated(r: &Report) -> bool {
        check(r, "gate.decision").evidence["gated"].as_bool().unwrap()
    }

    #[test]
    fn plausible_single_blob_passes_and_gates() {
        let shape = [10, 10, 10];
        // 5x5x5 centered cube, not touching border.
        let mask = box_mask((10, 10, 10), (2, 7), (2, 7), (2, 7), 1);
        let r = postcheck_segmentation("lesion", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(r.verdict, CheckStatus::Pass, "{r:?}");
        assert!(gated(&r));
    }

    #[test]
    fn empty_mask_fails_and_blocks() {
        let shape = [8, 8, 8];
        let mask = vec![0u8; 8 * 8 * 8];
        let r = postcheck_segmentation("lesion", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(r.verdict, CheckStatus::Fail);
        assert!(!gated(&r));
    }

    #[test]
    fn fragmented_mask_fails_connectivity() {
        let shape = [10, 10, 10];
        // Two equal, separated 2x2x2 blobs -> largest fraction 0.5.
        let mut mask = box_mask((10, 10, 10), (1, 3), (1, 3), (1, 3), 1);
        for (i, v) in box_mask((10, 10, 10), (6, 8), (6, 8), (6, 8), 1).into_iter().enumerate() {
            if v != 0 {
                mask[i] = 1;
            }
        }
        let r = postcheck_segmentation("lesion", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(check(&r, "connectivity.fragments").status, CheckStatus::Fail, "{r:?}");
        assert!(!gated(&r));
    }

    #[test]
    fn border_touching_warns_or_fails() {
        let shape = [10, 10, 10];
        // Blob jammed into the corner — much of it on the border.
        let mask = box_mask((10, 10, 10), (0, 3), (0, 3), (0, 3), 1);
        let r = postcheck_segmentation("lesion", &mask, &shape, &[1.0, 1.0, 1.0]);
        let s = check(&r, "boundary.out_of_field").status;
        assert!(matches!(s, CheckStatus::Warn | CheckStatus::Fail), "{r:?}");
    }

    #[test]
    fn volume_implausible_for_liver_fails() {
        let shape = [10, 10, 10];
        // A tiny blob can't be a liver (needs ~800 mL).
        let mask = box_mask((10, 10, 10), (2, 5), (2, 5), (2, 5), 1);
        let r = postcheck_segmentation("liver", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(check(&r, "volume.plausibility").status, CheckStatus::Fail, "{r:?}");
        assert!(!gated(&r));
    }

    #[test]
    fn unknown_label_warns_but_may_gate() {
        let shape = [10, 10, 10];
        let mask = box_mask((10, 10, 10), (2, 7), (2, 7), (2, 7), 1);
        let r = postcheck_segmentation("widget", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(check(&r, "volume.plausibility").status, CheckStatus::Warn);
        assert!(gated(&r)); // warn does not block
    }

    #[test]
    fn multi_label_warns() {
        let shape = [10, 10, 10];
        let mut mask = box_mask((10, 10, 10), (2, 7), (2, 7), (2, 7), 1);
        mask[(3 * 10 + 3) * 10 + 3] = 5; // stray second label
        let r = postcheck_segmentation("lesion", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(check(&r, "label.consistency").status, CheckStatus::Warn);
    }

    #[test]
    fn diffuse_scatter_fails_compactness() {
        let shape = [10, 10, 10];
        // A few stray voxels spread to opposite corners: huge bbox, tiny fill.
        let mut mask = vec![0u8; 1000];
        for (zz, yy, xx) in [(1, 1, 1), (1, 1, 8), (8, 8, 1), (8, 8, 8)] {
            mask[(zz * 10 + yy) * 10 + xx] = 1;
        }
        let r = postcheck_segmentation("lesion", &mask, &shape, &[1.0, 1.0, 1.0]);
        assert_eq!(check(&r, "compactness.bbox_fill").status, CheckStatus::Fail, "{r:?}");
        assert!(!gated(&r));
    }

    #[test]
    fn bad_shape_fails_fast() {
        let r = postcheck_segmentation("lesion", &[1, 0, 1], &[2, 2], &[1.0, 1.0, 1.0]);
        assert_eq!(r.verdict, CheckStatus::Fail);
        assert!(!gated(&r));
    }
}
