"""Render recorded Rust cloth positions; no Blender simulation or pose repair.

blender --background --python-exit-code 1 --python this_file -- TRACE_DIR NEW_OUTPUT
The rendered mattress is the authored mesh; the Rust prototype collider is an
explicit rounded-box approximation. This is not a live renderer/avatar capture.
"""
import hashlib
import json
import argparse
from pathlib import Path
import sys

import bpy
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_bed_sheet import ROOT, SOURCE, load_mattress, sheet_mesh, studio, write_json


def mattress_clearance(mattress, sheet):
    """Finite samples against the authored mesh, not a continuous certificate."""
    evaluated = mattress.evaluated_get(bpy.context.evaluated_depsgraph_get())
    bvh = BVHTree.FromPolygons(
        [evaluated.matrix_world @ v.co for v in evaluated.data.vertices],
        [list(face.vertices) for face in evaluated.data.polygons])
    points = [sheet.matrix_world @ v.co for v in sheet.data.vertices]
    sheet.data.calc_loop_triangles()
    samples = []
    for face in sheet.data.loop_triangles:
        a, b, c = (points[i] for i in face.vertices)
        samples.extend(((a+b)*.5, (b+c)*.5, (c+a)*.5, (a+b+c)/3))
    def penetration(point):
        nearest, normal, _, distance = bvh.find_nearest(point)
        return distance if (point-nearest).dot(normal) < 0 else 0.0
    return {'scope': 'Signed nearest-surface samples on the authored mattress mesh; no repair',
            'max_vertex_penetration_m': max(map(penetration, points)),
            'max_triangle_sample_penetration_m': max(map(penetration, samples)),
            'triangle_samples': len(samples),
            'sampling': 'Each render triangle centroid and three edge midpoints'}


def bind_runtime_topology(sheet, path):
    asset = json.loads(path.read_text())
    positions = [v.co.copy() for v in sheet.data.vertices]
    assert len(positions) == len(asset['material_uv'])
    sheet.data.clear_geometry()
    sheet.data.from_pydata(positions, [], asset['triangles'])
    uv = sheet.data.uv_layers.new(name='MaterialUV')
    for loop in sheet.data.loops:
        uv.data[loop.index].uv = asset['material_uv'][loop.vertex_index]
    for face in sheet.data.polygons:
        face.use_smooth = True
    sheet.data.update()


def apply_sample(sheet, sample):
    assert len(sample['positions']) == len(sheet.data.vertices)
    for vertex, (x, y, z) in zip(sheet.data.vertices, sample['positions']):
        vertex.co = (x, -z, y)
    sheet.data.update()
    bpy.context.view_layer.update()


def audit_trace(mattress, sheet, frames):
    samples = []
    for sample in frames:
        apply_sample(sheet, sample)
        samples.append({'frame':sample['frame'], **mattress_clearance(mattress, sheet)})
    maximum = max(s['max_triangle_sample_penetration_m'] for s in samples)
    vertices = max(s['max_vertex_penetration_m'] for s in samples)
    return {'samples':samples, 'pass':max(maximum, vertices) < .001,
            'threshold_m':.001, 'max_triangle_sample_penetration_m':maximum,
            'max_vertex_penetration_m':vertices,
            'scope':'Finite stored-time and triangle samples only; not swept or continuous collision'}


def preview(trace, out, selected_frames=None, topology=None, audit_all=False):
    out.mkdir(parents=True, exist_ok=False)
    frames = json.loads((trace/'positions.json').read_text())
    report = json.loads((trace/'report.json').read_text())
    mattress = load_mattress()
    sheet = sheet_mesh()
    if topology:
        bind_runtime_topology(sheet, topology)
    scene = bpy.context.scene
    studio(scene)
    scene.render.resolution_percentage = 50
    selected = [frames[0], frames[2], frames[10], frames[-1]]
    if selected_frames is not None:
        by_frame = {sample['frame']: sample for sample in frames}
        selected = [by_frame[frame] for frame in selected_frames]
    shots = []
    for sample in selected:
        apply_sample(sheet, sample)
        filename = f"frame-{sample['frame']:04d}.png"
        scene.render.filepath = str(out/filename)
        bpy.ops.render.render(write_still=True)
        shots.append({'file':filename, 'simulation_seconds':sample['frame']/60,
                      'authored_mattress_clearance': mattress_clearance(mattress, sheet)})
    inputs = [trace/'positions.json', trace/'observations.json', trace/'report.json',
              SOURCE, Path(__file__), ROOT/'scripts/condo/check_bed_sheet.py']
    if topology:
        inputs.append(topology)
    audit = audit_trace(mattress, sheet, frames) if audit_all else None
    write_json(out/'capture.json', {'scope':__doc__, 'solver_report':report, 'screenshots':shots,
        'topology':'Explicit runtime triangles' if topology else 'Legacy Blender quads',
        'surface_audit':audit,
        'inputs':[{'path':str(p), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in inputs]})
    for filename in ['report.json','observations.json']:
        (out/filename).write_bytes((trace/filename).read_bytes())
    print('CLOTH_PREVIEW_COMPLETE', str(out), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--frames', type=int, nargs='+')
    parser.add_argument('--topology', type=Path)
    parser.add_argument('--audit-all', action='store_true')
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    preview(args.trace.resolve(), args.output.resolve(), args.frames, args.topology, args.audit_all)
