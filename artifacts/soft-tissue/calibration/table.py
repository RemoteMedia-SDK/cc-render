import json, sys, pathlib
S = pathlib.Path(sys.argv[1])
names = [l.split('|')[0] for l in (S/'cases.txt').read_text().splitlines()] if (S/'cases.txt').exists() else sorted(p.stem for p in S.glob('*.json'))
print(f"{'case':16} {'fit':>5} {'down':>6} {'up':>6} {'inv':>6} {'side':>6} {'PS':>5} {'Hz':>5} {'zeta':>5} {'land':>6} {'Jmin':>5} {'shJ':>5} {'shNode':>6}")
for n in names:
    p = S/f'{n}.json'
    if not p.exists() or not p.read_text().strip():
        print(n, 'FAILED', (S/f'{n}.err').read_text()[-200:].strip()); continue
    d = json.loads(p.read_text())
    def mag(r, k):
        v = r['static'][k]
        return (sum(c*c for c in v['nipple_mm'])**.5) if 'nipple_mm' in v else float('nan')
    rows = []
    for r in d['regions']:
        L = r['landing']
        jmin = min(v['volume_ratio'][0] for v in r['static'].values() if 'volume_ratio' in v)
        rows.append((r['fit_error_mm'], mag(r,'face_down'), mag(r,'face_up'), mag(r,'upside_down'),
                     max(mag(r,'left_side_down'), mag(r,'right_side_down')), L['frequency_hz'] or 0, L['damping_ratio'] or 0,
                     -L['nipple_lowest_mm'], jmin,
                     min(r[k]['volume_ratio'][0] for k in ('shake_side','shake_bounce')) if 'shake_side' in r else float('nan'),
                     max(r[k]['peak_node_mm'] for k in ('shake_side','shake_bounce')) if 'shake_side' in r else float('nan'),
                     sum((a-b)**2 for a,b in zip(r['static']['face_down']['nipple_mm'], r['static']['face_up']['nipple_mm']))**.5))
    avg = [sum(x[i] for x in rows)/len(rows) for i in range(len(rows[0]))]
    print(f"{n:16} {avg[0]:5.2f} {avg[1]:6.1f} {avg[2]:6.1f} {avg[3]:6.1f} {avg[4]:6.1f} {avg[11]:5.0f} {avg[5]:5.2f} {avg[6]:5.2f} {avg[7]:6.1f} {min(x[8] for x in rows):5.2f} {min(x[9] for x in rows):5.2f} {max(x[10] for x in rows):6.1f}")
