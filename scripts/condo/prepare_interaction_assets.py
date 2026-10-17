"""Read Blender furniture into an articulation inventory and a cloth surface.

Authoring evidence only: does not modify the scene, enable interactions, or
prescribe avatar motion. Run with Blender's --background --python interface.
"""
import hashlib
import json
import os
from pathlib import Path
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).parent))
from bedroom_furniture import DUVET_GRID, ROOT

OUT = ROOT / 'interaction-authoring'
TARGETS = {
    'fridge': ('hinged', ('Kitchen detail fridge ',)),
    'kitchen_drawers': ('sliding', ('Kitchen detail drawer ', 'Kitchen detail base ')),
    'upper_cabinets': ('hinged', ('Kitchen detail upper ',)),
    'island_storage': ('mixed', ('Kitchen detail island ',)),
    'oven': ('hinged', ('Kitchen detail oven ',)),
    'entry_door': ('hinged', ('Workspace detail entry door', 'Workspace detail entry transom')),
    'entry_cabinet': ('hinged', ('Workspace detail entry cabinet', 'Workspace detail cabinet handle')),
    'desk_drawer': ('sliding', ('Workspace detail drawer ',)),
    'bedside_drawers': ('sliding', ('Bedroom detail bedside ',)),
    'vanity_drawers': ('sliding', ('Bathroom detail vanity ',)),
    'bed_cover': ('deformable', ('Bedroom detail folded duvet',)),
}


def runtime(point):
    return [float(point.x), float(point.z), float(-point.y)]


def describe(obj):
    corners = [Vector(runtime(obj.matrix_world @ Vector(c))) for c in obj.bound_box]
    low = [min(c[i] for c in corners) for i in range(3)]
    high = [max(c[i] for c in corners) for i in range(3)]
    return {'object':obj.name, 'mesh':obj.data.name, 'parent':obj.parent.name if obj.parent else None,
            'aabb_min':low, 'aabb_max':high, 'vertices':len(obj.data.vertices),
            'faces':len(obj.data.polygons), 'blender_rigid_body':obj.rigid_body is not None,
            'groups':{k:obj[k] for k in ('kitchen_group','bedroom_group','workspace_group','bathroom_group') if k in obj}}


def inventory():
    result = {}
    for name, (mechanism, prefixes) in TARGETS.items():
        objects = sorted((o for o in bpy.data.objects if o.type == 'MESH' and o.name.startswith(prefixes)), key=lambda o:o.name)
        assert objects, ('Expected authored target is missing', name)
        result[name] = {'intended_mechanism':mechanism, 'status':'static_visual_parts',
                        'parts':[describe(o) for o in objects]}
    return {'schema_version':1, 'coordinates':'runtime metres: X east, Y up, Z south',
            'targets':result, 'missing_assets':['separate top sheet', 'bathroom door leaf', 'bedroom door leaf', 'balcony sliding door leaf'],
            'limits':['Inventory is not a runtime articulation manifest.',
                      'Hinge pivots, travel limits, cavities, collision decomposition and dynamic grasps are not configured.']}


def cloth_surface(obj):
    columns, rows = DUVET_GRID
    count = (columns + 1) * (rows + 1)
    mesh = obj.data
    assert len(mesh.vertices) == 2 * count, 'Expected two corresponding solidified grid layers'
    positions, separations = [], []
    for i in range(count):
        top = obj.matrix_world @ mesh.vertices[i].co
        bottom = obj.matrix_world @ mesh.vertices[i + count].co
        separations.append((top - bottom).length)
        positions.append(runtime((top + bottom) * .5))
    assert .02 < min(separations) <= max(separations) < .04, 'Layer pairing no longer matches duvet thickness'
    faces = [list(f.vertices) for f in mesh.polygons if all(v < count for v in f.vertices)]
    assert len(faces) == columns * rows and all(len(f) == 4 for f in faces)
    triangles = [t for a,b,c,d in faces for t in ([a,b,c],[a,c,d])]
    incidence = {}
    for triangle in triangles:
        a,b,c = (Vector(positions[i]) for i in triangle)
        assert (b-a).cross(c-a).length > 1e-10, 'Degenerate cloth triangle'
        for a,b in zip(triangle,triangle[1:]+triangle[:1]):
            edge = tuple(sorted((a,b)))
            incidence[edge] = incidence.get(edge,0) + 1
    assert all(n in (1,2) for n in incidence.values()), 'Nonmanifold cloth surface'
    boundary = [list(edge) for edge,n in incidence.items() if n == 1]
    assert len(boundary) == 2 * (columns + rows)
    assert count - len(incidence) + len(triangles) == 1, 'Expected a single rectangular surface'
    edges = sorted(incidence)
    lengths = [(Vector(positions[b])-Vector(positions[a])).length for a,b in edges]
    return {'schema_version':1, 'id':'bed_cover', 'source_object':obj.name, 'source_mesh':mesh.name,
            'coordinates':'runtime metres: X east, Y up, Z south',
            'positions':positions, 'triangles':triangles, 'edges':[list(e) for e in edges],
            'rest_edge_lengths_m':lengths, 'boundary_edges':boundary,
            'material_uv':[[c/columns,r/rows] for r in range(rows+1) for c in range(columns+1)],
            'source_vertex_pairs':[[i,i+count] for i in range(count)],
            'edge_candidates':{'foot':list(range(columns+1)),
                'head':list(range(rows*(columns+1),count)),
                'west':[r*(columns+1) for r in range(rows+1)],
                'east':[r*(columns+1)+columns for r in range(rows+1)]},
            'pinned_vertices':[], 'solver':None, 'material_parameters':None,
            'limits':['Extracted midsurface of existing duvet, not a separately authored top sheet.',
                      'Edge candidates require current visibility, contact and reachability checks before use.',
                      'No cloth simulation, render-vertex binding, hand attachment or avatar manipulation is enabled.']}


def export():
    OUT.mkdir(parents=True,exist_ok=True)
    source = ROOT / 'toronto-condo.blend'
    bpy.ops.wm.open_mainfile(filepath=str(source))
    data = inventory()
    data['source'] = {'file':source.name,'sha256':hashlib.sha256(source.read_bytes()).hexdigest()}
    cloth = cloth_surface(bpy.data.objects['Bedroom detail folded duvet'])
    cloth['source'] = data['source']
    for name,payload in [('inventory.json',data),('bed-cover-surface.json',cloth)]:
        (OUT/name).write_text(json.dumps(payload,indent=2)+'\n')
    summary = {'targets':len(data['targets']), 'parts':sum(len(t['parts']) for t in data['targets'].values()),
               'cloth_vertices':len(cloth['positions']), 'cloth_triangles':len(cloth['triangles']),
               'cloth_boundary_edges':len(cloth['boundary_edges'])}
    (OUT/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print('INTERACTION_AUTHORING_PREPARED',summary,flush=True)


if __name__ == '__main__':
    export()
    os._exit(0)
