//! `govcore` — a deterministic governance core for medical-imaging / genomics
//! model I/O.
//!
//! Thesis: **the model proposes, the deterministic engine disposes.** Every
//! probabilistic step (a segmentation network, a variant caller, an upstream
//! scanner) hands its output to this crate, which validates and gates it with
//! pure, ordered, unit-tested logic. The Python layer is glue only — all
//! validation lives here.
//!
//! The crate is compiled to a native Python extension module via `maturin`.
//! Every public callable returns a plain Python object (a `dict`) produced by
//! serialising a Rust [`report::Report`], so callers act on fields, never on
//! parsed text.

use pyo3::prelude::*;
use serde::Serialize;

pub mod dicom_qc;
pub mod report;
pub mod seg_postcheck;
pub mod vcf_score;

/// Serialise any `Serialize` value into a native Python object.
fn to_py<'py, T: Serialize>(py: Python<'py>, value: &T) -> PyResult<Bound<'py, PyAny>> {
    pythonize::pythonize(py, value)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
}

/// Trivial liveness probe proving the Rust↔Python boundary end-to-end.
///
/// Returns a dict ``{"ok": True, "engine": "govcore", "version": ..., "echo": name}``.
#[pyfunction]
fn ping<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    let payload = serde_json::json!({
        "ok": true,
        "engine": "govcore",
        "version": env!("CARGO_PKG_VERSION"),
        "echo": name,
    });
    to_py(py, &payload)
}

/// Run deterministic QC on a DICOM series.
///
/// ``paths`` is a list of filesystem paths to the individual DICOM instances
/// (as Gradio provides on upload). Returns a structured report dict with a
/// per-check ``{status, detail, evidence}`` breakdown and an overall verdict.
#[pyfunction]
fn qc_series<'py>(py: Python<'py>, paths: Vec<String>) -> PyResult<Bound<'py, PyAny>> {
    let report = dicom_qc::qc_series(&paths);
    to_py(py, &report)
}

/// Apply deterministic plausibility gates to a segmentation mask.
///
/// ``mask`` is a flattened label volume (row-major over ``shape``), ``shape``
/// is ``[z, y, x]`` voxel dimensions, and ``spacing_mm`` is the physical voxel
/// spacing ``[z, y, x]`` in millimetres. Returns a structured report dict plus
/// a ``gated`` flag indicating whether the mask passes for downstream use.
#[pyfunction]
fn postcheck_segmentation<'py>(
    py: Python<'py>,
    label: &str,
    mask: Vec<u8>,
    shape: Vec<usize>,
    spacing_mm: Vec<f64>,
) -> PyResult<Bound<'py, PyAny>> {
    let report = seg_postcheck::postcheck_segmentation(label, &mask, &shape, &spacing_mm);
    to_py(py, &report)
}

/// Parse and deterministically score the variants in a VCF document.
///
/// ``vcf_text`` is the full text of a VCF file. Returns a structured report
/// dict with per-variant scores and the provenance of each score component, so
/// every score is auditable rather than a black box.
#[pyfunction]
fn score_vcf<'py>(py: Python<'py>, vcf_text: &str) -> PyResult<Bound<'py, PyAny>> {
    let report = vcf_score::score_vcf(vcf_text);
    to_py(py, &report)
}

/// The native Python module surface.
#[pymodule]
fn govcore(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(ping, m)?)?;
    m.add_function(wrap_pyfunction!(qc_series, m)?)?;
    m.add_function(wrap_pyfunction!(postcheck_segmentation, m)?)?;
    m.add_function(wrap_pyfunction!(score_vcf, m)?)?;
    Ok(())
}
