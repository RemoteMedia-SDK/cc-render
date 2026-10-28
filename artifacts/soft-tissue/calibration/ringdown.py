"""Landing ringdown per case: nipple/centroid peaks and how long the bounce lasts."""
import json, sys, pathlib
S = pathlib.Path(sys.argv[1])
names = [l.split('|')[0] for l in (S/'cases.txt').read_text().splitlines()]
rate = 240.
print(f"{'case':16} {'region':6} {'nip lo/hi':>12} {'cen lo/hi':>12} {'tip/cen':>7} {'>2mm s':>6} {'>1mm s':>6} {'@0.5s':>6} {'@1s':>6} {'ms/step':>7}")
for n in names:
    d = json.loads((S/f'{n}.json').read_text())
    for r in d['regions']:
        L = r['landing']; nip, cen = L['nipple_trace_mm'], L['centroid_trace_mm']
        after = nip[int(.15*rate):]
        last = lambda mm: next((i for i in range(len(after)-1, -1, -1) if abs(after[i]) > mm), -1) / rate
        at = lambda s: max(abs(x) for x in after[int(s*rate):int(s*rate)+int(rate/5)])
        swing = (max(nip)-min(nip)) / (max(cen)-min(cen))
        print(f"{n:16} {r['name'][:5]:6} {min(nip):5.1f}/{max(nip):5.1f} {min(cen):5.1f}/{max(cen):5.1f} {swing:7.2f} {last(2):6.2f} {last(1):6.2f} {at(.5):6.1f} {at(1):6.1f} {r['shake_side']['ms_per_step']:7.2f}")
