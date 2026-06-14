"""Space #3 — VCF variant explorer.

Upload a VCF; the deterministic Rust core `govcore.score_vcf` parses it and
scores each variant with a transparent weighted-evidence model. Every score is
the sum of named components, each carrying its provenance — so the table is
auditable, not a black box. Python only renders; all scoring is in Rust.
"""

from __future__ import annotations

import os
from pathlib import Path

import gradio as gr

import govcore

HERE = Path(__file__).resolve().parent
EXAMPLES_DIR = HERE / "examples"

TIER_ICON = {"high": "🔴", "moderate": "🟠", "low": "🟡", "minimal": "⚪", "benign-leaning": "🟢"}


def _summary_md(report: dict) -> str:
    parse = "✅ parsed" if report["parse_ok"] else "⚠️ parse issues"
    errs = report.get("parse_errors") or []
    err_md = ("\n\n**Parse errors:**\n" + "\n".join(f"- {e}" for e in errs)) if errs else ""
    return (
        f"## 🧬 {report['variant_count']} variant(s) scored — {parse}\n\n"
        f"{report['summary']}  ·  engine: `govcore.score_vcf` (deterministic Rust)"
        f"{err_md}"
    )


def _variant_table(report: dict) -> list[list[str]]:
    rows = []
    for v in report["variants"]:
        rows.append([
            f"{TIER_ICON.get(v['tier'], '')} {v['tier']}",
            f"{v['total_score']:.2f}",
            v["chrom"],
            v["pos"],
            v["id"],
            f"{v['reference']}→{v['alt']}",
            v["variant_class"],
            v["filter"],
        ])
    return rows


def run_vcf(file_path: str | None, pasted: str):
    text = ""
    if file_path:
        text = Path(file_path).read_text()
    elif pasted and pasted.strip():
        text = pasted
    if not text.strip():
        return "### ⬆️ Upload or paste a VCF to begin.", [], {}
    report = govcore.score_vcf(text)
    return _summary_md(report), _variant_table(report), report


THESIS = """
# 🧬 VCF explorer — auditable, deterministic variant scoring

A variant caller proposes variants; a **deterministic Rust core** scores each
one with a transparent weighted-evidence model — consequence/impact, rarity
(allele frequency), clinical significance, and quality/filter. Every score is
the **sum of named components, each with its provenance**, so nothing is a black
box. The weight table is in the report (`scoring_model`). Same VCF in, same
scores out.

Upload a `.vcf`, paste one, or load the bundled example.
"""


def build_demo() -> gr.Blocks:
    with gr.Blocks(title="VCF explorer — deterministic scoring") as demo:
        gr.Markdown(THESIS)
        with gr.Row():
            with gr.Column(scale=1):
                upload = gr.File(label="VCF file", file_count="single", type="filepath", file_types=[".vcf"])
                pasted = gr.Textbox(label="…or paste VCF text", lines=6, placeholder="##fileformat=VCFv4.2 …")
                run_btn = gr.Button("Score variants", variant="primary")
                gr.Examples(examples=[[str(EXAMPLES_DIR / "sample.vcf")]], inputs=upload, label="Example VCF")
            with gr.Column(scale=2):
                banner = gr.Markdown()
                table = gr.Dataframe(
                    headers=["Tier", "Score", "Chrom", "Pos", "ID", "REF→ALT", "Class", "Filter"],
                    col_count=(8, "fixed"),
                    wrap=True,
                    label="Scored variants",
                )
                report = gr.JSON(label="Full report — per-variant component provenance + scoring_model")

        outputs = [banner, table, report]
        run_btn.click(run_vcf, inputs=[upload, pasted], outputs=outputs)
        upload.change(run_vcf, inputs=[upload, pasted], outputs=outputs)
    return demo


if __name__ == "__main__":
    build_demo().launch(
        server_name=os.environ.get("GRADIO_SERVER_NAME", "0.0.0.0"),
        server_port=int(os.environ.get("GRADIO_SERVER_PORT", "7860")),
    )
