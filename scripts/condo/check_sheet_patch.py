"""Synthetic local-clamp lift/release fixture, NOT avatar/contact admission.

Three material vertices start attached by compliant pin springs. Animated input
targets exercise that local constraint, then weights go to zero. The control
never attaches. No body motion, scene runtime integration or whole-sheet pinning.
Run via Blender --python this_file -- NEW_OUTPUT_DIR [control] [video].
"""
import hashlib
import json
import os
from pathlib import Path
import sys

import bpy
from bpy_extras.anim_utils import animdata_get_channelbag_for_assigned_slot
from mathutils import Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).parent))
import check_bed_sheet as bed

ASSETS = bed.ROOT/'scenes/toronto_condo/interaction-authoring/bedding'
RELEASE_FRAME = 122
END_FRAME = 361
LIFT = Vector((0, .05, .55))


def blender_point(runtime):
    x, y, z = runtime
    return Vector((x, -z, y))


def initial_sheet():
    report = json.loads((ASSETS/'cloth-validation.json').read_text())
    assert report['pass'], 'Initial drape must have passed its recorded fixture'
    surface = json.loads((ASSETS/'top-sheet-surface.json').read_text())
    positions = json.loads((ASSETS/'final-positions.json').read_text())
    assert len(positions) == len(surface['positions'])
    sheet = bed.sheet_mesh()
    for vertex, point in zip(sheet.data.vertices, positions):
        vertex.co = blender_point(point)
    sheet.data.update()
    sheet.shape_key_add(name='Initial observed drape')
    rest = sheet.shape_key_add(name='Flat material rest')
    for vertex, point in zip(rest.data, surface['positions']):
        vertex.co = blender_point(point)
    rest.value = 0
    return sheet, surface, rest


def select_patch(surface):
    foot = set(surface['edge_candidates']['foot'])
    candidates = [(i,t) for i,t in enumerate(surface['triangles']) if len(foot.intersection(t)) == 2]
    index, vertices = min(candidates,
        key=lambda item: abs(sum(surface['material_uv'][v][0] for v in item[1])/3 - .5))
    return {'triangle': index, 'vertices': vertices, 'barycentric': [1/3]*3,
            'material_uv': [sum(surface['material_uv'][v][a] for v in vertices)/3 for a in (0,1)],
            'selection': 'Central material foot-edge triangle; no exposure/reachability/contact claim.'}


def keyed(owner, path, samples):
    for frame, value in samples:
        setattr(owner, path, value)
        owner.keyframe_insert(data_path=path, frame=frame)
    setattr(owner, path, samples[0][1])
    linear_animation(owner.id_data)


def linear_animation(block):
    bag = animdata_get_channelbag_for_assigned_slot(block.animation_data)
    assert bag is not None, 'Expected fixture animation channels'
    for curve in bag.fcurves:
        for key in curve.keyframe_points:
            key.interpolation = 'LINEAR'


def local_clamp(sheet, rest, patch, control):
    target = sheet.shape_key_add(name='Synthetic clamp targets')
    for vertex, basis in zip(target.data, sheet.data.shape_keys.reference_key.data):
        vertex.co = basis.co
    target.slider_max = 2
    for index in patch['vertices']:
        target.data[index].co += LIFT
    keyed(target, 'value', [(1,0), (25,0), (97,1), (121,1), (169,1.8), (END_FRAME,1.8)])
    group = sheet.vertex_groups.new(name='Temporary material patch')
    group.add(patch['vertices'], .85, 'REPLACE')
    mix = sheet.modifiers.new('Synthetic clamp release', 'VERTEX_WEIGHT_MIX')
    mix.vertex_group_a = group.name
    mix.mix_set = 'A'
    mix.mix_mode = 'MUL'
    keyed(mix, 'default_weight_b', [(1,0 if control else 1),
        (RELEASE_FRAME-1,0 if control else 1), (RELEASE_FRAME,0), (END_FRAME,0)])
    cloth = bed.configure_cloth(sheet)
    cloth.name = 'Local patch cloth'
    cloth.settings.rest_shape_key = rest
    cloth.settings.vertex_group_mass = group.name
    cloth.settings.pin_stiffness = 1
    cloth.point_cache.frame_end = END_FRAME
    return cloth, target


def target_marker(centre):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=24, ring_count=12, radius=.025, location=centre)
    marker = bpy.context.object
    marker.name = 'NONCOLLIDING synthetic clamp target'
    material = bpy.data.materials.new('Diagnostic clamp orange')
    material.use_nodes = True
    material.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (1,.22,.02,1)
    marker.data.materials.append(material)
    for frame, amount in [(1,0), (25,0), (97,1), (121,1), (169,1.8), (END_FRAME,1.8)]:
        marker.location = centre + LIFT*amount
        marker.keyframe_insert(data_path='location', frame=frame)
    marker.location = centre
    linear_animation(marker)
    return marker


def patch_metrics(sheet, points, patch, target):
    start = Vector(patch['initial_centre_blender_m'])
    group = sheet.vertex_groups['Temporary material patch']
    centre = sum((points[i] for i in patch['vertices']), Vector())/3
    desired = start + LIFT*target.value
    evaluated = sheet.evaluated_get(bpy.context.evaluated_depsgraph_get())
    weights = {v.index:g.weight for v in evaluated.data.vertices for g in v.groups
               if g.group == group.index and g.weight > 1e-6}
    return {'patch_centre_blender_m': list(centre), 'target_blender_m': list(desired),
            'patch_target_error_m': (centre-desired).length,
            'patch_height_change_m': centre.z-start.z,
            'active_constraint_vertices': sorted(weights), 'active_weights': list(weights.values())}


