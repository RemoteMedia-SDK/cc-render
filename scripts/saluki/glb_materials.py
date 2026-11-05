"""Add the CoD material maps of a Saluki export to a GLB made by smplx_to_glb.py.

The SMPL-X FBX only wires each material's albedo. Saluki writes every map of a
material to `<models>/<model>/_images/` and lists them in `_mat_info/<name>.txt`
by unnamed semantic id; read off the maps themselves (Valen, mw7):
  0x47  albedo RGB (its alpha is packed data, not opacity: unused)
  0x48  normal map, DXT5nm layout: X in alpha, Y in green with DirectX's
        convention (flipped here for glTF); red = gloss, blue = AO
  0x4a  opacity (hair cards, lashes, chains, lace); on the eye an iris mask
  0x4e  on skin: red = subsurface amount (low where hair is painted on the
        scalp), alpha = thickness (dark at ears, nose, lips, eyelids)
Writes normalTexture, an ORM texture (occlusion R, roughness = 1 - gloss G,
metallic B) and, where 0x4a is opacity, an RGBA base colour with alphaMode
MASK / BLEND. (CoD's own specular power curve 8192^gloss read wet in Bevy.)

--cc5-skin routes the skin materials (CoD's skin techset) through cc-render's
CC5 skin shader (src/cc_render/cc5_materials, `skin.wgsl`): it records
`extras.cc5` = RLSkin with the variables of CC5's default skin and writes the
0x4e map as its SSS / transmission maps to `<glb_out stem>.cc5maps/`.

A mask over the face (balaclava, ski mask, hockey / ballistic mask, with its clips)
is tagged `extras.face_mask`: cc-render's `CcRenderer::set_face_mask` takes it off.

Usage: python -I glb_materials.py in.glb out.glb --models <exported_files/mw7/models>
           --dirs rex_body_mp_valen_1_1,rex_head_mp_valen_1_1_high_fe [--cc5-skin]
           [--skin-gloss 0.7] [--micro-normal <CC5 skin micro normal .png>]
"""
import argparse, io, json, os, shutil, struct
import numpy as np
from PIL import Image

ap = argparse.ArgumentParser()
ap.add_argument('glb_in'); ap.add_argument('glb_out')
ap.add_argument('--models', required=True)
ap.add_argument('--dirs', required=True, help='comma-separated model folders under --models')
ap.add_argument('--cc5-skin', action='store_true', help="render skin with cc-render's CC5 skin shader")
ap.add_argument('--skin-gloss', type=float, default=1.0,
                help='gloss scale on skin (0.7 reads right on Bevy\'s StandardMaterial, without --cc5-skin)')
ap.add_argument('--micro-normal', help='tiled micro normal map for --cc5-skin (e.g. CC5\'s skin micro normal)')
args = ap.parse_args()

# CoD's skin shaders: every Saluki character's face and body skin uses the
# first; the cinematic faces of some high_fe heads (dunn, jae, park, price,
# valeria, west ...) the second, with the same 0x4e layout.
SKIN_TECHSETS = {'techset_41f783882053ba93', 'techset_397cfa4fe3a5c3b', 'techset_4e329c14662306e6'}  # 3rd: luca, ohji
# Valen (mw7): materials the maps cannot tell apart.
METAL = {'buckle_master_nickel', 'metal_silver', 'mp_western_lynch_diamond_earring_setting',
         'mp_vm_nima_necklace_base_iw9_1_1', 'mp_vm_nima_necklace_top_iw9_1_1',
         'material_59d34a67acd7d310', 'material_59d34d67acd7d829', 'material_6cd8974a524eda52',
         'material_df92f2bcd49b760', 'material_df9322bcd49bc79', 'material_358f32cda51b6908',
         'material_527c238d65d00403'}
NO_OPACITY = {'material_394f3523a7bf967e'}  # eye: 0x4a is an iris mask
BLEND = {'material_3e715c391e870b93'}  # soft-edged tear line / eye corner
STRANDS = {'material_6681b7981e3229d5', 'material_7c1b144a43404bd9',
           'head_shared_eyelashes_dark_extrathick_iw9', 'head_shared_eyelashes_dense'}
# The same by techset, for every operator: Valen's eye (on all 78 eye materials
# of the mw7 heads, `base_eye_infrared`), tear line, and hair / brow / lash cards.
EYE_TECHSET = 'techset_36a858705d2a74ea'
BLEND_TECHSETS = {'techset_4fd4ff109704cae3'}
STRAND_TECHSETS = {'techset_2043d2cd59ec5686', 'techset_1e68733b68f36e3', 'techset_6b4db11d5bdbbe45'}

