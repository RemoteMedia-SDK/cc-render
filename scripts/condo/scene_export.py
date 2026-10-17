"""Shared Blender export and UV checks for authored condo assets."""
import json
import struct
import bpy
from mathutils import Vector


def ensure_surface_uv(obj):
    if obj.type != 'MESH' or obj.data.uv_layers:
        return
    uv = obj.data.uv_layers.new()
    for polygon in obj.data.polygons:
        vertical = abs(polygon.normal.z) < .5
        for index in polygon.loop_indices:
            point = obj.data.vertices[obj.data.loops[index].vertex_index].co
            horizontal = point.x if abs(polygon.normal.y) > .5 else point.y
            uv.data[index].uv = (horizontal, point.z) if vertical else (point.x, point.y)


def veneer_uv(obj):
    """Continuous board patch, with grain along each panel's longest axis."""
    ensure_surface_uv(obj)
    size = obj.dimensions
    for face in obj.data.polygons:
        normal = max(range(3), key=lambda axis: abs(face.normal[axis]))
        along = max((axis for axis in range(3) if axis != normal), key=lambda axis: size[axis])
        across = next(axis for axis in range(3) if axis not in (normal, along))
        for index in face.loop_indices:
            point = obj.data.vertices[obj.data.loops[index].vertex_index].co
            obj.data.uv_layers[0].data[index].uv = (
                .012 + .085 * (point[across] / size[across] + .5),
                .2 + (point[along] + size[along] / 2) / 1.7)


def validate_texture_uvs(path):
    raw = path.read_bytes()
    length = struct.unpack_from('<I', raw, 12)[0]
    document = json.loads(raw[20:20 + length])
    for mesh in document['meshes']:
        for primitive in mesh['primitives']:
            material = document['materials'][primitive['material']]
            texture = material.get('pbrMetallicRoughness', {}).get('baseColorTexture')
            if texture:
                uv = texture.get('texCoord', 0)
                assert uv >= 0 and 'TEXCOORD_' + str(uv) in primitive['attributes'], (mesh['name'], 'missing texture UV')
    print('GLB_PREFLIGHT_OK', path.name, len(document['meshes']), flush=True)
    return document


def export_selection(path, collections):
    bpy.ops.object.select_all(action='DESELECT')
    for collection in collections:
        for obj in collection.objects:
            ensure_surface_uv(obj)
            obj.hide_set(False)
            obj.select_set(True)
    bpy.ops.export_scene.gltf(filepath=str(path), export_format='GLB', use_selection=True,
        export_apply=True, export_extras=True, export_animations=False,
        export_cameras=False, export_lights=False)
    return validate_texture_uvs(path)


def export_environment(path):
    return export_selection(path, [c for c in bpy.data.collections if c.name[:2] in ['01', '02', '03', '06']])


def update_saved_environments(root, update):
    """Apply an asset edit to both authored packages and validate each export."""
    for suffix, export in [('', 'interior-city.glb'), ('-lightmap', 'interior-city-lightmap.glb')]:
        source = root / f'toronto-condo{suffix}.blend'
        bpy.ops.wm.open_mainfile(filepath=str(source))
        update()
        export_environment(root / export)
        bpy.context.preferences.filepaths.save_version = 0
        bpy.ops.wm.save_as_mainfile(filepath=str(source))
        print('ENVIRONMENT_UPDATE_COMPLETE', update.__name__, export, flush=True)


def grouped_solids(tag, navigation_groups):
    """Conservative static bounds from tagged geometry, never body targets."""
    bpy.context.view_layer.update()
    result = []
    groups = sorted({o.get(tag) for o in bpy.data.objects if o.get(tag)})
    assert groups, ('No tagged geometry', tag)
    for group in groups:
        objects = [o for o in bpy.data.objects if o.get(tag) == group]
        corners = [o.matrix_world @ Vector(c) for o in objects for c in o.bound_box]
        low = Vector(tuple(min(c[i] for c in corners) for i in range(3)))
        high = Vector(tuple(max(c[i] for c in corners) for i in range(3)))
        centre, half = (low + high) / 2, (high - low) / 2
        result.append({'id':group, 'position':[centre.x, centre.z, -centre.y],
                       'half_extents':[half.x, half.z, half.y],
                       'navigation_obstacle':group in navigation_groups})
    return result


def update_manifest_entries(root, key, entries, filenames=('scene.json', 'scene-lightmap.json')):
    """Replace supplied IDs in the selected authored package lists."""
    ids = {entry['id'] for entry in entries}
    for filename in filenames:
        path = root / filename
        package = json.loads(path.read_text())
        package[key] = [entry for entry in package.get(key, []) if entry['id'] not in ids] + entries
        path.write_text(json.dumps(package, indent=2) + '\n')


def update_solid_manifests(root, solids):
    update_manifest_entries(root, 'solids', solids)
