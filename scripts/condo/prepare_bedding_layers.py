"""Author independent fitted-sheet, top-sheet and duvet surfaces for the condo.

Run in Blender. The fitted shell approximates sewn corners without an elastic hem;
these are prototype material dimensions, not calibrated fabric measurements.
"""
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import sys
import struct

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_bed_sheet import load_mattress, SOURCE
from prepare_cloth_package import ROOT, SCENE


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def headboard_clearance(scene):
    """Move only the three headboard nodes and their matching solid by 55 mm."""
    source = SCENE/scene['geometry']
    raw = source.read_bytes()
    magic, version, size, length, kind = struct.unpack_from('<4sIIII', raw)
    assert (magic,version,size,kind) == (b'glTF',2,len(raw),0x4E4F534A)
    document = json.loads(raw[20:20+length])
    changed = []
    for index,node in enumerate(document['nodes']):
        if node.get('extras',{}).get('bedroom_group') != 'bed_headboard':
            continue
        assert index in document['scenes'][0]['nodes'] and 'matrix' not in node
        before = node['translation'][:]
        node['translation'][2] -= .055
        changed.append({'node':node['name'],'before':before,'after':node['translation'][:]})
    assert len(changed) == 3
    encoded = json.dumps(document,separators=(',',':')).encode()
    encoded += b' ' * (-len(encoded)%4)
    tail = raw[20+length:]
    target = SCENE/'interior-bedding-clearance.glb'
    target.write_bytes(struct.pack('<4sIIII',b'glTF',2,20+len(encoded)+len(tail),len(encoded),0x4E4F534A)+encoded+tail)
    next(s for s in scene['solids'] if s['id']=='bed_headboard')['position'][2] -= .055
    scene['geometry'] = target.name
    return {'source_sha256':digest(source),'result_sha256':digest(target),
            'unchanged_binary_sha256':hashlib.sha256(tail).hexdigest(),'nodes':changed,
            'limits':'Existing lightmap texels are retained; the small placement change is not rebaked.'}


def coordinates(half, radius):
    inner = half - radius
    count = math.ceil(2 * inner / .10)
    middle = [-inner + 2 * inner * i / count for i in range(count + 1)]
    ends = [inner + radius * i / 4 for i in range(1, 5)]
    return [-v for v in reversed(ends)] + middle + ends


def fitted_surface(solid):
    centre = Vector(solid['position'])
    half = Vector(solid['half_extents'])
    radius = solid['border_radius_m']
    inner = half - Vector((radius,) * 3)
    axes = [coordinates(half.x, radius), [-.08, -.04, 0., inner.y] +
            [inner.y + radius * i / 4 for i in range(1, 5)], coordinates(half.z, radius)]
    positions, uv, triangles, vertices = [], [], [], {}

    def vertex(key):
        if key in vertices:
            return vertices[key]
        p = Vector([axes[i][key[i]] for i in range(3)])
        core = Vector([max(-inner[i], min(inner[i], p[i])) for i in range(3)])
        local = core + (p-core).normalized() * (radius + .004)
        index = len(positions)
        vertices[key] = index
        positions.append(list(centre + local))
        drop = half.y - p.y
        uv.append([.5 + p.x / (2*half.x) * (1 + drop/half.x),
                   .5 + p.z / (2*half.z) * (1 + drop/half.z)])
        return index

    # A welded top and four sides, open at the lower hem.
    for axis, side in [(1, -1), (0, 0), (0, -1), (2, 0), (2, -1)]:
        a, b = [i for i in range(3) if i != axis]
        for row in range(len(axes[b])-1):
            for col in range(len(axes[a])-1):
                corners = []
                for da, db in [(0,0), (1,0), (1,1), (0,1)]:
                    key = [0,0,0]
                    key[axis] = 0 if side == 0 else len(axes[axis])-1
                    key[a], key[b] = col+da, row+db
                    corners.append(vertex(tuple(key)))
                x,y,z,w = corners
                triangles.extend([[x,y,z], [x,z,w]])
    for face in triangles:
        a,b,c = [Vector(positions[i]) for i in face]
        if (b-a).cross(c-a).dot((a+b+c)/3-centre) < 0:
            face[1],face[2] = face[2],face[1]
    return {'schema_version':1, 'rest_positions':positions, 'initial_positions':positions,
            'triangles':triangles, 'material_uv':uv}


