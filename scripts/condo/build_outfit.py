"""Blender-authored additive tank top/trousers; never edits the source avatar."""
import argparse
import json
import math
from pathlib import Path
import sys

import bpy
import bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from outfit_package import accessor, read_glb, write_outfit
from landscape import mesh_object
from knit_texture import knit_textures


def body_guide(source):
    document, binary = read_glb(source)
    body = next(m for m in document['meshes'] if m.get('name') == 'CC_Base_Body')
    positions, joints, weights, faces = [], [], [], []
    offset = 0
    for primitive in body['primitives'][1:4]:
        attrs = primitive['attributes']
        p = accessor(document, binary, attrs['POSITION'])
        positions.extend(p)
        joints.extend(accessor(document, binary, attrs['JOINTS_0']))
        w = accessor(document, binary, attrs['WEIGHTS_0'])
        weights.extend(w / w.sum(axis=1, keepdims=True))
        faces.extend(accessor(document, binary, primitive['indices']).reshape(-1, 3) + offset)
        offset += len(p)
    surface = BVHTree.FromPolygons(positions, faces, all_triangles=True)
    return positions, faces, np.asarray(joints), np.asarray(weights), surface


def make_object(name, positions, faces):
    if name in bpy.data.objects:
        raise ValueError('Garment object already exists: ' + name)
    material = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mesh_object(name, positions, faces, material, bpy.context.collection, smooth=True)
    return bpy.data.objects[name]


def trousers(positions, faces):
    obj = make_object('Condo charcoal trousers', positions, faces)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.00001)
    for _ in range(12):
        bmesh.ops.smooth_vert(bm, verts=list(bm.verts), factor=.5, use_axis_x=True,
                              use_axis_y=True, use_axis_z=True)
    bm.normal_update()
    for v in bm.verts:
        # Ease belongs to the authored garment, never to the body or its pose.
        v.co += v.normal * .014
    for height, normal in [(1.07, (0, 0, 1)), (.20, (0, 0, -1))]:
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                              dist=.000001, plane_co=(0, 0, height), plane_no=normal,
                              clear_outer=True, clear_inner=False)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(obj.data)
    bm.free()
    return obj


def fit_garment_surface(obj, guide):
    """Author clearance in garment rest geometry; never modifies body or motion."""
    surface = guide[4]
    for vertex in obj.data.vertices:
        point, normal, _, _ = surface.find_nearest(vertex.co)
        clearance = (vertex.co - point).dot(normal)
        ease = float(np.interp(vertex.co.z, [1.32, 1.40], [.028, .006]))
        if clearance < ease:
            vertex.co += normal * (ease - clearance)
    obj.data.update()


def shape_knit_front(obj):
    """Give the garment a continuous front instead of copying breast contours."""
    heights = [.99, 1.06, 1.12, 1.20, 1.28, 1.35, 1.40]
    for vertex in obj.data.vertices:
        x, y, z = vertex.co
        if y >= -.01 or z > 1.40:
            continue
        width = float(np.interp(z, heights, [.17, .16, .15, .16, .16, .16, .14]))
        depth = float(np.interp(z, heights, [.125, .12, .115, .15, .17, .12, .055]))
        front = -depth * max(0, 1 - (x / width) ** 2) ** .5
        vertex.co.y = min(y, front)


def tank_top(guide):
    """Cut a sleeveless shell from the rest surface; keep shoulder continuity."""
    obj = make_object('Condo cream knit top', *guide[:2])
    attach_source_weights(obj, guide)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.00001)
    for _ in range(12):
        bmesh.ops.smooth_vert(bm, verts=list(bm.verts), factor=.5, use_axis_x=True,
                              use_axis_y=True, use_axis_z=True)
    bm.to_mesh(obj.data)
    bm.free()
    shape_knit_front(obj)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    for _ in range(8):
        bmesh.ops.smooth_vert(bm, verts=list(bm.verts), factor=.4, use_axis_x=True,
                              use_axis_y=True, use_axis_z=True)
    bm.to_mesh(obj.data)
    bm.free()
    fit_garment_surface(obj, guide)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    # These planes cut garment openings in rest geometry, not body pose targets.
    bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                          dist=.000001, plane_co=(0, 0, .99), plane_no=(0, 0, -1),
                          clear_outer=True, clear_inner=False)
    cut_armholes(bm)
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.00002)
    bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=.00002)
    openings = garment_openings(bm)
    if len(openings) != 4:
        raise ValueError(f'Expected neck, two armholes and hem; found {openings}')
    obj['openings'] = json.dumps(openings)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bm.to_mesh(obj.data)
    bm.free()
    bpy.context.view_layer.objects.active = obj
    thickness = obj.modifiers.new('Fabric edge thickness', 'SOLIDIFY')
    thickness.thickness = .002
    thickness.offset = 1
    bpy.ops.object.modifier_apply(modifier=thickness.name)
    return obj


