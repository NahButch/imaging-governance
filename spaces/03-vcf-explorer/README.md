---
title: VCF Explorer — Deterministic Variant Scoring
emoji: 🧬
colorFrom: purple
colorTo: indigo
sdk: docker
app_port: 7860
pinned: false
license: mit
---

# VCF explorer — auditable, deterministic variant scoring

**Thesis:** a variant caller proposes variants; a deterministic Rust core
(`govcore.score_vcf`) parses the VCF and scores each variant with a transparent
weighted-evidence model. Every score is the **sum of named components**
(consequence/impact, rarity, clinical significance, quality/filter), and each
component carries its **provenance** — the field it came from, the raw value,
and the points. The weight table ships in the report, so the model itself is
auditable. Same VCF in, same scores out.

Upload a `.vcf`, paste VCF text, or load the bundled example (variants spanning
pathogenic→benign, a multi-allelic site, a frameshift indel, and a
failed-filter call).

## Run locally

```bash
pip install maturin gradio
scripts/build_govcore.sh
python spaces/03-vcf-explorer/app.py     # http://localhost:7860
```