# Mask detection (face_masks): what is the head itself, never a mask. Read off the 38 mw7 operator
# heads: the eye, tear line and meniscus; hair, brow, lash and beard cards; and
# CoD's generic alpha-card techset only where the face mesh carries it (lashes;
# elsewhere beanie fuzz shells).
FACE_TECHSETS = {EYE_TECHSET, 'techset_4fd4ff109704cae3', 'techset_21f8315e3ec56c3a'}
HAIR_TECHSETS = STRAND_TECHSETS | {'techset_6293e690fa2147d5', 'techset_55bb4d18d2883894',
                                   'techset_7176105580c94f24', 'techset_7c7d79f426b290b'}
CARD_TECHSETS = {'techset_2577c4a5eeba838e'}
GEAR_WORDS = ('lens', 'duct', 'radio', 'beanie', 'helmet', 'goggle', 'glass', 'balaclava', 'mask', 'hood',
              'strap', 'buckle', 'headset')
MASK_REACH = 0.016  # m from the upper lip
MASK_WORDS = ('balaclava', 'skimask', 'face_mask', 'gaiter')
MASK_HARDWARE_TECHSETS = {'techset_52bc17b9b56ecce'}  # ballistic mask shell clips, buckles, rim

# CC5's default RLSkin settings (a CC5 export's Std_Skin_Body / Std_Skin_Head),
# without its region masks: the whole material is "unmasked".
CC5_SKIN_VARS = {
    'MicroNormal Tiling': 25.0, 'MicroNormal Strength': 0.8, 'Flip MicroNormal Y': 1.0,
    'Keep Original Roughness': 0.0, 'Original Roughness Strength': 0.85, 'Micro Roughness Scale': 0.05,
    'Unmasked Roughness Scale': 0.0, 'Edge Roughness Multiplier': 0.044, '_Specular': 0.4,
    'Unmasked Scatter Scale': 2.0,
}
# the head: finer micro normal, half the body's scatter (as CC5's RLHead)
CC5_FACE_VARS = {**CC5_SKIN_VARS, 'MicroNormal Strength': 0.4, 'Unmasked Scatter Scale': 1.0}
CC5_SSS = {'Falloff': [255.0, 94.35, 76.5], 'Radius': 2.0}
# 0x4e thickness below which skin transmits light from behind
THIN = 0.12


def find_maps(name):
    for d in args.dirs.split(','):
        info = os.path.join(args.models, d, '_mat_info', name + '.txt')
        if os.path.exists(info):
            lines = open(info).read().splitlines()
            rows = dict(l.split(',') for l in lines if l.startswith('unk_semantic'))
            techset = next((l.split(':', 1)[1].strip() for l in lines if l.startswith('Techset:')), '')
            # the images sit under the material's full name: material_*, m/<name> or ma/<name>
            full = next((l.split(':', 1)[1].strip() for l in lines if l.startswith('Name:')), '')
            idir = os.path.join(args.models, d, '_images', *(full or (name if name.startswith('material_') else 'm/' + name)).split('/'))
            get = lambda s: (os.path.join(idir, rows[s] + '.png') if rows.get(s) and not rows[s].startswith('$') else None)
            return {'albedo': get('unk_semantic_47'), 'normal': get('unk_semantic_48'), 'opacity': get('unk_semantic_4a'),
                    'sss': get('unk_semantic_4e'), 'skin': techset in SKIN_TECHSETS, 'head': d.startswith('rex_head_'),
                    'techset': techset}
    return None


def png(arr):
    b = io.BytesIO(); Image.fromarray(arr).save(b, 'PNG', compress_level=6); return b.getvalue()


def u8(x):
    return (np.clip(x, 0, 1) * 255 + 0.5).astype(np.uint8)


data = open(args.glb_in, 'rb').read()
magic, ver, _ = struct.unpack_from('<III', data, 0)
jlen, jtype = struct.unpack_from('<II', data, 12)
gltf = json.loads(data[20:20 + jlen])
off = 20 + jlen
blen, btype = struct.unpack_from('<II', data, off)
bin_ = bytearray(data[off + 8: off + 8 + blen])

samplers = gltf.setdefault('samplers', [])
samplers.append({'magFilter': 9729, 'minFilter': 9987, 'wrapS': 10497, 'wrapT': 10497})
sampler = len(samplers) - 1


def add_texture(blob):
    while len(bin_) % 4: bin_.append(0)
    gltf['bufferViews'].append({'buffer': 0, 'byteOffset': len(bin_), 'byteLength': len(blob)})
    bin_.extend(blob)
    gltf['images'].append({'bufferView': len(gltf['bufferViews']) - 1, 'mimeType': 'image/png'})
    gltf['textures'].append({'sampler': sampler, 'source': len(gltf['images']) - 1})
    return len(gltf['textures']) - 1


