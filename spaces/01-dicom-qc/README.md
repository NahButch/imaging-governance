---
title: DICOM QC — Deterministic Governance
emoji: 🩻
colorFrom: indigo
colorTo: blue
sdk: docker
app_port: 7860
pinned: false
license: mit
---

# DICOM QC — deterministic governance

**Thesis:** a probabilistic imaging pipeline proposes a series; a deterministic
Rust core (`govcore`) disposes — validating geometry, completeness, header
conformance and residual PHI with pure, ordered, unit-tested logic. Same series
in, same report out.

Upload a DICOM series (a `.zip` or several `.dcm` files), or click one of the
five bundled examples — one clean control plus four with **known, labeled
defects** (broken geometry, missing slices, residual PHI, header anomaly). The
app is thin Gradio glue; every check runs in Rust.

## Run locally

```bash
# from the repo root
pip install maturin gradio
scripts/build_govcore.sh            # builds the `govcore` extension into your venv
python spaces/01-dicom-qc/app.py    # http://localhost:7860
```

## Deploy

This Space ships as a Docker SDK Space. `scripts/deploy_space.sh 01-dicom-qc`
stages the `govcore` crate next to `app.py`, then the `Dockerfile` builds the
Rust extension with `maturin` and launches Gradio on port 7860.
