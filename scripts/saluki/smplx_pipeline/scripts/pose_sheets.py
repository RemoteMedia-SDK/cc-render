"""Composite skeleton overlays onto character renders and lay them out as contact sheets.

usage: python -I pose_sheets.py <poses_dir> <out_prefix> [close]
"""
import glob
import os
import sys

from PIL import Image, ImageDraw, ImageFont

src, out_prefix = sys.argv[1], sys.argv[2]
CLOSE = len(sys.argv) > 3 and sys.argv[3] == "close"
VIEWS = ["cfront", "cside", "cback", "ctop"] if CLOSE else ["front", "34", "side", "back"]
TILE = (300, 300) if CLOSE else (330, 385)
LABEL_W = 170
tags = sorted({os.path.basename(p).rsplit("_", 2)[0] for p in glob.glob(os.path.join(src, "*_char.png"))})
try:
    font = ImageFont.truetype("arial.ttf", 20)
except OSError:
    font = ImageFont.load_default()


def tile(tag, view):
    im = Image.open(os.path.join(src, f"{tag}_{view}_char.png")).convert("RGBA")
    sk = os.path.join(src, f"{tag}_{view}_skel.png")
    if os.path.exists(sk):
        s = Image.open(sk).convert("RGBA")
        a = s.getchannel("A").point(lambda v: int(v * 0.85))
        s.putalpha(a)
        im = Image.alpha_composite(im, s)
    return im.convert("RGB").resize(TILE, Image.LANCZOS)


SHEETS = [tags[:6], tags[6:]] if CLOSE else [tags[:5], tags[5:]]
for n, rows in enumerate(SHEETS, start=1):
    sheet = Image.new("RGB", (LABEL_W + TILE[0] * len(VIEWS), 34 + TILE[1] * len(rows)), "white")
    d = ImageDraw.Draw(sheet)
    for c, v in enumerate(VIEWS):
        d.text((LABEL_W + c * TILE[0] + 10, 6), v, fill="black", font=font)
    for r, tag in enumerate(rows):
        y = 34 + r * TILE[1]
        d.text((8, y + TILE[1] // 2 - 10), tag[3:].replace("_", " "), fill="black", font=font)
        for c, v in enumerate(VIEWS):
            sheet.paste(tile(tag, v), (LABEL_W + c * TILE[0], y))
    sheet.save(f"{out_prefix}_{n}.png")
    print(f"{out_prefix}_{n}.png", rows)