# CC5 maps go next to the GLB: Bevy's glTF loader drops embedded textures no
# StandardMaterial references.
stem = os.path.splitext(os.path.basename(args.glb_out))[0]
maps_rel = stem + '.cc5maps'
maps_dir = os.path.join(os.path.dirname(os.path.abspath(args.glb_out)), maps_rel)
micro_rel = None
if args.cc5_skin:
    os.makedirs(maps_dir, exist_ok=True)
    if args.micro_normal:
        micro_rel = maps_rel + '/' + os.path.basename(args.micro_normal)
        shutil.copyfile(args.micro_normal, os.path.join(maps_dir, os.path.basename(args.micro_normal)))


def cc5_skin_maps(sss_png):
    """0x4e -> CC5's SSS map (mid grey = full scatter, as cc-render reads it) and
    transmission map (thin = bright), as paths relative to the GLB."""
    key = os.path.splitext(os.path.basename(sss_png))[0]
    sss_file, trans_file = f'{key}_SSSMap.png', f'{key}_TransMap.png'
    if not os.path.exists(os.path.join(maps_dir, sss_file)):
        a = np.asarray(Image.open(sss_png).convert('RGBA')).astype(np.float32) / 255
        amount, thickness = a[..., 0], a[..., 3]
        # RGB (not L): cc-render mips RGBA8 maps only
        Image.fromarray(np.repeat(u8(0.5 * amount)[..., None], 3, -1)).save(os.path.join(maps_dir, sss_file))
        # Only the thinnest parts let light through (Valen: ears 0.03, nose wings 0.04,
        # lips 0.06, nose tip 0.12; cheeks / forehead 0.4-0.9, body >= 0.25), like
        # CC5's transmission maps: black but for the ears and nose wings.
        trans = np.clip(1 - thickness / THIN, 0, 1) * amount
        Image.fromarray(np.repeat(u8(trans)[..., None], 3, -1)).save(os.path.join(maps_dir, trans_file))
    return {'SSS Map': maps_rel + '/' + sss_file, 'Transmission Map': maps_rel + '/' + trans_file}


def face_part(name, maps, face_mesh_mats):
    """Is this material the head itself (skin, eyes, mouth, hair cards), never a mask?"""
    low = name.lower()
    if any(w in low for w in GEAR_WORDS):
        return False
    if 'inside_mouth' in low or maps is None:
        return True
    ts = maps['techset']
    return (ts in SKIN_TECHSETS or ts in FACE_TECHSETS or ts in HAIR_TECHSETS
            or (ts in CARD_TECHSETS and name in face_mesh_mats))


def positions(acc_i):
    a = gltf['accessors'][acc_i]; bv = gltf['bufferViews'][a['bufferView']]
    off = bv.get('byteOffset', 0) + a.get('byteOffset', 0)
    return np.frombuffer(bytes(bin_[off:off + a['count'] * 12]), np.float32).reshape(-1, 3)


def face_masks():
    """The materials of a mask over the face, for cc-render's mask toggle (`extras.face_mask`).
    A mask covers the mouth: some vertex within MASK_REACH of the upper lip (balaclavas, ski
    masks, hockey and ballistic masks: 0.4-1.5 cm; goggles, glasses, headsets and helmet
    straps stay >= 2.2 cm away). The lip sits 3 cm below and 1 cm behind the nose tip, the
    face skin's most forward point (+Z) above its lower 40 %."""
    names = [m['name'].split('\\')[-1] for m in gltf['materials']]
    maps_of = {i: find_maps(n) for i, n in enumerate(names)}
    face_mesh_mats = {names[p['material']] for m in gltf['meshes'] if m['name'].endswith('_face') for p in m['primitives']}
    prims = [(p['material'], positions(p['attributes']['POSITION']), m['name'].endswith('_face'))
             for m in gltf['meshes'] for p in m['primitives']]
    skin = [v for i, v, face in prims if face and maps_of[i] and maps_of[i]['skin']]
    if not skin:  # no face mesh (smplx_to_glb.py without --arkit-map)
        return set()
    skin = np.concatenate(skin)
    upper = skin[skin[:, 1] > np.percentile(skin[:, 1], 40)]
    lip = upper[np.argmax(upper[:, 2])] + np.array([0, -0.03, -0.01], np.float32)
    mask = {i for i, v, _ in prims if not face_part(names[i], maps_of[i], face_mesh_mats)
            and np.linalg.norm(v - lip, axis=1).min() < MASK_REACH}
    mask |= {i for i, n in enumerate(names) if any(w in n.lower() for w in MASK_WORDS)}
    if mask:  # the mask's clips and buckles, which the game toggles with it
        mask |= {i for i, n in enumerate(names) if n.endswith('_toggle_on')
                 or (maps_of[i] and maps_of[i]['techset'] in MASK_HARDWARE_TECHSETS)}
    return mask


