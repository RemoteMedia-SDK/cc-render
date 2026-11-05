"""Montage expression renders: pairs (operator | CC5) in a grid. usage: python -I expr_montage.py <dir> <out.png>"""
import os, sys
from PIL import Image, ImageDraw, ImageFont
d, out = sys.argv[1], sys.argv[2]
names = []
for f in sorted(os.listdir(d), key=lambda f: os.path.getmtime(os.path.join(d, f))):
    if f.startswith("expr_") and f.endswith("_op.png"):
        names.append(f[5:-7])
tw, th = 260, 300
cols = 4
rows = (len(names) + cols - 1) // cols
sheet = Image.new("RGB", (cols * tw * 2, rows * (th + 26)), "white")
dr = ImageDraw.Draw(sheet)
try:
    font = ImageFont.truetype("arial.ttf", 18)
except OSError:
    font = ImageFont.load_default()
for i, n in enumerate(names):
    r, c = divmod(i, cols)
    x, y = c * tw * 2, r * (th + 26)
    dr.text((x + 6, y + 3), n, fill="black", font=font)
    for j, side in enumerate(("op", "cc")):
        sheet.paste(Image.open(os.path.join(d, f"expr_{n}_{side}.png")).resize((tw, th)), (x + j * tw, y + 26))
sheet.save(out)
print(out, names)
