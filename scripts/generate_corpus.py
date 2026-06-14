"""Generate the synthetic DICOM QC corpus — the ground-truth eval set for the
deterministic validator in `govcore::dicom_qc`.

Every fixture is built from scratch with pydicom (no real patient data). Each
carries exactly one *intended* defect so the validator's verdict can be checked
against a known label. A `manifest.json` records, per fixture, the intended
defect, the expected overall verdict, and the specific check id that should
flag it.

Outputs:
  data/qc-corpus/<fixture>/*.dcm     # raw labeled series
  data/qc-corpus/manifest.json       # ground-truth labels
  spaces/01-dicom-qc/examples/*.zip  # the same series, zipped for the Space UI

Run:  python scripts/generate_corpus.py
"""

from __future__ import annotations

import json
import shutil
import zipfile
from pathlib import Path

import numpy as np
from pydicom.dataset import FileDataset, FileMetaDataset
from pydicom.uid import CTImageStorage, ExplicitVRLittleEndian, generate_uid

REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "data" / "qc-corpus"
EXAMPLES = REPO / "spaces" / "01-dicom-qc" / "examples"

# Deterministic UID roots so regenerating the corpus is byte-stable per fixture.
ROOT = "1.2.826.0.1.3680043.8.498"
STUDY_UID = f"{ROOT}.1.1"
IMPL_UID = f"{ROOT}.0.1"
AXIAL = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
TILTED = [1.0, 0.0, 0.0, 0.0, 0.7071, 0.7071]


def make_slice(
    path: Path,
    *,
    series_uid: str,
    instance_number: int,
    z: float,
    orientation=AXIAL,
    modality: str | None = "CT",
    include_rows_cols: bool = True,
    phi: bool = False,
) -> None:
    """Write one CT instance with controllable defects."""
    sop_uid = f"{series_uid}.{instance_number}"
    meta = FileMetaDataset()
    meta.MediaStorageSOPClassUID = CTImageStorage
    meta.MediaStorageSOPInstanceUID = sop_uid
    meta.TransferSyntaxUID = ExplicitVRLittleEndian
    meta.ImplementationClassUID = IMPL_UID

    ds = FileDataset(str(path), {}, file_meta=meta, preamble=b"\0" * 128)
    ds.SOPClassUID = CTImageStorage
    ds.SOPInstanceUID = sop_uid
    ds.SeriesInstanceUID = series_uid
    ds.StudyInstanceUID = STUDY_UID
    if modality is not None:
        ds.Modality = modality
    ds.InstanceNumber = instance_number
    ds.ImageOrientationPatient = list(orientation)
    ds.ImagePositionPatient = [0.0, 0.0, float(z)]
    ds.SliceThickness = 2.0

    if phi:
        # Deliberately leave real-looking identifiers in place.
        ds.PatientName = "Doe^Jane"
        ds.PatientID = "MRN-00990077"
        ds.PatientBirthDate = "19700101"
        ds.InstitutionName = "General Hospital"
        ds.ReferringPhysicianName = "Welby^Marcus"
    else:
        # Properly de-identified placeholders.
        ds.PatientName = "ANONYMOUS"
        ds.PatientID = "ANON"

    if include_rows_cols:
        rows = cols = 8
        ds.Rows = rows
        ds.Columns = cols
        ds.SamplesPerPixel = 1
        ds.PhotometricInterpretation = "MONOCHROME2"
        ds.BitsAllocated = 16
        ds.BitsStored = 16
        ds.HighBit = 15
        ds.PixelRepresentation = 0
        ds.PixelData = np.zeros((rows, cols), dtype=np.uint16).tobytes()

    path.parent.mkdir(parents=True, exist_ok=True)
    ds.save_as(str(path), enforce_file_format=True)


def build_clean(dir: Path) -> int:
    uid = f"{ROOT}.2.1"
    for n in range(1, 6):
        make_slice(dir / f"slice_{n:02d}.dcm", series_uid=uid, instance_number=n, z=(n - 1) * 2.0)
    return 5


