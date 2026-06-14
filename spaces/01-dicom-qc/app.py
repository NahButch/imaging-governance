"""Space #1 — DICOM QC governance.

Thin Gradio glue over the deterministic Rust core `govcore`. The app only
unpacks the upload and renders the result; **all** validation logic lives in
`govcore.qc_series` (Rust). Model proposes, deterministic engine disposes.
"""

from __future__ import annotations

import os
import tempfile
import zipfile
from pathlib import Path

import gradio as gr

import govcore

STATUS_ICON = {"pass": "✅", "warn": "⚠️", "fail": "❌"}
STATUS_WORD = {"pass": "PASS", "warn": "WARN", "fail": "FAIL"}

HERE = Path(__file__).resolve().parent
EXAMPLES_DIR = HERE / "examples"


def _collect_dicom_paths(files: list[str]) -> list[str]:
    """Expand the upload (zip(s) and/or loose files) into instance file paths.

    Zips are extracted to a temp dir; every extracted regular file is handed to
    the engine, which is itself the judge of what is and isn't valid DICOM.
    """
    paths: list[str] = []
    for f in files or []:
        p = Path(f)
        if p.suffix.lower() == ".zip":
            dest = Path(tempfile.mkdtemp(prefix="dicomqc_"))
            with zipfile.ZipFile(p) as zf:
                zf.extractall(dest)
            paths.extend(str(q) for q in sorted(dest.rglob("*")) if q.is_file())
        elif p.is_file():
            paths.append(str(p))
    return paths


def _verdict_banner(report: dict) -> str:
    verdict = report["verdict"]
    icon = STATUS_ICON.get(verdict, "")
    c = report["counts"]
    return (
        f"## {icon} Overall verdict: **{STATUS_WORD.get(verdict, verdict.upper())}**\n\n"
        f"{report['summary']}  \n"
        f"`{c['pass']} pass · {c['warn']} warn · {c['fail']} fail`  ·  "
        f"engine: `govcore` (deterministic Rust core)"
    )


def _checks_table(report: dict) -> list[list[str]]:
    rows = []
    for c in report["checks"]:
        rows.append([
            STATUS_ICON.get(c["status"], "") + " " + STATUS_WORD.get(c["status"], c["status"]),
            c["name"],
            c["detail"],
        ])
    return rows


def run_qc(files: list[str]):
    """Gradio handler: returns (banner_md, checks_table, full_report_json)."""
    paths = _collect_dicom_paths(files)
    if not paths:
        return (
            "### ⬆️ Upload a DICOM series (a `.zip` or multiple `.dcm` files) to begin.",
            [],
            {},
        )
    report = govcore.qc_series(paths)
    return _verdict_banner(report), _checks_table(report), report


def _example_list() -> list[list[str]]:
    return [[str(z)] for z in sorted(EXAMPLES_DIR.glob("*.zip"))]


THESIS = """
# 🩻 DICOM QC — deterministic governance

A probabilistic pipeline (a scanner, an upstream model, a de-identifier) proposes
an imaging series; a **deterministic Rust core** disposes — validating geometry,
completeness, header conformance and residual PHI with pure, ordered,
unit-tested logic. The verdict below is reproducible byte-for-byte: same series
in, same report out.

Upload a series, or try a bundled example with a **known, labeled defect**.
"""


def build_demo() -> gr.Blocks:
    with gr.Blocks(title="DICOM QC — deterministic governance") as demo:
        gr.Markdown(THESIS)
        with gr.Row():
            with gr.Column(scale=1):
                files = gr.File(
                    label="DICOM series (.zip or multiple .dcm files)",
                    file_count="multiple",
                    type="filepath",
                )
                run_btn = gr.Button("Run QC", variant="primary")
                gr.Examples(
                    examples=_example_list(),
                    inputs=files,
                    label="Labeled example series (one clean, four defective)",
                )
            with gr.Column(scale=2):
                banner = gr.Markdown()
                table = gr.Dataframe(
                    headers=["Status", "Check", "Detail"],
                    col_count=(3, "fixed"),
                    wrap=True,
                    label="Deterministic checks",
                )
                evidence = gr.JSON(label="Full structured report (expandable evidence)")

        run_btn.click(run_qc, inputs=files, outputs=[banner, table, evidence])
        files.change(run_qc, inputs=files, outputs=[banner, table, evidence])
    return demo


if __name__ == "__main__":
    build_demo().launch(
        server_name=os.environ.get("GRADIO_SERVER_NAME", "0.0.0.0"),
        server_port=int(os.environ.get("GRADIO_SERVER_PORT", "7860")),
    )
