"""Render real govcore output from each Space into styled report-card PNGs for
the README gallery. Deterministic: runs the actual Rust core on bundled inputs,
no screenshots / browser needed.

Run:  python scripts/render_gallery.py
"""

from __future__ import annotations

import sys
from pathlib import Path
from xml.sax.saxutils import escape

import cairosvg

import govcore

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "branding" / "gallery"
sys.path.insert(0, str(REPO / "spaces" / "02-seg-governance" / "model"))
import mock_model  # noqa: E402

INK, RUST, TEAL = "#1b2230", "#d6602e", "#11868a"
SCOLOR = {"pass": "#21a366", "warn": "#e0a200", "fail": "#db4d36"}
TCOLOR = {"high": "#db4d36", "moderate": "#e0a200", "low": "#e0a200",
          "minimal": "#8a93a3", "benign-leaning": "#21a366"}
W = 1040
DETAIL_X = 470


def _trunc(s: str, n: int = 78) -> str:
    s = " ".join(s.split())
    return s if len(s) <= n else s[: n - 1] + "…"


def card(title: str, subtitle: str, badge: str, badge_color: str, rows: list[tuple[str, str, str, str]]) -> str:
    """rows = list of (status_text, status_color, name, detail)."""
    top, rh, foot = 92, 38, 54
    h = top + len(rows) * rh + foot
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {h}" '
        f'font-family="\'Segoe UI\', Helvetica, Arial, sans-serif">',
        f'<rect width="{W}" height="{h}" rx="16" fill="#ffffff" stroke="#e2e5ea" stroke-width="2"/>',
        f'<rect width="{W}" height="6" rx="3" fill="{RUST}"/>',
        f'<text x="28" y="46" font-size="23" font-weight="800" fill="{INK}">{escape(title)}</text>',
        f'<text x="28" y="72" font-size="14" fill="#5b6573">{escape(subtitle)}</text>',
        f'<rect x="{W-228}" y="28" width="200" height="40" rx="20" fill="{badge_color}"/>',
        f'<text x="{W-128}" y="54" font-size="16" font-weight="800" fill="#fff" text-anchor="middle">{escape(badge)}</text>',
    ]
    y = top + 8
    for stext, scolor, name, detail in rows:
        parts.append(f'<rect x="28" y="{y-18}" width="86" height="26" rx="6" fill="{scolor}"/>')
        parts.append(f'<text x="71" y="{y}" font-size="13" font-weight="700" fill="#fff" text-anchor="middle">{escape(stext)}</text>')
        parts.append(f'<text x="130" y="{y}" font-size="15" font-weight="700" fill="{INK}">{escape(name)}</text>')
        parts.append(f'<text x="{DETAIL_X}" y="{y}" font-size="14" fill="#5b6573">{escape(_trunc(detail))}</text>')
        y += rh
    parts.append(f'<line x1="28" y1="{h-40}" x2="{W-28}" y2="{h-40}" stroke="#eef0f3" stroke-width="1.5"/>')
    parts.append(f'<text x="28" y="{h-18}" font-size="13" fill="#8a93a3">deterministic · <tspan font-weight="700" fill="{TEAL}">govcore</tspan> (Rust core) · same input → same report</text>')
    parts.append("</svg>")
    return "\n".join(parts)


def render(svg: str, name: str) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{name}.svg").write_text(svg)
    cairosvg.svg2png(bytestring=svg.encode(), write_to=str(OUT / f"{name}.png"), scale=2, background_color="white")
    print("rendered", OUT / f"{name}.png")


def main() -> None:
    # --- Space #1: DICOM QC on the residual-PHI fixture ---
    paths = sorted(str(p) for p in (REPO / "data/qc-corpus/residual_phi").glob("*.dcm"))
    rep = govcore.qc_series(paths)
    rows = [(c["status"].upper(), SCOLOR[c["status"]], c["name"], c["detail"]) for c in rep["checks"]]
    render(card("DICOM QC — Space #1", "data/qc-corpus/residual_phi · 5-slice CT",
                rep["verdict"].upper(), SCOLOR[rep["verdict"]], rows), "01-dicom-qc")

    # --- Space #2: segmentation governance on a blocked scenario ---
    prop = mock_model.propose("Fragmented kidney (model failure)")
    rep = govcore.postcheck_segmentation(prop.label, prop.mask.reshape(-1).tolist(),
                                         list(prop.mask.shape), list(prop.spacing))
    gated = next(c for c in rep["checks"] if c["id"] == "gate.decision")["evidence"]["gated"]
    rows = [(c["status"].upper(), SCOLOR[c["status"]], c["name"], c["detail"]) for c in rep["checks"]]
    render(card("Segmentation governance — Space #2", "model proposal: 'Fragmented kidney' · 48³ volume",
                "GATED" if gated else "BLOCKED", "#21a366" if gated else "#db4d36", rows), "02-seg-governance")

    # --- Space #3: VCF scoring on the bundled example ---
    text = (REPO / "spaces/03-vcf-explorer/examples/sample.vcf").read_text()
    rep = govcore.score_vcf(text)
    rows = []
    tier_label = {"benign-leaning": "BENIGN"}
    for v in rep["variants"]:
        locus = f"{v['chrom']}:{v['pos']} {v['id']}"
        detail = f"{v['reference']}→{v['alt']} · {v['variant_class']} · score {v['total_score']:.2f}"
        label = tier_label.get(v["tier"], v["tier"].upper())
        rows.append((label, TCOLOR.get(v["tier"], "#8a93a3"), locus, detail))
    render(card("VCF explorer — Space #3", "spaces/03-vcf-explorer/examples/sample.vcf",
                f"{rep['variant_count']} SCORED", TEAL, rows), "03-vcf-explorer")


if __name__ == "__main__":
    main()
