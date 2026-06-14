# govcore

> The deterministic Rust core for [imaging-governance](../../). The model
> proposes; `govcore` disposes.

A single crate, compiled to a native Python module with [`maturin`](https://www.maturin.rs/),
that validates and gates the output of probabilistic medical-imaging / genomics
models with **pure, ordered, unit-tested** logic. No learned parameters, no
randomness, no clock — identical bytes in produce an identical report out.

## The contract

Every function returns the same auditable shape (a Python `dict`): a list of
checks emitted in a fixed order, an overall `verdict` equal to the worst status,
and per-status counts.

```jsonc
{
  "module": "dicom_qc",
  "verdict": "fail",                 // worst of pass | warn | fail
  "summary": "FAIL — 1 failure(s), …",
  "counts": { "pass": 9, "warn": 0, "fail": 1 },
  "checks": [
    { "id": "anonymization.phi", "name": "PHI-leak scan",
      "status": "fail", "detail": "…", "evidence": { /* structured */ } }
  ]
}
```

(`score_vcf` returns a per-variant variant: `{module, summary, parse_ok,
variant_count, variants:[…], scoring_model}`, where each variant's score is the
sum of named, provenance-carrying components.)

## API

```python
import govcore

govcore.ping("hi")                                  # liveness probe
govcore.qc_series(paths)                            # Space #1 — DICOM series QC
govcore.postcheck_segmentation(label, mask, shape, spacing_mm)  # Space #2 — seg gates
govcore.score_vcf(vcf_text)                         # Space #3 — variant scoring
```

| Function | Input | Does |
|----------|-------|------|
| `qc_series` | list of DICOM instance paths | header/pixel-data/geometry/completeness/PHI checks |
| `postcheck_segmentation` | flat label `mask`, `shape` `[z,y,x]`, `spacing_mm` | volume / connectivity / clipping / compactness gates + binary `gated` flag |
| `score_vcf` | full VCF text | parse + weighted-evidence per-variant scores with provenance |

## Build & test

```bash
# Rust unit tests (46 tests, no Python needed)
cargo test

# Build into the active virtualenv as an importable module
pip install maturin
maturin develop            # or: ../../scripts/build_govcore.sh
python -c "import govcore; print(govcore.ping('ok'))"
```

The crate builds an `abi3` wheel (Python ≥ 3.9), CPU-only, no GPU or network
dependency.

## Layout

```
src/
  lib.rs           # #[pymodule] surface + serde→dict bridge
  report.rs        # shared Report / Check / CheckStatus contract
  dicom_qc.rs      # Space #1 checks
  seg_postcheck.rs # Space #2 gates
  vcf_score.rs     # Space #3 scoring
```

License: [MIT](../../LICENSE).
