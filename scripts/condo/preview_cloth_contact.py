"""Render recorded cloth contact tests; no Blender cloth solve or repair.

blender --background --python-exit-code 1 --python this_file -- TRACE_JSON NEW_OUTPUT
The trace identifies whether colours mark connected halves or separate fabric layers.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from tabletop_props import box, material
from preview_drawer_trace import blender_point as point, configure_animation, encode_animation
from preview_cloth_trace import animation_rate


def setup(trace):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 720
    scene.world = bpy.data.worlds.new('Physics studio')
    scene.world.use_nodes = True
    scene.world.node_tree.nodes['Background'].inputs['Color'].default_value = (.14, .16, .19, 1)
    scene.world.node_tree.nodes['Background'].inputs['Strength'].default_value = .6
    box('Fixed floor', (.5, -.5, -.05), (2.2, 1.8, .1),
        material('Stone support', (.24, .27, .29), .85), scene.collection)
    mesh = bpy.data.meshes.new('Recorded solver topology')
    mesh.from_pydata([point(p) for p in trace['frames'][0]['positions']], [], trace['triangles'])
    sheet = bpy.data.objects.new(trace.get('fixture', 'Connected folded sheet'), mesh)
    scene.collection.objects.link(sheet)
    mesh.materials.append(material('Fabric A', (.8, .72, .57), .9))
    mesh.materials.append(material('Fabric B', (.16, .4, .58), .9))
    for face, index in zip(mesh.polygons, trace['face_materials']):
        face.material_index = index
        face.use_smooth = True
    camera = bpy.data.objects.new('Physics camera', bpy.data.cameras.new('Physics camera'))
    scene.collection.objects.link(camera)
    camera.location = (2.1, -2.25, 1.05)
    camera.rotation_euler = (Vector((.5, -.5, .1)) - camera.location).to_track_quat('-Z', 'Y').to_euler()
    camera.data.type = 'ORTHO'
    camera.data.ortho_scale = 1.9
    scene.camera = camera
    for name, location, energy, size in [('Key', (0, -1, 3), 500, 3), ('Rim', (1.5, .5, 2), 350, 2)]:
        light = bpy.data.objects.new(name, bpy.data.lights.new(name, 'AREA'))
        scene.collection.objects.link(light)
        light.location = location
        light.data.energy = energy
        light.data.shape = 'DISK'
        light.data.size = size
        light.rotation_euler = (Vector((.5, -.5, 0)) - light.location).to_track_quat('-Z', 'Y').to_euler()
    return scene, sheet


def preview(source, output):
    trace = json.loads(source.read_text())
    fps = animation_rate(trace['frames'])
    output.mkdir(parents=True, exist_ok=False)
    scene, sheet = setup(trace)
    configure_animation(scene, output)
    shots = []
    for ordinal, sample in enumerate(trace['frames']):
        for vertex, position in zip(sheet.data.vertices, sample['positions']):
            vertex.co = point(position)
        sheet.data.update()
        bpy.context.view_layer.update()
        observed = [list(v.co) for v in sheet.data.vertices]
        expected = [point(p) for p in sample['positions']]
        error = max(abs(a-b) for p, q in zip(observed, expected) for a, b in zip(p, q))
        if error > 1e-7:
            raise ValueError(f'Render mesh differs from recorded solver positions: {error}')
        filename = f'frames/{ordinal:04d}.png'
        scene.render.filepath = str(output / filename)
        bpy.ops.render.render(write_still=True)
        shots.append({'file': filename, 'simulation_seconds': sample['frame']/60,
                      'max_mesh_coordinate_error_m': error})
    encode_animation(output, shots, fps)
    helpers = [Path(__file__), Path(__file__).with_name('tabletop_props.py'),
               Path(__file__).with_name('preview_drawer_trace.py'), Path(__file__).with_name('preview_cloth_trace.py')]
    report = {'scope': __doc__, 'screenshots': shots, 'fps': fps,
              'measurements': {k:v for k,v in trace.items() if k not in ('frames','triangles','face_materials')},
              'inputs': [{'path':str(p), 'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in [source, *helpers]]}
    (output/'capture.json').write_text(json.dumps(report, indent=2)+'\n')
    print('CLOTH_CONTACT_PREVIEW_COMPLETE', output, flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    preview(args.source.resolve(), args.output.resolve())