masks = face_masks()
for i in masks:
    gltf['materials'][i].setdefault('extras', {})['face_mask'] = True
print('FACE_MASK', sorted(gltf['materials'][i]['name'].split('\\')[-1] for i in masks))

cache = {}
for mat in gltf['materials']:
    name = mat['name'].split('\\')[-1]
    maps = find_maps(name)
    if not maps:
        print('no _mat_info for', mat['name']); continue
    pbr = mat.setdefault('pbrMetallicRoughness', {})
    metal = name in METAL
    if maps['normal']:
        gloss_scale = args.skin_gloss if maps['skin'] else 1.0
        key = (maps['normal'], metal, gloss_scale)
        if key not in cache:
            a = np.asarray(Image.open(maps['normal']).convert('RGBA')).astype(np.float32) / 255
            x = a[..., 3] * 2 - 1
            y = 1 - a[..., 1] * 2  # DirectX -> glTF
            z = np.sqrt(np.clip(1 - x * x - y * y, 0, 1))
            nrm = np.stack([x, y, z], -1) * 0.5 + 0.5
            orm = np.stack([a[..., 2], 1.0 - a[..., 0] * gloss_scale, np.full_like(x, 1.0 if metal else 0.0)], -1)
            cache[key] = (add_texture(png(u8(nrm))), add_texture(png(u8(orm))))
        n_tex, orm_tex = cache[key]
        mat['normalTexture'] = {'index': n_tex}
        mat['occlusionTexture'] = {'index': orm_tex}
        pbr['metallicRoughnessTexture'] = {'index': orm_tex}
        pbr['metallicFactor'] = 1.0
        pbr['roughnessFactor'] = 1.0
    ts = maps['techset']
    if maps['opacity'] and maps['albedo'] and name not in NO_OPACITY and ts != EYE_TECHSET:
        alb = Image.open(maps['albedo']).convert('RGB')
        op = Image.open(maps['opacity']).convert('L').resize(alb.size, Image.BILINEAR)
        rgba = np.dstack([np.asarray(alb), np.asarray(op)])
        pbr['baseColorTexture'] = {'index': add_texture(png(rgba))}
        if name in BLEND or ts in BLEND_TECHSETS:
            mat['alphaMode'] = 'BLEND'; mat.pop('alphaCutoff', None)
        else:
            strand = name in STRANDS or ts in STRAND_TECHSETS
            mat['alphaMode'] = 'MASK'; mat['alphaCutoff'] = 0.35 if strand else 0.5
    cc5 = ''
    if args.cc5_skin and maps['skin']:
        cc5_maps = cc5_skin_maps(maps['sss']) if maps['sss'] else {}
        if micro_rel:
            cc5_maps['MicroNormal'] = micro_rel
        mat.setdefault('extras', {})['cc5'] = {
            'shader': 'RLSkin', 'maps': cc5_maps, 'srgb': [],
            'vars': CC5_FACE_VARS if maps['head'] else CC5_SKIN_VARS, 'sss': CC5_SSS,
        }
        cc5 = ' cc5=RLSkin(' + ','.join(cc5_maps) + ')'
    print(f"{name[:44]:44} normal={'y' if maps['normal'] else '-'} metal={int(metal)} alpha={mat.get('alphaMode', 'OPAQUE')}{cc5}")

while len(bin_) % 4: bin_.append(0)
gltf_buf = gltf['buffers'][0]
assert gltf_buf.get('uri') is None
gltf_buf['byteLength'] = len(bin_)
js = json.dumps(gltf, separators=(',', ':')).encode()
js += b' ' * ((4 - len(js) % 4) % 4)
total = 12 + 8 + len(js) + 8 + len(bin_)
with open(args.glb_out, 'wb') as f:
    f.write(struct.pack('<III', magic, ver, total))
    f.write(struct.pack('<II', len(js), 0x4E4F534A)); f.write(js)
    f.write(struct.pack('<II', len(bin_), 0x004E4942)); f.write(bin_)
print('wrote', args.glb_out, total // 1_000_000, 'MB')

# smplx_to_glb.py --arkit-map's map follows the GLB (the renderer loads it with it)
map_in = os.path.splitext(args.glb_in)[0] + '.arkit_map.json'
if os.path.exists(map_in):
    amap = json.load(open(map_in, encoding='utf-8'))
    amap['_glb_path'] = os.path.basename(args.glb_out)
    map_out = os.path.splitext(args.glb_out)[0] + '.arkit_map.json'
    json.dump(amap, open(map_out, 'w', encoding='utf-8'), indent=2)
    print('wrote', map_out)
