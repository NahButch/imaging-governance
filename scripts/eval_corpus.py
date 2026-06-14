"""Evaluate the deterministic validator against the labeled QC corpus.

Runs `govcore.qc_series` over every fixture in data/qc-corpus and asserts the
overall verdict and the intended-defect check id match the manifest labels.
This is the ground-truth regression for Space #1's core. Exits non-zero on any
mismatch so CI can gate on it.

Run:  python scripts/eval_corpus.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import govcore

REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "data" / "qc-corpus"


def main() -> int:
    manifest = json.loads((CORPUS / "manifest.json").read_text())
    failures = 0

    for fx in manifest["fixtures"]:
        paths = sorted(str(p) for p in (REPO / fx["path"]).glob("*.dcm"))
        report = govcore.qc_series(paths)
        verdict = report["verdict"]
        by_id = {c["id"]: c for c in report["checks"]}

        ok = verdict == fx["expected_verdict"]
        detail = f"verdict={verdict} (expected {fx['expected_verdict']})"

        flag = fx["expected_flag"]
        if flag is not None:
            flagged = by_id.get(flag, {}).get("status")
            flag_ok = flagged in ("warn", "fail")
            ok = ok and flag_ok
            detail += f", {flag}={flagged}"

        mark = "OK  " if ok else "FAIL"
        print(f"  [{mark}] {fx['name']:16s} {detail}")
        if not ok:
            failures += 1

    if failures:
        print(f"\n{failures} fixture(s) did not match their labels.")
        return 1
    print(f"\nAll {len(manifest['fixtures'])} fixtures matched their ground-truth labels.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
