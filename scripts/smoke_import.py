"""Smoke test: prove the PyO3 boundary works end-to-end.

Imports the compiled `govcore` module and exercises every exposed callable,
asserting each returns a well-formed structured report dict. Exits non-zero on
any failure so CI can gate on it.
"""

import sys

import govcore


def _check_report(obj: dict, module: str) -> None:
    assert isinstance(obj, dict), f"{module}: expected dict, got {type(obj)}"
    for key in ("module", "verdict", "summary", "counts", "checks"):
        assert key in obj, f"{module}: missing key {key!r}"
    assert obj["module"] == module, f"{module}: wrong module field {obj['module']!r}"
    assert obj["verdict"] in ("pass", "warn", "fail"), obj["verdict"]
    assert isinstance(obj["checks"], list) and obj["checks"], f"{module}: no checks"
    for c in obj["checks"]:
        for key in ("id", "name", "status", "detail", "evidence"):
            assert key in c, f"{module}: check missing {key!r}"


def main() -> int:
    print(f"govcore.__version__ = {govcore.__version__}")

    pong = govcore.ping("hello")
    assert pong == {
        "ok": True,
        "engine": "govcore",
        "version": govcore.__version__,
        "echo": "hello",
    }, pong
    print(f"ping -> {pong}")

    _check_report(govcore.qc_series([]), "dicom_qc")
    _check_report(
        govcore.postcheck_segmentation("liver", [0, 1, 1, 0], [1, 2, 2], [1.0, 1.0, 1.0]),
        "seg_postcheck",
    )

    # vcf_score has its own per-variant shape (not the check-list Report).
    vcf = govcore.score_vcf(
        "##fileformat=VCFv4.2\n#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n"
        "1\t100\trs1\tA\tT\t80\tPASS\tIMPACT=HIGH;AF=0.00005;CLNSIG=Pathogenic;DP=60\n"
    )
    assert vcf["module"] == "vcf_score", vcf
    for key in ("summary", "parse_ok", "variant_count", "variants", "scoring_model"):
        assert key in vcf, f"vcf_score missing key {key!r}"
    assert vcf["variant_count"] == 1, vcf
    v = vcf["variants"][0]
    assert v["tier"] == "high", v
    assert v["components"], "no score provenance"
    print(f"score_vcf -> 1 variant, tier={v['tier']}, score={v['total_score']}")

    print("OK: govcore PyO3 boundary verified.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