def duvet_surface():
    columns, rows = 22, 24
    points, uv, triangles = [], [], []
    for row in range(rows+1):
        for col in range(columns+1):
            u, v = col/columns, row/rows
            points.append([3.04 + 2.26*u, .94, -2.04 - 1.94*v])
            uv.append([u,v])
            if row < rows and col < columns:
                a = row*(columns+1)+col
                triangles.extend([[a,a+1,a+columns+2], [a,a+columns+2,a+columns+1]])
    return {'schema_version':1,'rest_positions':points,'initial_positions':points,
            'triangles':triangles,'material_uv':uv}


def author_mesh(cloth, asset):
    mesh = bpy.data.meshes.new(cloth['id'])
    mesh.from_pydata([(x,-z,y) for x,y,z in asset['initial_positions']], [], asset['triangles'])
    mesh.update()
    layer = mesh.uv_layers.new(name='MaterialUV')
    for loop in mesh.loops:
        layer.data[loop.index].uv = asset['material_uv'][loop.vertex_index]
    for face in mesh.polygons:
        face.use_smooth = True
    obj = bpy.data.objects.new(cloth['id'], mesh)
    bpy.context.collection.objects.link(obj)
    mat = bpy.data.materials.new(cloth['id'] + ' fabric')
    mat.diffuse_color = (*cloth['color_srgb'], 1)
    mesh.materials.append(mat)
    obj['runtime_surface'] = cloth['surface']
    obj['runtime_sha256'] = cloth['sha256']


def build():
    source_hash = digest(SOURCE)
    load_mattress()
    package_path = SCENE/'scene-entry-door.json'
    scene = json.loads(package_path.read_text())
    headboard = headboard_clearance(scene)
    original = scene['cloths'][0]
    mattress = next(s for s in scene['solids'] if s['id'] == 'bed_mattress')
    folder = SCENE/'interaction-authoring/bedding'
    definitions = [
        ('bed_fitted_sheet', fitted_surface(mattress), .45, .001, [.89,.86,.79]),
        ('bed_top_sheet', json.loads((SCENE/original['surface']).read_text()), .8, .001, [.83,.80,.72]),
        ('bed_duvet', duvet_surface(), 1.8, .012, [.30,.40,.33]),
    ]
    cloths = []
    for name, asset, mass, thickness, color in definitions:
        target = folder/f'{name}.json'
        target.write_text(json.dumps(asset, separators=(',',':'))+'\n')
        cloth = copy.deepcopy(original)
        cloth.update(id=name, surface=str(target.relative_to(SCENE)), sha256=digest(target),
                     color_srgb=color, replaces_nodes=original['replaces_nodes'] if name=='bed_duvet' else [])
        cloth['parameters'].update(total_mass_kg=mass, thickness_m=thickness)
        cloths.append(cloth)
        author_mesh(cloth, asset)
    scene['cloths'] = cloths
    scene['id'] += '-layered-bedding'
    target = SCENE/'scene-bedding.json'
    target.write_text(json.dumps(scene, indent=2)+'\n')
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(folder/'layered-bedding.blend'))
    report = {'source_blend_sha256':source_hash, 'source_unchanged':source_hash==digest(SOURCE),
              'base_scene_sha256':digest(package_path), 'scene_sha256':digest(target), 'headboard':headboard,
              'script_sha256':digest(Path(__file__)), 'fitted_hem':'Open edge; no elastic pre-tension',
              'fitted_initial_clearance_m':.004,
              'layers':[{'id':c['id'],'sha256':c['sha256'],'parameters':c['parameters'],
                         'vertices':len(a['initial_positions']),'triangles':len(a['triangles'])}
                        for c,(_,a,*_) in zip(cloths,definitions)],
              'limits':['Sewn-shape approximation without an elastic hem; uncalibrated fabric.',
                        'Duvet is one thick midsurface, not volumetric fill or two stitched shells.',
                        'Authored initial placements require runtime settling and validation.']}
    (folder/'layered-bedding-authoring.json').write_text(json.dumps(report,indent=2)+'\n')
    assert report['source_unchanged']
    print('LAYERED_BEDDING_AUTHORED', target, flush=True)


if __name__ == '__main__':
    build()
    os._exit(0)
