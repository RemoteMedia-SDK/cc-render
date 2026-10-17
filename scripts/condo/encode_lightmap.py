"""Encode Blender's linear float bake as an explicitly linear 8-bit PNG."""
from pathlib import Path
import argparse
import hashlib
import json
import numpy as np
from PIL import Image
from denoise_lightmap import denoise

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--oidn-lib',type=Path,help='OIDN shared library with RTLightmap support')
parser.add_argument('--only',help='Encode only this image from the scene lightmap manifest')
parser.add_argument('--settings',type=Path,help='Bake settings for a partial surface refresh')
args = parser.parse_args()

root = Path(__file__).resolve().parents[2]/'scenes/toronto_condo'
package = json.loads((root/'scene-lightmap.json').read_text())
filenames = {m['image'] for m in package['lightmaps']}
if args.only and (args.only not in filenames or args.settings is None):
    parser.error('--only must name a declared lightmap and requires --settings')
settings = json.loads((args.settings or root/'sky-bake-settings.json').read_text())
ranges = {}
for filename in sorted({args.only} if args.only else filenames):
    pixels = np.load(root/Path(filename).with_suffix('.npy'))[:,:,:3]
    assert np.isfinite(pixels).all() and pixels.min() >= 0
    if args.oidn_lib: pixels = denoise(pixels,args.oidn_lib)
    scale = max(1.,float(pixels.max()))
    # Blender buffers start at the bottom; PNG rows start at the top.
    Image.fromarray(np.uint8(np.rint(np.flipud(pixels)*255/scale))).save(root/filename)
    ranges[filename] = {'minimum':float(pixels.min()),'maximum':float(pixels.max()),'encoding_scale':scale}
    for lightmap in package['lightmaps']:
        if lightmap['image'] == filename: lightmap['sky_strength'] = 2.0*scale
(root/'scene-lightmap.json').write_text(json.dumps(package,indent=2)+'\n')
report = {
    'source':'Blender Cycles normalized white world; diffuse direct and indirect, color excluded',
    **settings,
    'denoiser': {'filter':'OIDN RTLightmap',
                 'library':str(args.oidn_lib.resolve()),
                 'sha256':hashlib.sha256(args.oidn_lib.read_bytes()).hexdigest()} if args.oidn_lib else None,
    'encoding':'RGB8 linear, vertical flip from Blender pixel buffer; load with is_srgb=false',
    'ranges':ranges,
    'limitations':'Static architecture/furniture only. No avatar, sun or practical lamps are baked.'
}
if args.only:
    previous = json.loads((root/'floor-bake.json').read_text())
    previous['ranges'].update(ranges)
    previous.setdefault('surface_updates', {})[args.only] = report
    report = previous
(root/'floor-bake.json').write_text(json.dumps(report,indent=2)+'\n')