def cut_armholes(mesh):
    """Cut curved armhole boundaries while retaining the lower side panels."""
    for side in [-1, 1]:
        # Flatten only the garment's cutting boundary, cut, then restore it.
        for vertex in mesh.verts:
            vertex.co.x -= side * float(np.interp(vertex.co.z, [1.24, 1.30], [.26, .16]))
        bmesh.ops.bisect_plane(mesh, geom=list(mesh.verts) + list(mesh.edges) + list(mesh.faces),
                              dist=.000001, plane_co=(0, 0, 0), plane_no=(side, 0, 0),
                              clear_outer=True, clear_inner=False)
        for vertex in mesh.verts:
            vertex.co.x += side * float(np.interp(vertex.co.z, [1.24, 1.30], [.26, .16]))


def garment_openings(mesh):
    """Report connected boundary loops before adding the fabric thickness."""
    pending = {edge for edge in mesh.edges if edge.is_boundary}
    openings = []
    while pending:
        frontier, vertices = [pending.pop()], set()
        while frontier:
            edge = frontier.pop()
            vertices.update(edge.verts)
            adjacent = {e for v in edge.verts for e in v.link_edges if e in pending}
            pending.difference_update(adjacent)
            frontier.extend(adjacent)
        points = np.asarray([tuple(v.co) for v in vertices])
        if any(sum(e.is_boundary for e in v.link_edges) != 2 for v in vertices):
            raise ValueError('Garment opening is branched instead of a closed loop')
        openings.append({'vertices': len(vertices), 'min': points.min(axis=0).tolist(),
                         'max': points.max(axis=0).tolist()})
    return openings


def attach_source_weights(obj, guide):
    """Retain body-surface correspondence before changing the garment shape."""
    source_joints, source_weights = guide[2:4]
    groups = {int(joint): obj.vertex_groups.new(name=f'joint_{joint}')
              for joint in np.unique(source_joints)}
    for vertex, (joints, weights) in enumerate(zip(source_joints, source_weights)):
        for joint, weight in zip(joints, weights):
            if weight > 0:
                groups[int(joint)].add([vertex], float(weight), 'REPLACE')


def retained_weights(obj):
    """Read weights propagated by Blender through garment cuts and thickness."""
    joints, weights = [], []
    for vertex in obj.data.vertices:
        influences = [(int(obj.vertex_groups[g.group].name.removeprefix('joint_')), g.weight)
                      for g in vertex.groups if g.weight > 0]
        retained = sorted(influences, key=lambda pair: pair[1], reverse=True)[:4]
        retained += [(0, 0)] * (4 - len(retained))
        total = sum(weight for _, weight in retained)
        if total <= 0:
            raise ValueError('Garment vertex lost its source skin weights')
        joints.append([joint for joint, _ in retained])
        weights.append([weight / total for _, weight in retained])
    return joints, weights


def transfer_weights(obj, guide):
    positions, faces, source_joints, source_weights, surface = guide
    joints, weights = [], []
    for v in obj.data.vertices:
        point, _, face, _ = surface.find_nearest(v.co)
        indices = faces[face]
        coordinates = barycentric_transform(point, *(Vector(positions[i]) for i in indices),
                                             Vector((1, 0, 0)), Vector((0, 1, 0)), Vector((0, 0, 1)))
        combined = {}
        for index, strength in zip(indices, coordinates):
            for joint, weight in zip(source_joints[index], source_weights[index]):
                combined[int(joint)] = combined.get(int(joint), 0) + float(weight) * max(strength, 0)
        retained = sorted(combined.items(), key=lambda pair: pair[1], reverse=True)[:4]
        retained += [(0, 0)] * (4 - len(retained))
        total = sum(w for _, w in retained)
        joints.append([j for j, _ in retained])
        weights.append([w / total for _, w in retained])
    return joints, weights


def garment_data(obj, colour, guide):
    obj.data.calc_loop_triangles()
    joints, weights = retained_weights(obj) if obj.vertex_groups else transfer_weights(obj, guide)
    return {'name': obj.name, 'colour': colour,
            'positions': [tuple(v.co) for v in obj.data.vertices],
            'normals': [tuple(v.normal) for v in obj.data.vertices],
            'triangles': [tuple(t.vertices) for t in obj.data.loop_triangles],
            'joints': joints, 'weights': weights}


