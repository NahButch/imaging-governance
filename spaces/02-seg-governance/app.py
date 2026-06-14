"""Space #2 — segmentation governance.

A tiny CPU 'model' proposes a segmentation mask; the deterministic Rust core
`govcore.postcheck_segmentation` disposes — gating it through or blocking it.
The UI shows the raw model output and the governance verdict side by side. All
plausibility logic lives in Rust; Python only renders.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

import gradio as gr
import numpy as np
from PIL import Image

import govcore

sys.path.insert(0, str(Path(__file__).resolve().parent / "model"))
import mock_model  # noqa: E402

STATUS_ICON = {"pass": "✅", "warn": "⚠️", "fail": "❌"}
STATUS_WORD = {"pass": "PASS", "warn": "WARN", "fail": "FAIL"}


def _overlay(volume: np.ndarray, mask: np.ndarray, size: int = 288) -> Image.Image:
    """Mid-axial slice with the proposed mask painted red, upscaled for display."""
    z = volume.shape[0] // 2
    g = (np.clip(volume[z], 0.0, 1.0) * 255).astype(np.uint8)
    rgb = np.stack([g, g, g], axis=-1)
    rgb[mask[z] > 0] = [255, 40, 40]
    return Image.fromarray(rgb, "RGB").resize((size, size), Image.NEAREST)


def _gate_banner(report: dict) -> str:
    gate = next(c for c in report["checks"] if c["id"] == "gate.decision")
    gated = gate["evidence"]["gated"]
    head = "🟢 GATED THROUGH" if gated else "🔴 BLOCKED"
    c = report["counts"]
    return (
        f"## {head}\n\n"
        f"{report['summary']}  \n"
        f"`{c['pass']} pass · {c['warn']} warn · {c['fail']} fail`  ·  "
        f"deterministic gate in `govcore` (Rust)"
    )


def _checks_table(report: dict) -> list[list[str]]:
    return [
        [STATUS_ICON.get(c["status"], "") + " " + STATUS_WORD.get(c["status"], c["status"]), c["name"], c["detail"]]
        for c in report["checks"]
    ]


def run(name: str):
    prop = mock_model.propose(name)
    mask = prop.mask
    shape = list(mask.shape)
    spacing = list(prop.spacing)

    report = govcore.postcheck_segmentation(
        prop.label, mask.reshape(-1).astype(np.uint8).tolist(), shape, spacing
    )

    voxel_vol = spacing[0] * spacing[1] * spacing[2]
    fg = int((mask > 0).sum())
    raw_md = (
        f"### 🧠 Raw model output\n"
        f"_{prop.note}_\n\n"
        f"- label proposed: **{prop.label}**\n"
        f"- foreground voxels: **{fg:,}**\n"
        f"- claimed volume: **{fg * voxel_vol / 1000:.1f} mL**\n"
        f"- shape: `{shape}` · spacing: `{spacing}` mm\n\n"
        f"The model asserts this mask with no self-check — governance decides if it is usable."
    )
    return _overlay(prop.volume, mask), raw_md, _gate_banner(report), _checks_table(report), report


THESIS = """
# 🫁 Segmentation governance — model proposes, engine disposes

A tiny CPU segmentation model proposes a mask; a **deterministic Rust core**
runs plausibility gates over it — volume bounds for the labeled structure,
connectivity/fragment sanity, out-of-field clipping, label consistency — and
emits a binary **gate decision**. Bad model output is blocked before it can be
used downstream. Pick a scenario; some are deliberate model failures.
"""


def build_demo() -> gr.Blocks:
    with gr.Blocks(title="Segmentation governance") as demo:
        gr.Markdown(THESIS)
        names = mock_model.scenario_names()
        with gr.Row():
            scenario = gr.Dropdown(choices=names, value=names[0], label="Scenario (model proposal)")
            run_btn = gr.Button("Segment + govern", variant="primary")
        with gr.Row():
            with gr.Column():
                img = gr.Image(label="Proposed mask (red) over mid-slice", type="pil")
                raw = gr.Markdown()
            with gr.Column():
                banner = gr.Markdown()
                table = gr.Dataframe(
                    headers=["Status", "Gate", "Detail"], col_count=(3, "fixed"), wrap=True,
                    label="Deterministic plausibility gates",
                )
                evidence = gr.JSON(label="Full structured report")

        outputs = [img, raw, banner, table, evidence]
        run_btn.click(run, inputs=scenario, outputs=outputs)
        scenario.change(run, inputs=scenario, outputs=outputs)
    return demo


if __name__ == "__main__":
    build_demo().launch(
        server_name=os.environ.get("GRADIO_SERVER_NAME", "0.0.0.0"),
        server_port=int(os.environ.get("GRADIO_SERVER_PORT", "7860")),
    )
