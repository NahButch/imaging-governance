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
    _check_report(govcore.score_vcf("##fileformat=VCFv4.2\n"), "vcf_score")

    print("OK: govcore PyO3 boundary verified.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
