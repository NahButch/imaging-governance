"""The probabilistic 'proposer' for Space #2.

The governance layer is the point of this Space, not the network — so the model
side is deliberately tiny and CPU-only, selected by a single env flag:

  MOCK_MODEL=1 (default)  -> bundled scenarios whose proposed masks include
                             realistic failure modes (fragmented, clipped,
                             oversized) so the deterministic gate visibly blocks
                             bad output.
  MOCK_MODEL=0            -> a live intensity-threshold segmenter (numpy only,
                             no torch) run on the same synthetic volumes.

Either way the output is the same: a labeled structure, a synthetic source
volume, a proposed mask, and the voxel spacing — handed to govcore for gating.
"""

from __future__ import annotations

import os
from dataclasses import dataclass

import numpy as np

SHAPE = (48, 48, 48)  # z, y, x — small enough for free-tier CPU
SPACING = (4.0, 4.0, 4.0)  # mm; voxel = 64 mm^3


@dataclass
class Proposal:
    name: str
    label: str
    volume: np.ndarray  # float32 source volume (z, y, x)
    mask: np.ndarray  # uint8 proposed segmentation (z, y, x)
    spacing: tuple[float, float, float]
    note: str


def _grid():
    z, y, x = SHAPE
    zz, yy, xx = np.meshgrid(np.arange(z), np.arange(y), np.arange(x), indexing="ij")
    return zz, yy, xx


def _sphere(center, radius) -> np.ndarray:
    zz, yy, xx = _grid()
    d2 = (zz - center[0]) ** 2 + (yy - center[1]) ** 2 + (xx - center[2]) ** 2
    return d2 <= radius * radius


def _volume_from(mask: np.ndarray, rng: np.random.Generator) -> np.ndarray:
    """Synthesise a plausible-looking source volume: bright structure on a dim,
    mildly noisy background."""
    base = rng.normal(0.15, 0.03, size=SHAPE).astype(np.float32)
    base[mask] = rng.normal(0.8, 0.05, size=int(mask.sum())).astype(np.float32)
    return np.clip(base, 0.0, 1.0)


def _scenarios() -> dict[str, Proposal]:
    rng = np.random.default_rng(0)  # deterministic
    cx = (SHAPE[0] // 2, SHAPE[1] // 2, SHAPE[2] // 2)
    out: dict[str, Proposal] = {}

    # 1. Clean liver — single plausible blob, gate should pass.
    liver = _sphere(cx, 15)  # ~14k voxels * 64 mm^3 ≈ 905 mL (in liver bounds)
    out["Clean liver (model OK)"] = Proposal(
        "Clean liver (model OK)", "liver", _volume_from(liver, rng), liver.astype(np.uint8), SPACING,
        "A single, well-formed liver segmentation of plausible size.",
    )

    # 2. Fragmented kidney — many disconnected specks, connectivity should fail.
    truth = _sphere((cx[0], cx[1], cx[2]), 9)
    holes = rng.random(SHAPE) < 0.6
    frag = (truth & ~holes).astype(np.uint8)  # punch the blob into fragments
    out["Fragmented kidney (model failure)"] = Proposal(
        "Fragmented kidney (model failure)", "kidney", _volume_from(truth, rng), frag, SPACING,
        "The network shattered the kidney into disconnected fragments.",
    )

    # 3. Clipped kidney — jammed into a corner so it is plausibly sized but
    #    heavily truncated; out-of-field gate should be the headline failure.
    clipped = _sphere((5, 5, 5), 10).astype(np.uint8)
    out["Clipped kidney (out of field)"] = Proposal(
        "Clipped kidney (out of field)", "kidney", _volume_from(clipped.astype(bool), rng), clipped, SPACING,
        "Segmentation runs off the edge of the field of view.",
    )

    # 4. Oversized 'tumor' — implausibly large, volume gate should fail.
    huge = _sphere(cx, 22).astype(np.uint8)  # ~44k voxels * 64 mm^3 ≈ 2800 mL > 1000
    out["Oversized tumor (implausible volume)"] = Proposal(
        "Oversized tumor (implausible volume)", "tumor", _volume_from(huge.astype(bool), rng), huge, SPACING,
        "A 'tumor' larger than physiologically plausible.",
    )

    # 5. Plausible lesion — small, single, in-field; gate should pass.
    lesion = _sphere(cx, 6).astype(np.uint8)  # ~900 voxels * 64 mm^3 ≈ 58 mL
    out["Plausible lesion (model OK)"] = Proposal(
        "Plausible lesion (model OK)", "lesion", _volume_from(lesion.astype(bool), rng), lesion, SPACING,
        "A small, compact lesion well within the field.",
    )
    return out


_SCENARIOS = _scenarios()


def scenario_names() -> list[str]:
    return list(_SCENARIOS.keys())


def _threshold_segment(volume: np.ndarray) -> np.ndarray:
    """A live, CPU-only 'model': Otsu-style intensity threshold (numpy only)."""
    hist, edges = np.histogram(volume.ravel(), bins=64)
    centers = (edges[:-1] + edges[1:]) / 2
    total = hist.sum()
    w = np.cumsum(hist)
    wb = np.where(w == 0, 1, w)
    mb = np.cumsum(hist * centers) / wb
    mt = (np.cumsum((hist * centers)[::-1])[::-1]) / np.where((total - w) == 0, 1, total - w)
    between = w * (total - w) * (mb - mt) ** 2
    thr = centers[int(np.argmax(between))]
    return (volume >= thr).astype(np.uint8)


def propose(name: str) -> Proposal:
    """Return the model's proposed segmentation for a scenario."""
    base = _SCENARIOS[name]
    if os.environ.get("MOCK_MODEL", "1") == "1":
        return base
    # Live mode: re-segment the source volume with the threshold model.
    mask = _threshold_segment(base.volume)
    return Proposal(base.name, base.label, base.volume, mask, base.spacing,
                    f"Live threshold segmenter (MOCK_MODEL=0) on '{base.label}' volume.")
