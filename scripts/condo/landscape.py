"""Author compact, shared-material distant park canopies in Blender."""
import json
import math
import bpy
from foliage import detailed_tree


def build_landscape(path, collection, material):
    data = json.loads(path.read_text())
    grass = material('Mapped waterfront park lawns', (.19,.28,.115), .96)
    vertices = [(x,y,-90.42) for triangle in data['parks'] for x,y in triangle]
    faces = [(i,i+1,i+2) for i in range(0,len(vertices),3)]
    mesh_object('Toronto mapped park lawns', vertices, faces, grass, collection)

    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2, radius=1)
    template = bpy.context.object
    unit_vertices = [tuple(v.co) for v in template.data.vertices]
    unit_faces = [tuple(p.vertices) for p in template.data.polygons]
    bpy.data.objects.remove(template, do_unlink=True)
    batches = [([],[]) for _ in range(3)]
    leaf_batches = [([],[]) for _ in range(5)]
    trunk_vertices, trunk_faces = [], []
    detailed_count = 0
    for index,(x,y,height,palette) in enumerate(data['trees']):
        if math.hypot(x,y) < 600:
            detailed_tree(x,y,height,index,leaf_batches,(trunk_vertices,trunk_faces))
            detailed_count += 1
            continue
        vertices,faces = batches[palette]
        radius = height*.28
        for lobe in range(3):
            angle = index*2.39996+lobe*math.tau/3
            center = (x+math.cos(angle)*radius*.38,y+math.sin(angle)*radius*.38,-90.5+height*(.67+.05*lobe))
            base = len(vertices)
            vertices.extend((center[0]+vx*radius,center[1]+vy*radius,center[2]+vz*height*.29)
                            for vx,vy,vz in unit_vertices)
            faces.extend(tuple(base+v for v in f) for f in unit_faces)
        base = len(trunk_vertices)
        for z in [-90.5,-90.5+height*.60]:
            trunk_vertices.extend((x+math.cos(i*math.tau/5)*.22,y+math.sin(i*math.tau/5)*.22,z) for i in range(5))
        trunk_faces.extend((base+i,base+(i+1)%5,base+(i+1)%5+5,base+i+5) for i in range(5))
    colors = [(.12,.22,.065),(.19,.29,.09),(.24,.32,.12)]
    for i,(vertices,faces) in enumerate(batches):
        mesh_object('Park canopy batch '+str(i),vertices,faces,material('Park foliage '+str(i),colors[i],.92),collection,True)
    leaf_colors = [(.075,.15,.025),(.11,.21,.04),(.15,.25,.06),(.19,.29,.085),(.23,.32,.11)]
    for i,(vertices,faces) in enumerate(leaf_batches):
        mesh_object('Near park leaves '+str(i),vertices,faces,
                    material('Broadleaf foliage '+str(i),leaf_colors[i],.9),collection)
    mesh_object('Park trunks',trunk_vertices,trunk_faces,material('Park bark',(.16,.105,.06)),collection)
    print('LANDSCAPE_EXPORT',len(data['trees']),'scenic trees in mapped parks',flush=True)
    print('DETAILED_TREES',detailed_count,'leaves',detailed_count*1600,flush=True)


def mesh_object(name, vertices, faces, material, collection, smooth=False):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(vertices,[],faces)
    mesh.update()
    obj = bpy.data.objects.new(name,mesh)
    collection.objects.link(obj)
    mesh.materials.append(material)
    for polygon in mesh.polygons:
        polygon.use_smooth = smooth
