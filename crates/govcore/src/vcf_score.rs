//! Space #3 logic — deterministic VCF parsing + weighted-evidence scoring.
//!
//! A variant caller proposes variants; this module disposes by scoring each one
//! with a **transparent, deterministic** weighted-evidence model. Every score is
//! the sum of named components, and each component carries its provenance (which
//! field it came from, the raw value, the weight, the points) — so the score is
//! auditable, never a black box. The scoring weights themselves are emitted in
//! the report. Same VCF in, same scores out.
//!
//! Fresh, self-contained implementation — no dependency on any external VCF
//! tooling.

use serde::Serialize;
use serde_json::{json, Value};

// --- scoring weights (emitted in the report for auditability) ---
const IMPACT_HIGH: f64 = 5.0;
const IMPACT_MODERATE: f64 = 3.0;
const IMPACT_LOW: f64 = 1.0;
const IMPACT_MODIFIER: f64 = 0.5;
const FRAMESHIFT_BONUS: f64 = 2.0;

const CLNSIG_PATHOGENIC: f64 = 4.0;
const CLNSIG_LIKELY_PATHOGENIC: f64 = 3.0;
const CLNSIG_VUS: f64 = 0.5;
const CLNSIG_LIKELY_BENIGN: f64 = -2.0;
const CLNSIG_BENIGN: f64 = -3.0;

const TIER_HIGH: f64 = 8.0;
const TIER_MODERATE: f64 = 4.0;
const TIER_LOW: f64 = 1.5;