def cylindrical_knit_uv(mesh):
    """Keep knit rows aligned around the torso, with a seam at the centre back."""
    uv = mesh.uv_layers.new(name='Knit rows').data
    for polygon in mesh.polygons:
        points = [mesh.vertices[mesh.loops[loop].vertex_index].co for loop in polygon.loop_indices]
        around = [math.atan2(p.x, -p.y) / math.tau + .5 for p in points]
        crosses_seam = max(around) - min(around) > .5
        for loop, point, u in zip(polygon.loop_indices, points, around):
            if crosses_seam and u < .5:
                u += 1
            uv[loop].uv = (u * 32, (point.z - .99) * 32)
    return uv


def add_knit_uv(obj, garment):
    uv = cylindrical_knit_uv(obj.data)
    obj.data.calc_loop_triangles()
    expanded = {key: [] for key in ['positions', 'normals', 'joints', 'weights', 'uv']}
    indices, triangles = {}, []
    for triangle in obj.data.loop_triangles:
        corners = []
        for vertex, loop in zip(triangle.vertices, triangle.loops):
            coordinate = tuple(float(x) for x in uv[loop].uv)
            key = (vertex, coordinate)
            if key not in indices:
                indices[key] = len(indices)
                for field in ['positions', 'normals', 'joints', 'weights']:
                    expanded[field].append(garment[field][vertex])
                expanded['uv'].append(coordinate)
            corners.append(indices[key])
        triangles.append(corners)
    garment.update(expanded, triangles=triangles, textures=knit_textures())


def knit_blender_material(obj, garment, destination):
    material = obj.data.materials[0]
    material.use_nodes = True
    nodes, links = material.node_tree.nodes, material.node_tree.links
    shader = nodes.get('Principled BSDF')
    shader.inputs['Roughness'].default_value = .9
    tint = nodes.new('ShaderNodeMixRGB')
    tint.blend_type = 'MULTIPLY'
    tint.inputs[0].default_value = 1
    tint.inputs[2].default_value = garment['colour'] + [1]
    for channel, data in garment['textures'].items():
        path = destination.with_suffix('.knit-' + channel + '.png')
        path.write_bytes(data)
        image = bpy.data.images.load(str(path.resolve()))
        image.colorspace_settings.name = 'Non-Color' if channel == 'normal' else 'sRGB'
        image.pack()
        texture = nodes.new('ShaderNodeTexImage')
        texture.image = image
        if channel == 'base':
            links.new(texture.outputs['Color'], tint.inputs[1])
            links.new(tint.outputs[0], shader.inputs['Base Color'])
        else:
            normal = nodes.new('ShaderNodeNormalMap')
            normal.inputs['Strength'].default_value = .6
            links.new(texture.outputs['Color'], normal.inputs['Color'])
            links.new(normal.outputs[0], shader.inputs['Normal'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    outputs = [args.destination.with_suffix(suffix) for suffix in
               ['.glb', '.blend', '.json', '.knit-base.png', '.knit-normal.png']]
    if any(path.exists() for path in outputs):
        raise FileExistsError('Choose a new variant name; existing outfit files are preserved')
    bpy.ops.wm.read_factory_settings(use_empty=True)
    guide = body_guide(args.source)
    objects = [tank_top(guide), trousers(*guide[:2])]
    colours = [[.78, .72, .60], [.045, .052, .06]]
    for obj, colour in zip(objects, colours):
        obj.data.materials[0].diffuse_color = colour + [1]
    garments = [garment_data(obj, colour, guide) for obj, colour in zip(objects, colours)]
    add_knit_uv(objects[0], garments[0])
    knit_blender_material(objects[0], garments[0], args.destination)
    report = write_outfit(args.source, args.destination, garments)
    report['garments'] = [{'name': g['name'], 'vertices': len(g['positions']),
                           'triangles': len(g['triangles'])} for g in garments]
    report['top_openings_before_thickness'] = json.loads(objects[0]['openings'])
    report['limits'] = ['Static skinned clothing, no cloth simulation or body morph fitting.',
                         'Original body, skeleton, facial shapes and motion are untouched.',
                         'Visual and clipping acceptance requires rendered motion review.']
    args.destination.with_suffix('.json').write_text(json.dumps(report, indent=2) + '\n')
    bpy.ops.wm.save_as_mainfile(filepath=str(args.destination.with_suffix('.blend').resolve()))
    print('OUTFIT_EXPORTED', json.dumps(report))


if __name__ == '__main__':
    main()
