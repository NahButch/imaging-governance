---
title: Segmentation Governance — Model Proposes, Engine Disposes
emoji: 🫁
colorFrom: green
colorTo: blue
sdk: docker
app_port: 7860
pinned: false
license: mit
---

# Segmentation governance

**Thesis:** a probabilistic segmentation model proposes a label mask; a
deterministic Rust core (`govcore.postcheck_segmentation`) disposes — running
plausibility gates (volume bounds, connectivity/fragments, out-of-field
clipping, label consistency) and emitting a binary **gate decision**. Bad model
output is blocked before it reaches downstream use.

Pick a scenario and see the raw model proposal next to the governance verdict.
Several scenarios are deliberate model failures (fragmented, clipped, oversized)
so you can watch the deterministic gate block them.

## Model flag

The model side is intentionally tiny and CPU-only, chosen by one env flag:

| `MOCK_MODEL` | behaviour |
|---|---|
| `1` (default) | bundled scenarios incl. realistic failure modes |
| `0` | live numpy intensity-threshold segmenter on the sample volume |

## Run locally

```bash
pip install maturin gradio numpy pillow
scripts/build_govcore.sh
python spaces/02-seg-governance/app.py     # http://localhost:7860
```
