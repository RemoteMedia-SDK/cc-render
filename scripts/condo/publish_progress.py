#!/usr/bin/env python3
"""Archive a render iteration and rebuild the offline progress gallery."""
import argparse
from datetime import datetime, timezone
import hashlib
from html import escape
import json
from pathlib import Path
import re
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[2]
GALLERY = ROOT / "artifacts/toronto-condo"


def file_record(path, base):
    return {"path": path.relative_to(base).as_posix(), "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def render_entry(entry):
    def paragraphs(items):
        return "".join(f"<li>{escape(item)}</li>" for item in items)

    shots = "".join(
        f'<a href="{escape(shot["path"])}" target="_blank" rel="noopener">'
        f'<img loading="lazy" src="{escape(shot["path"])}" '
        f'alt="{escape(entry["title"])} — screenshot {i + 1}">'
        f'<span>Screenshot {i + 1} · full resolution ↗</span></a>'
        for i, shot in enumerate(entry["images"]))
    video = ""
    if entry.get("video"):
        video = (f'<video controls preload="metadata" poster="{entry["images"][0]["path"]}">'
                 f'<source src="{entry["video"]["path"]}" type="video/mp4"></video>')
    metrics = ""
    if entry.get("metrics"):
        metrics = (f'<details><summary>Capture metrics and evidence limits</summary>'
                   f'<pre>{escape(json.dumps(entry["metrics"], indent=2))}</pre></details>')
    evidence = " ".join(f'<a href="{escape(f["path"])}">{escape(Path(f["path"]).name)}</a>'
                        for f in entry.get("evidence", []))
    return f'''<article id="{entry['id']}">
      <div class="heading"><span class="badge">{escape(entry['status'])}</span>
      <time datetime="{entry['captured_at']}">{entry['captured_at'].replace('T', ' ')}</time></div>
      <h2><a href="#{entry['id']}">{escape(entry['title'])}</a></h2>
      <p class="medium">{escape(entry['medium'])}</p>
      {video}<div class="shots">{shots}</div>
      <div class="notes"><section><h3>Progress</h3><ul>{paragraphs(entry['progress'])}</ul></section>
      <section><h3>Still to improve</h3><ul>{paragraphs(entry['remaining'])}</ul></section></div>
      {metrics}<a class="metadata" href="{entry['id']}/entry.json">Entry metadata + file hashes</a>
      <p class="metadata">{evidence}</p>
    </article>'''


def render_gallery(entries):
    ordered = sorted(entries, key=lambda e: e["captured_at"], reverse=True)
    contents = "\n".join(render_entry(e) for e in ordered)
    count = sum(len(e["images"]) for e in ordered)
    return f'''<!doctype html><html lang="en"><head><meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Toronto condo · progress gallery</title><style>
    :root{{color-scheme:dark;font:16px/1.55 system-ui,sans-serif;background:#141b1c;color:#eef0e8}}
    *{{box-sizing:border-box}}body{{max-width:1440px;margin:auto;padding:32px}}
    header{{padding:30px 0 24px}}h1{{font-size:clamp(30px,4vw,52px);line-height:1.1;margin:14px 0}}
    h2{{font-size:25px;margin:12px 0 2px}}h3{{font-size:15px;color:#e0c69f}}
    p{{max-width:850px}}a{{color:#cde3df}}h2 a{{color:inherit;text-decoration:none}}
    .eyebrow{{color:#e0c69f;letter-spacing:.12em;text-transform:uppercase;font-size:12px}}
    .intro,.medium,time,.metadata{{color:#b5c1bc}}.medium{{margin:0 0 20px}}
    article{{background:#202929;border:1px solid #394442;border-radius:16px;padding:24px;margin:26px 0}}
    .heading{{display:flex;gap:16px;align-items:center;flex-wrap:wrap;font-size:13px}}
    .badge{{background:#493f2d;color:#f2d8a9;border-radius:20px;padding:4px 12px}}
    .shots{{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:12px}}
    .shots a{{text-decoration:none;background:#141b1c;border-radius:8px;overflow:hidden}}
    img{{width:100%;display:block;aspect-ratio:16/9;object-fit:contain}}
    .shots span{{display:block;padding:10px 12px;font-size:13px}}
    video{{display:block;width:100%;max-height:700px;background:#101414;margin:16px 0;border-radius:8px}}
    .notes{{display:grid;grid-template-columns:1fr 1fr;gap:28px}}ul{{padding-left:20px}}
    li{{margin:7px 0}}details{{background:#182020;padding:12px 16px;border-radius:8px;margin:16px 0}}
    summary{{cursor:pointer}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font-size:13px}}
    .metadata{{font-size:13px}}footer{{color:#b5c1bc;padding:20px 0}}
    @media(max-width:650px){{body{{padding:16px}}article{{padding:16px}}.notes{{grid-template-columns:1fr;gap:0}}}}
    </style></head><body><header><div class="eyebrow">Scene development · warm modern</div>
    <h1>A home above the waterfront.</h1>
    <p class="intro">Toronto condo progress: oak, stone, plants and soft lighting, with Lake Ontario,
    city traffic and changing weather beyond the windows. Latest captures appear first.</p>
    <p>{len(entries)} iterations · {count} screenshots · <a href="progress.json">Progress manifest</a>
    · <a href="../../docs/design/toronto-high-rise-condo.md">Scene plan</a></p>
    <p class="intro">Early prototypes below. Each entry keeps its original screenshots and recording;
    capture timings are not GPU benchmarks. Open any screenshot for the full-resolution image.
    Refresh this page after a new capture is published.</p></header>{contents}
    <footer>Local project gallery · no external assets or services required.</footer></body></html>'''


def publish(args):
    source = args.source.resolve()
    images = [source] if source.is_file() else sorted(source.glob("*.png"))
    if (not images and args.status != "Failed") or any(p.suffix.lower() != ".png" for p in images):
        raise ValueError("Source must be a PNG or a capture directory containing PNG screenshots")
    dated_files = images or list(source.glob("*.log"))
    if not dated_files:
        raise ValueError("A failed capture must contain its original log")
    stamp = datetime.fromtimestamp(min(p.stat().st_mtime for p in dated_files), timezone.utc)
    slug = re.sub(r"[^a-z0-9]+", "-", args.title.lower()).strip("-")[:60] or "capture"
    entry_id = stamp.strftime("%Y%m%dT%H%M%SZ") + "-" + slug
    GALLERY.mkdir(parents=True, exist_ok=True)
    target = GALLERY / entry_id
    if target.exists():
        raise ValueError(f"Iteration already archived: {target}; existing captures are never overwritten")
    manifest = GALLERY / "progress.json"
    entries = json.loads(manifest.read_text())["entries"] if manifest.exists() else []
    # Stage the complete entry before exposing it in the manifest.
    with tempfile.TemporaryDirectory(dir=GALLERY, prefix=".capture-") as temporary:
        staging = Path(temporary) / entry_id
        staging.mkdir()
        for path in images:
            shutil.copy2(path, staging / path.name)
        entry = {"id": entry_id, "title": args.title, "status": args.status,
                 "captured_at": stamp.isoformat(timespec="seconds"), "medium": args.medium,
                 "progress": args.progress, "remaining": args.remaining,
                 "images": [file_record(staging / p.name, Path(temporary)) for p in images]}
        if source.is_dir():
            for filename, key in [("view.mp4", "video"), ("capture.json", "report")]:
                path = source / filename
                if path.exists():
                    shutil.copy2(path, staging / filename)
                    entry[key] = file_record(staging / filename, Path(temporary))
            if (source / "capture.json").exists():
                entry["metrics"] = json.loads((source / "capture.json").read_text())
            entry["evidence"] = []
            for path in sorted(source.iterdir()):
                if path.suffix not in (".json", ".ndjson", ".log") or path.name == "capture.json":
                    continue
                shutil.copy2(path, staging / path.name)
                entry["evidence"].append(file_record(staging / path.name, Path(temporary)))
        (staging / "entry.json").write_text(json.dumps(entry, indent=2) + "\n")
        staging.rename(target)
    entries.append(entry)
    manifest.write_text(json.dumps({"schema_version": 1, "entries": entries}, indent=2) + "\n")
    (GALLERY / "index.html").write_text(render_gallery(entries))
    print(GALLERY / "index.html")
    print(f"Archived {len(images)} screenshots as {entry_id}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--title", required=True)
    parser.add_argument("--medium", default="Bevy runtime · beautiful_assistant · 1920 × 1080")
    parser.add_argument("--status", default="Prototype", choices=["Concept", "Prototype", "Review", "Accepted", "Failed"])
    parser.add_argument("--progress", action="append", required=True)
    parser.add_argument("--remaining", action="append", required=True)
    publish(parser.parse_args())