/// One named, auditable contribution to a variant's total score.
#[derive(Debug, Clone, Serialize)]
pub struct ScoreComponent {
    pub name: String,
    /// Provenance: where the value came from and how it mapped to points.
    pub detail: String,
    pub value: Value,
    pub points: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantScore {
    pub chrom: String,
    pub pos: String,
    pub id: String,
    pub reference: String,
    pub alt: String,
    pub variant_class: String,
    pub filter: String,
    pub total_score: f64,
    pub tier: String,
    pub components: Vec<ScoreComponent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreReport {
    pub module: String,
    pub summary: String,
    pub parse_ok: bool,
    pub parse_errors: Vec<String>,
    pub variant_count: usize,
    pub variants: Vec<VariantScore>,
    /// The weight table, so a reviewer can audit the model itself.
    pub scoring_model: Value,
}

fn parse_info(info: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    if info == "." || info.is_empty() {
        return map;
    }
    for token in info.split(';') {
        if token.is_empty() {
            continue;
        }
        match token.split_once('=') {
            Some((k, v)) => {
                map.insert(k.to_string(), v.to_string());
            }
            None => {
                map.insert(token.to_string(), "true".to_string());
            }
        }
    }
    map
}

/// Look up the first present key (case-sensitive) from a list of aliases,
/// returning the matched key and its value.
fn first_of(
    info: &std::collections::BTreeMap<String, String>,
    keys: &[&str],
) -> Option<(String, String)> {
    keys.iter().find_map(|k| info.get(*k).map(|v| (k.to_string(), v.clone())))
}

fn classify(reference: &str, alt: &str) -> (String, bool) {
    if alt.starts_with('<') || alt == "*" {
        return ("symbolic".to_string(), false);
    }
    let (rl, al) = (reference.len(), alt.len());
    let frameshift = rl != al && (rl as i64 - al as i64).unsigned_abs() % 3 != 0;
    let class = if rl == 1 && al == 1 {
        "SNV"
    } else if rl == al {
        "MNV"
    } else if rl < al {
        "insertion"
    } else {
        "deletion"
    };
    (class.to_string(), frameshift)
}

fn impact_component(
    info: &std::collections::BTreeMap<String, String>,
    class: &str,
    frameshift: bool,
) -> ScoreComponent {
    // Prefer an explicit IMPACT (e.g. from SnpEff/VEP), else the ANN field's
    // impact slot, else infer from the variant class.
    let impact = info.get("IMPACT").cloned().or_else(|| {
        info.get("ANN").and_then(|ann| ann.split('|').nth(2).map(|s| s.to_string()))
    });
    let (points, detail, value) = match impact.as_deref().map(|s| s.to_ascii_uppercase()) {
        Some(ref i) if i == "HIGH" => (IMPACT_HIGH, "IMPACT=HIGH (annotation)", json!("HIGH")),
        Some(ref i) if i == "MODERATE" => (IMPACT_MODERATE, "IMPACT=MODERATE (annotation)", json!("MODERATE")),
        Some(ref i) if i == "LOW" => (IMPACT_LOW, "IMPACT=LOW (annotation)", json!("LOW")),
        Some(ref i) if i == "MODIFIER" => (IMPACT_MODIFIER, "IMPACT=MODIFIER (annotation)", json!("MODIFIER")),
        _ => {
            // Inferred from variant class.
            let base = match class {
                "insertion" | "deletion" => 2.0,
                "MNV" => 1.5,
                "SNV" => 1.0,
                _ => 0.5,
            };
            (base, "inferred from variant class (no IMPACT annotation)", json!(class))
        }
    };
    let mut total = points;
    let mut detail = detail.to_string();
    if frameshift {
        total += FRAMESHIFT_BONUS;
        detail.push_str(&format!("; +{FRAMESHIFT_BONUS} frameshift (indel length not a multiple of 3)"));
    }
    ScoreComponent { name: "consequence_impact".to_string(), detail, value, points: total }
}

fn rarity_component(info: &std::collections::BTreeMap<String, String>) -> ScoreComponent {
    let af = first_of(info, &["gnomAD_AF", "AF_popmax", "AF", "MAF"]);
    match af.and_then(|(k, v)| v.split(',').next().and_then(|x| x.parse::<f64>().ok()).map(|f| (k, f))) {
        Some((key, f)) => {
            let (points, bucket) = if f < 0.0001 {
                (3.0, "ultra-rare (<0.01%)")
            } else if f < 0.001 {
                (2.0, "rare (<0.1%)")
            } else if f < 0.01 {
                (1.0, "uncommon (<1%)")
            } else if f < 0.05 {
                (0.5, "low-frequency (<5%)")
            } else {
                (0.0, "common (>=5%)")
            };
            ScoreComponent {
                name: "rarity".to_string(),
                detail: format!("{key}={f} -> {bucket}"),
                value: json!(f),
                points,
            }
        }
        None => ScoreComponent {
            name: "rarity".to_string(),
            detail: "no allele-frequency field (AF/gnomAD_AF) present".to_string(),
            value: json!(null),
            points: 0.0,
        },
    }
}

fn clinical_component(info: &std::collections::BTreeMap<String, String>) -> ScoreComponent {
    match info.get("CLNSIG") {
        Some(raw) => {
            let l = raw.to_ascii_lowercase();
            let (points, label) = if l.contains("likely_pathogenic") {
                (CLNSIG_LIKELY_PATHOGENIC, "likely pathogenic")
            } else if l.contains("pathogenic") {
                (CLNSIG_PATHOGENIC, "pathogenic")
            } else if l.contains("likely_benign") {
                (CLNSIG_LIKELY_BENIGN, "likely benign")
            } else if l.contains("benign") {
                (CLNSIG_BENIGN, "benign")
            } else if l.contains("uncertain") || l.contains("vus") {
                (CLNSIG_VUS, "uncertain significance")
            } else {
                (0.0, "unscored clinical term")
            };
            ScoreComponent {
                name: "clinical_significance".to_string(),
                detail: format!("CLNSIG={raw} -> {label}"),
                value: json!(raw),
                points,
            }
        }
        None => ScoreComponent {
            name: "clinical_significance".to_string(),
            detail: "no CLNSIG field present".to_string(),
            value: json!(null),
            points: 0.0,
        },
    }
}

fn quality_component(
    qual: &str,
    filter: &str,
    info: &std::collections::BTreeMap<String, String>,
) -> ScoreComponent {
    let mut points = 0.0;
    let mut notes = Vec::new();
    if let Ok(q) = qual.parse::<f64>() {
        if q >= 50.0 {
            points += 1.0;
            notes.push(format!("QUAL={q} (+1.0 high confidence)"));
        } else if q >= 20.0 {
            points += 0.5;
            notes.push(format!("QUAL={q} (+0.5)"));
        } else {
            notes.push(format!("QUAL={q} (low)"));
        }
    }
    if let Some(dp) = info.get("DP").and_then(|v| v.parse::<f64>().ok()) {
        if dp < 10.0 {
            points -= 1.0;
            notes.push(format!("DP={dp} (-1.0 low depth)"));
        } else {
            notes.push(format!("DP={dp}"));
        }
    }
    if !(filter == "PASS" || filter == ".") {
        points -= 2.0;
        notes.push(format!("FILTER={filter} (-2.0 not PASS)"));
    } else {
        notes.push(format!("FILTER={filter}"));
    }
    ScoreComponent {
        name: "quality_filter".to_string(),
        detail: notes.join("; "),
        value: json!({ "qual": qual, "filter": filter }),
        points,
    }
}

fn tier_of(score: f64) -> &'static str {
    if score >= TIER_HIGH {
        "high"
    } else if score >= TIER_MODERATE {
        "moderate"
    } else if score >= TIER_LOW {
        "low"
    } else if score >= 0.0 {
        "minimal"
    } else {
        "benign-leaning"
    }
}

fn score_record(
    chrom: &str,
    pos: &str,
    id: &str,
    reference: &str,
    alt: &str,
    qual: &str,
    filter: &str,
    info: &std::collections::BTreeMap<String, String>,
) -> VariantScore {
    let (class, frameshift) = classify(reference, alt);
    let components = vec![
        impact_component(info, &class, frameshift),
        rarity_component(info),
        clinical_component(info),
        quality_component(qual, filter, info),
    ];
    let total: f64 = components.iter().map(|c| c.points).sum();
    let total = (total * 1000.0).round() / 1000.0; // stable rounding
    VariantScore {
        chrom: chrom.to_string(),
        pos: pos.to_string(),
        id: id.to_string(),
        reference: reference.to_string(),
        alt: alt.to_string(),
        variant_class: class,
        filter: filter.to_string(),
        total_score: total,
        tier: tier_of(total).to_string(),
        components,
    }
}

fn scoring_model_json() -> Value {
    json!({
        "consequence_impact": { "HIGH": IMPACT_HIGH, "MODERATE": IMPACT_MODERATE, "LOW": IMPACT_LOW, "MODIFIER": IMPACT_MODIFIER, "frameshift_bonus": FRAMESHIFT_BONUS },
        "clinical_significance": { "pathogenic": CLNSIG_PATHOGENIC, "likely_pathogenic": CLNSIG_LIKELY_PATHOGENIC, "vus": CLNSIG_VUS, "likely_benign": CLNSIG_LIKELY_BENIGN, "benign": CLNSIG_BENIGN },
        "rarity_points": { "lt_0.0001": 3.0, "lt_0.001": 2.0, "lt_0.01": 1.0, "lt_0.05": 0.5, "ge_0.05": 0.0 },
        "quality_filter": { "qual_ge_50": 1.0, "qual_ge_20": 0.5, "dp_lt_10": -1.0, "filter_not_pass": -2.0 },
        "tiers": { "high": TIER_HIGH, "moderate": TIER_MODERATE, "low": TIER_LOW }
    })
}

/// Parse and deterministically score the variants in a VCF document.
pub fn score_vcf(vcf_text: &str) -> ScoreReport {
    let mut variants = Vec::new();
    let mut parse_errors = Vec::new();
    let mut saw_header = false;
    let mut saw_fileformat = false;

    for (lineno, raw) in vcf_text.lines().enumerate() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("##") {
            if line.starts_with("##fileformat=") {
                saw_fileformat = true;
            }
            continue;
        }
        if line.starts_with('#') {
            saw_header = true; // the #CHROM column header
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 8 {
            parse_errors.push(format!("line {}: expected >=8 tab-separated fields, got {}", lineno + 1, fields.len()));
            continue;
        }
        let (chrom, pos, id, reference, alt_field, qual, filter, info_field) =
            (fields[0], fields[1], fields[2], fields[3], fields[4], fields[5], fields[6], fields[7]);
        let info = parse_info(info_field);
        // Multi-allelic records: score each ALT independently.
        for alt in alt_field.split(',') {
            variants.push(score_record(chrom, pos, id, reference, alt, qual, filter, &info));
        }
    }

    let parse_ok = saw_header && parse_errors.is_empty();
    let summary = if !saw_fileformat && variants.is_empty() && parse_errors.is_empty() {
        "No VCF content recognised.".to_string()
    } else {
        let high = variants.iter().filter(|v| v.tier == "high").count();
        let moderate = variants.iter().filter(|v| v.tier == "moderate").count();
        format!(
            "{} variant call(s) scored — {high} high, {moderate} moderate tier{}.",
            variants.len(),
            if parse_errors.is_empty() { "" } else { " (with parse errors)" }
        )
    };

    ScoreReport {
        module: "vcf_score".to_string(),
        summary,
        parse_ok,
        parse_errors,
        variant_count: variants.len(),
        variants,
        scoring_model: scoring_model_json(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "##fileformat=VCFv4.2\n#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO";

    fn vcf(body: &str) -> String {
        format!("{HEADER}\n{body}")
    }

    fn only(report: &ScoreReport) -> &VariantScore {
        assert_eq!(report.variants.len(), 1, "{report:?}");
        &report.variants[0]
    }

    #[test]
    fn pathogenic_rare_high_impact_scores_high() {
        let r = score_vcf(&vcf(
            "1\t100\trs1\tA\tT\t80\tPASS\tIMPACT=HIGH;AF=0.00005;CLNSIG=Pathogenic;DP=60",
        ));
        let v = only(&r);
        assert_eq!(v.tier, "high", "{v:?}");
        // 5 (HIGH) + 3 (ultra-rare) + 4 (pathogenic) + 1 (QUAL>=50) = 13
        assert!(v.total_score >= TIER_HIGH);
        assert!(r.parse_ok);
    }

    #[test]
    fn common_benign_scores_low() {
        let r = score_vcf(&vcf(
            "1\t200\trs2\tG\tC\t60\tPASS\tIMPACT=MODIFIER;AF=0.45;CLNSIG=Benign;DP=40",
        ));
        let v = only(&r);
        assert!(v.total_score < TIER_LOW, "{v:?}");
        assert_eq!(v.tier, "benign-leaning");
    }

    #[test]
    fn multiallelic_splits_into_two() {
        let r = score_vcf(&vcf("1\t300\t.\tA\tT,G\t50\tPASS\tAF=0.001"));
        assert_eq!(r.variant_count, 2);
        assert_eq!(r.variants[0].alt, "T");
        assert_eq!(r.variants[1].alt, "G");
    }

    #[test]
    fn frameshift_gets_bonus() {
        // single-base deletion -> length diff 1, not multiple of 3 -> frameshift
        let r = score_vcf(&vcf("1\t400\t.\tAC\tA\t50\tPASS\t."));
        let v = only(&r);
        assert_eq!(v.variant_class, "deletion");
        let impact = v.components.iter().find(|c| c.name == "consequence_impact").unwrap();
        assert!(impact.detail.contains("frameshift"), "{impact:?}");
    }

    #[test]
    fn failed_filter_penalised() {
        let r = score_vcf(&vcf("1\t500\t.\tA\tT\t10\tq10\tAF=0.2;DP=5"));
        let v = only(&r);
        let q = v.components.iter().find(|c| c.name == "quality_filter").unwrap();
        assert!(q.points < 0.0, "{q:?}");
    }

    #[test]
    fn malformed_line_recorded() {
        let r = score_vcf(&vcf("1\t600\tbad_line_too_few_fields"));
        assert!(!r.parse_errors.is_empty());
        assert!(!r.parse_ok);
    }

    #[test]
    fn provenance_present_on_every_component() {
        let r = score_vcf(&vcf("1\t700\t.\tA\tT\t99\tPASS\tIMPACT=MODERATE;AF=0.0003;CLNSIG=Uncertain_significance;DP=30"));
        let v = only(&r);
        assert_eq!(v.components.len(), 4);
        for c in &v.components {
            assert!(!c.detail.is_empty(), "component {} lacks provenance", c.name);
        }
    }
}