def build_broken_geometry(dir: Path) -> int:
    uid = f"{ROOT}.2.2"
    for n in range(1, 6):
        orient = TILTED if n == 3 else AXIAL  # one tilted slice breaks consistency
        make_slice(dir / f"slice_{n:02d}.dcm", series_uid=uid, instance_number=n, z=(n - 1) * 2.0, orientation=orient)
    return 5


def build_missing_slices(dir: Path) -> int:
    uid = f"{ROOT}.2.3"
    present = [1, 2, 4, 5]  # instance 3 absent
    for n in present:
        make_slice(dir / f"slice_{n:02d}.dcm", series_uid=uid, instance_number=n, z=(n - 1) * 2.0)
    return len(present)


def build_residual_phi(dir: Path) -> int:
    uid = f"{ROOT}.2.4"
    for n in range(1, 6):
        make_slice(dir / f"slice_{n:02d}.dcm", series_uid=uid, instance_number=n, z=(n - 1) * 2.0, phi=True)
    return 5


def build_header_anomaly(dir: Path) -> int:
    uid = f"{ROOT}.2.5"
    for n in range(1, 6):
        # Drop Modality and Rows/Columns -> required-tag failure.
        make_slice(
            dir / f"slice_{n:02d}.dcm",
            series_uid=uid,
            instance_number=n,
            z=(n - 1) * 2.0,
            modality=None,
            include_rows_cols=False,
        )
    return 5


FIXTURES = [
    ("clean_control", build_clean, "none", "pass", None,
     "Well-formed 5-slice axial CT, de-identified."),
    ("broken_geometry", build_broken_geometry, "broken_geometry", "fail", "geometry.orientation",
     "One slice has an inconsistent ImageOrientationPatient."),
    ("missing_slices", build_missing_slices, "missing_slices", "fail", "completeness.instance_numbers",
     "Instance number 3 omitted from an otherwise contiguous series."),
    ("residual_phi", build_residual_phi, "residual_phi", "fail", "anonymization.phi",
     "Direct identifiers (PatientName/ID/DOB) left populated."),
    ("header_anomaly", build_header_anomaly, "header_anomaly", "fail", "header.required_tags",
     "Required tags Modality/Rows/Columns are absent."),
]


def main() -> None:
    if CORPUS.exists():
        shutil.rmtree(CORPUS)
    EXAMPLES.mkdir(parents=True, exist_ok=True)

    manifest = {
        "description": "Synthetic DICOM QC ground-truth corpus. No real patient data.",
        "generator": "scripts/generate_corpus.py",
        "fixtures": [],
    }

    for name, builder, defect, verdict, flag, blurb in FIXTURES:
        fixture_dir = CORPUS / name
        n_slices = builder(fixture_dir)

        zip_path = EXAMPLES / f"{name}.zip"
        if zip_path.exists():
            zip_path.unlink()
        with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as zf:
            for f in sorted(fixture_dir.glob("*.dcm")):
                zf.write(f, arcname=f"{name}/{f.name}")

        manifest["fixtures"].append({
            "name": name,
            "path": f"data/qc-corpus/{name}",
            "example_zip": f"spaces/01-dicom-qc/examples/{name}.zip",
            "slices": n_slices,
            "intended_defect": defect,
            "expected_verdict": verdict,
            "expected_flag": flag,
            "description": blurb,
        })
        print(f"  {name:16s} {n_slices} slice(s) -> verdict={verdict:4s} flag={flag}")

    (CORPUS / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"\nWrote {len(FIXTURES)} fixtures to {CORPUS}")
    print(f"Wrote {len(FIXTURES)} example zips to {EXAMPLES}")
    print(f"Manifest: {CORPUS / 'manifest.json'}")


if __name__ == "__main__":
    main()
