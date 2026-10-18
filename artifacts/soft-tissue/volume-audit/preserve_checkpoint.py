"""Preserve numeric evidence; leave large raw images in their durable run folders."""
import gzip
import hashlib
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parents[3]
ARTIFACTS = ROOT / "artifacts/soft-tissue"
OUT = ARTIFACTS / "volume-audit"
RUN_NAMES = (
    "lab-boundary-live120-01", "lab-boundary-nodal-front-01",
    "lab-boundary-opt120-01", "lab-boundary-opt240-01",
    "lab-nodal-isochoric-front-01", "lab-nodal-volume-front-01",
    "lab-nodal-volume-front-02", "lab-soft-nodal-isochoric-front-01",
    "lab-soft-volume-nodes-01",
)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def preserve():
    runs = [ARTIFACTS / name for name in RUN_NAMES]
    traces = sorted(p for run in runs for p in run.glob("*.ndjson"))
    concat = sorted(p for name in ("locking-audit", "volume-audit")
                    for p in (ARTIFACTS / name).glob("*.ffconcat"))
    members = traces + concat
    archive = OUT / "checkpoint-traces.tar.gz"
    with archive.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as zipped:
            with tarfile.open(fileobj=zipped, mode="w|") as tar:
                for path in members:
                    info = tar.gettarinfo(str(path), str(path.relative_to(ROOT)))
                    info.uid = info.gid = info.mtime = 0
                    info.uname = info.gname = ""
                    with path.open("rb") as stream:
                        tar.addfile(info, stream)
    with tarfile.open(archive, "r:gz") as tar:
        for path in members:
            with tar.extractfile(str(path.relative_to(ROOT))) as stream:
                assert hashlib.file_digest(stream, "sha256").hexdigest() == digest(path)

    folders = runs + [ARTIFACTS / "locking-audit", OUT]
    evidence = sorted(p for folder in folders for p in folder.iterdir()
                      if p.is_file() and p.suffix != ".png"
                      and p.name != "checkpoint-manifest.json")
    records = [{"path": str(p.relative_to(ROOT)), "bytes": p.stat().st_size,
                "sha256": digest(p), "archived": p in members} for p in evidence]
    raw_images = [{"directory": str(run.relative_to(ROOT)),
                   "png_count": len(list(run.glob("*.png"))),
                   "png_bytes": sum(p.stat().st_size for p in run.glob("*.png"))}
                  for run in runs]
    video = OUT / "tracked-off-on-boundary.mp4"
    validation = json.loads((OUT / "boundary-video-validation.json").read_text())
    assert digest(video) == validation["video_sha256"]
    report = {
        "date": "2026-10-18", "prior_checkpoint": "33bcb3d",
        "user_visual_feedback": "Looks much much better",
        "feedback_scope": "Latest boundary-pressure comparison; not production acceptance",
        "selected_video": str(video.relative_to(ROOT)),
        "selected_asset": "assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json",
        "selected_asset_sha256": digest(ROOT / "assets/tissue/beautiful_assistant.base-boundary-nodal-volume-prototype.json"),
        "trace_archive": str(archive.relative_to(ROOT)),
        "trace_archive_verified_members": len(members),
        "files": records, "raw_images_retained_locally_not_in_git": raw_images,
        "temporary_files": "New evidence lives in this worktree; prior migration is recorded in preserved-tmp/manifest.json",
    }
    (OUT / "checkpoint-manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"verified_members": len(members), "archive_bytes": archive.stat().st_size,
                      "evidence_files": len(records), "video_sha256": digest(video)}, indent=2))


if __name__ == "__main__":
    preserve()