def verdict(samples, patch, control):
    held = [s for s in samples if 105 <= s['frame'] < RELEASE_FRAME]
    released = [s for s in samples if s['frame'] >= RELEASE_FRAME]
    common = {
        'stretch': max(s['max_edge_stretch_ratio'] for s in samples) < 1.10,
        'mattress_vertex_clearance': max(s['max_vertex_penetration_m'] for s in samples) < .003,
        'solver_success': all(s.get('solver_status') == ['SUCCESS'] for s in samples[1:]),
        'settled': max(s['max_vertex_speed_m_s'] for s in samples[-12:]) < .05,
        'supported': min(s['mattress_support_fraction'] for s in samples[-12:]) > .4,
        'no_constraints_after_release': all(not s['active_constraint_vertices'] for s in released),
    }
    if control:
        common['never_attached'] = all(not s['active_constraint_vertices'] for s in samples)
        common['no_target_following'] = max(s['patch_height_change_m'] for s in held) < .1
    else:
        attached = [s for s in samples if s['frame'] < RELEASE_FRAME]
        common['only_local_patch'] = all(s['active_constraint_vertices'] == sorted(patch['vertices']) for s in attached)
        common['lifted'] = min(s['patch_height_change_m'] for s in held) > .4
        common['bounded_grip_slip'] = max(s['patch_target_error_m'] for s in attached) < .03
        common['fell_after_release'] = samples[120]['patch_centre_blender_m'][2] - samples[-1]['patch_centre_blender_m'][2] > .2
    return common


def render_frames(scene, out, frame, video):
    if frame in (1,121,169,END_FRAME):
        scene.render.filepath = str(out/f'patch-{frame:04d}.png')
        bpy.ops.render.render(write_still=True)
    if video and (frame-1) % 3 == 0:
        scene.render.resolution_percentage = 50
        scene.cycles.samples = 4
        scene.render.filepath = str(out/'video_frames'/f'frame-{frame:04d}.png')
        bpy.ops.render.render(write_still=True)
        scene.render.resolution_percentage = 100
        scene.cycles.samples = 16


def run(out, control, video=False):
    out.mkdir(parents=True, exist_ok=False)
    if video:
        (out/'video_frames').mkdir()
    inputs = [ASSETS/name for name in ('cloth-validation.json','top-sheet-surface.json','final-positions.json')]
    inputs += [Path(__file__), Path(bed.__file__), bed.SOURCE]
    sources = [{'file':str(p.relative_to(bed.ROOT)), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in inputs]
    mattress = bed.load_mattress()
    sheet, surface, rest = initial_sheet()
    patch = select_patch(surface)
    start = sum((sheet.data.vertices[i].co for i in patch['vertices']), Vector())/3
    patch['initial_centre_blender_m'] = list(start)
    cloth, target = local_clamp(sheet, rest, patch, control)
    target_marker(start)
    scene = bpy.context.scene
    scene.frame_end = END_FRAME
    scene.render.fps = 24
    bed.studio(scene)
    scene.frame_set(1)
    bpy.context.view_layer.update()
    evaluated = mattress.evaluated_get(bpy.context.evaluated_depsgraph_get())
    bvh = BVHTree.FromPolygons([evaluated.matrix_world @ v.co for v in evaluated.data.vertices],
                              [list(f.vertices) for f in evaluated.data.polygons])
    scope = 'Synthetic initial local-clamp constraint; no contact acquisition, fingers, avatar, or runtime integration.'
    bed.write_json(out/'fixture.json', {'scope':scope, 'control':control,'patch':patch,
        'initial_conditions':'Recorded drape with zero initial velocity; flat material rest lengths retained.',
        'initially_attached':not control,'release_frame':RELEASE_FRAME,'lift_blender_m':list(LIFT),
        'input_hashes':sources,'script_source':Path(__file__).read_text(),
        'base_script_source':Path(bed.__file__).read_text()})
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'local-patch-fixture.blend'))
    samples, previous = [], None
    for frame in range(1,END_FRAME+1):
        scene.frame_set(frame)
        previous, metrics = bed.observe(sheet,bvh,surface,previous,cloth.name)
        if frame == 1:
            error = max((v-p.co).length for v,p in zip(previous,sheet.data.shape_keys.reference_key.data))
            bed.write_json(out/'initial-state-check.json', {'max_position_error_m':error,'pass':error<1e-6})
            assert error < 1e-6, 'Initial cloth differs from the recorded drape'
        row = {'frame':frame, **metrics, **patch_metrics(sheet,previous,patch,target)}
        samples.append(row)
        render_frames(scene,out,frame,video)
        if frame % 48 == 0:
            print('PATCH_FRAME',frame,row,flush=True)
    criteria = verdict(samples,patch,control)
    report = {'scope':scope,'control':control,'criteria':criteria,'pass':all(criteria.values()),
        'samples':samples,'limits':['No actual-contact admission or collision with the orange marker.',
        'Three nodal pin springs approximate a material patch; not a barycentric single-contact solver.',
        'Vertex-only mattress clearance, no continuous triangle or self-intersection certificate.']}
    bed.write_json(out/'patch-validation.json',report)
    bed.write_json(out/'final-positions.json',[[v.x,v.z,-v.y] for v in previous])
    print('PATCH_RESULT',criteria,flush=True)
    assert report['pass'], 'Local patch fixture failed; evidence retained'


if __name__ == '__main__':
    args = sys.argv[sys.argv.index('--')+1:]
    assert set(args[1:]) <= {'control','video'}, 'Unknown fixture option'
    run(Path(args[0]).resolve(), 'control' in args[1:], 'video' in args[1:])
    os._exit(0)
