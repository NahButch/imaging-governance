"""Per-Space smoke test: import each Space's app and exercise its handler on a
bundled example, asserting the deterministic core drives a sensible result.

Loads each app.py under a unique module name so the three (all named `app.py`)
don't collide. Exits non-zero on any failure so CI can gate on it.

Run:  python scripts/space_smoke.py
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SPACES = REPO / "spaces"


def _load(path: Path, name: str):
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    # Space #1 — DICOM QC
    s1 = _load(SPACES / "01-dicom-qc" / "app.py", "space1_app")
    _, _, clean = s1.run_qc([str(s1.EXAMPLES_DIR / "clean_control.zip")])
    assert clean["verdict"] == "pass", clean
    _, _, phi = s1.run_qc([str(s1.EXAMPLES_DIR / "residual_phi.zip")])
    assert phi["verdict"] == "fail", phi
    print("  [OK] 01-dicom-qc      clean=pass, residual_phi=fail")

    # Space #2 — segmentation governance
    s2 = _load(SPACES / "02-seg-governance" / "app.py", "space2_app")
    names = s2.mock_model.scenario_names()
    gated = blocked = 0
    for name in names:
        report = s2.run(name)[4]
        if next(c for c in report["checks"] if c["id"] == "gate.decision")["evidence"]["gated"]:
            gated += 1
        else:
            blocked += 1
    assert gated >= 1 and blocked >= 1, (gated, blocked)
    print(f"  [OK] 02-seg-governance {gated} gated, {blocked} blocked across {len(names)} scenarios")

    # Space #3 — VCF explorer
    s3 = _load(SPACES / "03-vcf-explorer" / "app.py", "space3_app")
    _, _, vcf = s3.run_vcf(str(s3.EXAMPLES_DIR / "sample.vcf"), "")
    assert vcf["variant_count"] >= 1 and vcf["parse_ok"], vcf
    assert any(v["tier"] == "high" for v in vcf["variants"]), vcf
    print(f"  [OK] 03-vcf-explorer   {vcf['variant_count']} variants scored, parse_ok")

    print("\nAll Space app smoke tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
