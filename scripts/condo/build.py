import bpy
import math
import json
import sys
from mathutils import Vector
from pathlib import Path

out = Path(__file__).resolve().parents[2] / 'scenes/toronto_condo'
out.mkdir(parents=True, exist_ok=True)
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.unit_settings.system = 'METRIC'
scene.unit_settings.scale_length = 1
scene['purpose'] = 'Warm modern Toronto condo. City massing from City of Toronto 2025; fictional interior.'
scene['orientation'] = 'Blender X east, Y north, Z up; condo finished floor Z=0.'

def material(name, rgb, roughness=0.65):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*rgb, 1)
    m.use_nodes = True
    p = m.node_tree.nodes.get('Principled BSDF')
    p.inputs['Base Color'].default_value = (*rgb, 1)
    p.inputs['Roughness'].default_value = roughness
    return m

oak = material('Warm oak', (.44, .27, .13))
stone = material('Pale limestone', (.72, .69, .59))
plaster = material('Warm plaster', (.82, .80, .73))
cream = material('Cream upholstery', (.77, .72, .61))
green = material('Muted foliage', (.16, .28, .16))
clay = material('Clay planter', (.42, .24, .17))
charcoal = material('Dark bronze', (.055, .07, .07), .35)
sage = material('Sage textile', (.32, .43, .38))
tile = material('Bathroom tile', (.48, .55, .53))
paper = material('Plan text', (.15, .23, .24))
glass = material('Glazing placeholder', (.45, .69, .72), .15)
glass.diffuse_color = (.67,.81,.84,.13)
glass.node_tree.nodes['Principled BSDF'].inputs['Alpha'].default_value = .13
glass.surface_render_method = 'DITHERED'
glass.node_tree.nodes['Principled BSDF'].inputs['Metallic'].default_value = 0
solids = []

def collection(name):
    c = bpy.data.collections.new(name)
    scene.collection.children.link(c)
    return c

shell = collection('01 Architecture - cutaway study')
furniture = collection('02 Furniture - proxy geometry')
plants = collection('03 Plants - proxy geometry')
annotations = collection('04 Plan annotations - not runtime assets')
fullshell = collection('05 Full height shell - hidden for review')
fullshell.hide_render = True
fullshell.hide_viewport = True

def move_to(o, c):
    for old in list(o.users_collection):
        old.objects.unlink(o)
    c.objects.link(o)

def box(name, pos, size, mat, c=furniture, bevel=.035):
    bpy.ops.mesh.primitive_cube_add(size=1, location=pos)
    o = bpy.context.object
    o.name = name
    o.dimensions = size
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    o.data.materials.append(mat)
    move_to(o, c)
    if bevel:
        mod = o.modifiers.new('Small edge radius', 'BEVEL')
        mod.width = bevel
        mod.segments = 4
        o.modifiers.new('Weighted normals', 'WEIGHTED_NORMAL')
    o['scene_id'] = o.name.lower().replace(' ', '_').replace('.', '_')
    physical = c == shell and not any(word in name for word in ['seam','mullion','sill','rail','post'])
    physical = physical or name in ['Kitchen cabinetry','Island cabinet','Sofa base','Bed base','Vanity','Entry storage','Coffee table']
    if physical:
        obstacle = not any(word in name.lower() for word in ['floor','slab','ceiling','overhang','threshold'])
        solids.append({'id':o['scene_id'], 'position':[pos[0],pos[2],-pos[1]],
            'half_extents':[size[0]/2,size[2]/2,size[1]/2], 'navigation_obstacle':obstacle,
            'approach':[0,0,size[1]/2+.65] if obstacle else None})
    return o

def cylinder(name, pos, radius, depth, mat, c=furniture):
    bpy.ops.mesh.primitive_cylinder_add(vertices=32, radius=radius, depth=depth, location=pos)
    o = bpy.context.object
    o.name = name
    o.data.materials.append(mat)
    move_to(o,c)
    return o

def label(text, x, y, size=.24):
    curve=bpy.data.curves.new(text, 'FONT')
    curve.body=text
    curve.align_x='CENTER'
    curve.size=size
    curve.extrude=.001
    ob=bpy.data.objects.new(text,curve)
    annotations.objects.link(ob)
    ob.location=(x,y,.07)
    curve.materials.append(paper)

def plant(x,y,r=.33,h=1.3):
    cylinder('Terracotta planter', (x,y,.23),r,.46,clay,plants)
    cylinder('Planter rim', (x,y,.45),r*1.04,.045,clay,plants)
    cylinder('Potting soil', (x,y,.455),r*.93,.015,charcoal,plants)
    cylinder('Plant trunk', (x,y,(h+.44)/2),.014,h-.44,oak,plants)
    vertices=[];faces=[]
    for i in range(26):
        angle=i*2.39996
        z=.58+(h-.45)*i/26
        length=r*(.85+.25*math.sin(i*3.7))
        origin=Vector((x,y,z))
        along=Vector((math.cos(angle),math.sin(angle),.20))
        across=Vector((-math.sin(angle),math.cos(angle),0))
        base=len(vertices)
        for j in range(7):
            t=j/6
            center=origin+along*(.09+length*t)+Vector((0,0,.055*math.sin(t*math.pi)))
            width=length*.31*math.sin(t*math.pi)**.7
            for side in [-1,0,1]:
                point=center+across*width*side+Vector((0,0,-.025*abs(side)*math.sin(t*math.pi)))
                vertices.append(tuple(point))
        for j in range(6):
            for k in range(2):
                q=base+j*3+k;faces.append((q,q+3,q+4,q+1))
    mesh=bpy.data.meshes.new('Broadleaf foliage');mesh.from_pydata(vertices,[],faces);mesh.update()
    ob=bpy.data.objects.new('Broadleaf foliage',mesh);plants.objects.link(ob);mesh.materials.append(green)
    for poly in mesh.polygons:poly.use_smooth=True

floor=box('Interior floor 12 x 10 m',(0,0,-.12),(12,10,.24),oak,shell)
for uv in floor.data.uv_layers.active.data: uv.uv*=8
for x in range(24):
    box('Oak plank seam',(-5.75+x*.5,0,.004),(.004,9.95,.003),oak,shell,0)
box('Balcony slab 12 x 2.4 m',(0,-6.2,-.16),(12,2.4,.28),stone,shell)
box('North wall west',(-3.45,5,1.45),(5.1,.15,2.9),plaster,shell)
box('North wall east',(3.1,5,1.45),(5.8,.15,2.9),plaster,shell)
box('West wall',(-6,0,1.45),(.15,10,2.9),plaster,shell)
box('East window sill',(6,0,.16),(.15,10,.32),stone,shell)
box('Bathroom floor',(-4.4,3.5,.01),(3.05,2.85,.02),tile,shell)
box('Bathroom east wall',(-2.8,3.5,1.45),(.14,3,2.9),plaster,shell)
box('Bathroom south wall A',(-5.2,2,1.45),(1.6,.14,2.9),plaster,shell)
box('Bathroom south wall B',(-2.98,2,1.45),(.36,.14,2.9),plaster,shell)
box('Bedroom west wall',(2,3.1,1.45),(.14,3.8,2.9),plaster,shell)
box('Bedroom south wall',(4.7,1.2,1.45),(2.6,.14,2.9),plaster,shell)
box('Bedroom door jamb',(2.12,1.2,1.45),(.24,.14,2.9),plaster,shell)
box('Ceiling',(0,0,2.98),(12,10,.16),plaster,shell)
box('Balcony overhang',(0,-6.2,2.98),(12,2.4,.16),stone,shell)
box('North full wall west',(-3.45,5,1.45),(5.1,.15,2.9),plaster,fullshell)
box('North full wall east',(3.1,5,1.45),(5.8,.15,2.9),plaster,fullshell)
box('West full wall',(-6,0,1.45),(.15,10,2.9),plaster,fullshell)
box('Ceiling',(0,0,2.98),(12,10,.16),plaster,fullshell)

# South glazing is represented by a sill and marker mullions in this cutaway.
box('South glazing sill',(0,-5,.06),(12,.1,.12),charcoal,shell)
for x in [-6,-4,-2,2,4,6]:
    box('Glazing mullion',(x,-5,1.45),(.045,.08,2.9),charcoal,shell,.01)
for x in [-5,-3,3,5]:
    box('South glass pane',(x,-5,1.45),(1.94,.015,2.84),glass,shell,.002)
for y in [-4,-2,0,2,4]:
    box('East glass pane',(6,y,1.6),(.015,1.95,2.55),glass,shell,.002)
    box('East mullion',(6,y-1,1.45),(.07,.04,2.9),charcoal,shell,.008)
box('Open balcony threshold',(0,-5,.02),(2,.3,.04),stone,shell)
for x in [-6,-4,-2,0,2,4,6]:
    box('Guard post',(x,-7.4,.55),(.035,.035,1.1),charcoal,shell,.005)
box('Guard top rail',(0,-7.4,1.1),(12,.04,.04),charcoal,shell,.008)
for x in [-5,-3,-1,1,3,5]:
    box('Balcony glass guard',(x,-7.4,.55),(1.96,.018,1.05),glass,shell,.002)
for x in [-6,6]:
    box('Side guard top rail',(x,-6.2,1.1),(.04,2.4,.04),charcoal,shell,.008)
    box('Balcony side glass',(x,-6.2,.55),(.018,2.4,1.05),glass,shell,.002)

# Kitchen and dining.
box('Kitchen cabinetry',(-5.55,.2,.45),(.8,3.25,.9),oak)
box('Kitchen stone worktop',(-5.55,.2,.93),(.85,3.3,.065),stone)
box('Sink',(-5.55,.5,.97),(.52,.65,.025),charcoal)
box('Island cabinet',(-3.55,.0,.45),(1,2.5,.9),oak)
box('Island stone worktop',(-3.55,0,.94),(1.15,2.65,.07),stone)
cylinder('Dining pedestal',(-3.7,-3.2,.36),.25,.72,oak)
cylinder('Dining top',(-3.7,-3.2,.76),.8,.08,stone)
for dx,dy in [(-1,0),(1,0),(0,-1),(0,1)]:
    cx,cy=-3.7+dx,-3.2+dy
    box('Dining chair upholstered seat',(cx,cy,.47),(.48,.48,.12),cream,bevel=.055)
    back=box('Dining chair curved back',(cx+dx*.21,cy+dy*.21,.75),(.48,.09,.44),cream,bevel=.04)
    back.rotation_euler.z=math.atan2(-dx,dy)
    for a in [-.18,.18]:
        for b in [-.18,.18]:cylinder('Dining chair oak leg',(cx+a,cy+b,.23),.022,.46,oak)

# Inward-facing living seating keeps the lake behind the sofa in the hero view.
box('Living rug',(2.8,-1.6,.025),(4.8,4.1,.025),cream)
box('Sofa base',(2.8,-1.1,.31),(3.1,.98,.42),sage,bevel=.10)
for x in [1.5,4.1]:
    for y in [-.78,-1.44]: cylinder('Sofa oak foot',(x,y,.09),.055,.18,oak)
box('Sofa back',(2.8,-.71,.67),(3.1,.23,.66),sage,bevel=.10)
for x in [1.35,4.25]:
    box('Sofa arm',(x,-1.1,.58),(.27,.95,.38),sage,bevel=.10)
for x in [1.85,2.8,3.75]:
    box('Sofa cushion',(x,-1.17,.56),(.90,.72,.18),cream,bevel=.065)
box('Coffee table',(2.9,-.4,.37),(1.4,.75,.12),stone,bevel=.06)
for x in [2.45,3.35]:cylinder('Coffee table oak base',(x,-.4,.18),.17,.36,oak)
box('Reading chair seat',(5,-3.7,.46),(.75,.8,.18),cream,bevel=.08)
box('Reading chair back',(5,-3.36,.75),(.75,.18,.62),cream,bevel=.08)
for x in [4.63,5.37]:
    box('Reading chair arm',(x,-3.7,.62),(.12,.83,.18),oak,bevel=.05)
    for y in [-3.99,-3.40]:cylinder('Reading chair leg',(x,y,.22),.025,.44,oak)
cylinder('Side table',(4.85,-2.6,.27),.3,.54,oak)
plant(5.4,-.2,.35,1.5)
plant(5.4,-4.5,.3,1.1)

# Work nook and entry.
box('Desk top',(.35,3.45,.76),(2.15,.8,.08),oak)
for x in [-.58,1.28]:
    box('Desk support',(x,3.45,.36),(.07,.7,.72),charcoal)
box('Monitor',(.35,3.7,1.15),(.85,.06,.5),charcoal)
box('Desk chair',(.35,2.5,.28),(.6,.6,.56),sage)
box('Desk chair back',(.35,2.25,.8),(.6,.1,.6),sage)
box('Entry storage',(-1.8,4.65,1.05),(1.4,.5,2.1),oak)

# Bedroom and bathroom furniture proxies.
box('Bedroom rug',(4.25,3,.025),(3.1,3.2,.025),cream)
box('Bed base',(4.2,3.4,.2),(1.8,2.2,.4),oak)
box('Mattress',(4.2,3.4,.49),(1.8,2.2,.22),cream)
box('Duvet',(4.2,3.1,.64),(1.8,1.5,.1),sage)
for x in [3.75,4.65]:
    box('Pillow',(x,4.18,.68),(.65,.4,.14),cream)
box('Bedside unit',(5.5,4.2,.3),(.5,.6,.6),oak)
box('Vanity',(-5.5,3,.43),(.7,1.2,.86),oak)
box('Vanity top',(-5.5,3,.9),(.72,1.22,.06),stone)
box('Shower tray',(-5.2,4.3,.065),(1.25,1.25,.13),stone)
cylinder('Toilet proxy',(-3.55,4.25,.24),.28,.48,plaster)

# Balcony retains a central through route.
for x in [-3.8,3.8]:
    box('Balcony chair',(x,-6.1,.3),(.8,.85,.6),sage)
    box('Balcony chair back',(x,-5.77,.67),(.8,.16,.6),sage)
cylinder('Balcony table',(2.6,-6.3,.3),.38,.6,stone)
plant(-5.35,-6.5,.4,1)
plant(5.35,-6.5,.4,1)

label('KITCHEN',-4.3,1.55)
label('DINING',-3.8,-4.6)
label('LIVING',2.8,-4.62)
label('WORK',.35,1.45)
label('BEDROOM',4.4,1.6)
label('BATH',-4.2,2.35)
label('ENTRY',-.4,4.55,.18)
label('BALCONY',0,-6.7,.3)
label('SOUTH  /  LAKE ONTARIO VIEW',0,-8.2,.3)
label('WARM MODERN  /  TORONTO CONDO',0,6.2,.4)

# Recessed fixtures and a reading lamp provide visible light sources.
emissive=material('Warm lamp diffuser',(1,.82,.58),.5)
p=emissive.node_tree.nodes['Principled BSDF']
p.inputs['Emission Color'].default_value=(1,.63,.28,1)
p.inputs['Emission Strength'].default_value=3
for x,y in [(-3.8,-.5),(3,-1.8),(0,3.5)]:
    cylinder('Recessed bronze trim',(x,y,2.884),.105,.025,charcoal)
    cylinder('Recessed warm diffuser',(x,y,2.865),.082,.012,emissive)
cylinder('Reading lamp base',(5.35,-2.65,.035),.20,.07,charcoal)
cylinder('Reading lamp stem',(5.35,-2.65,.76),.018,1.5,charcoal)
cylinder('Reading lamp linen shade',(5.35,-2.65,1.48),.22,.34,cream)
cylinder('Reading lamp diffuser',(5.35,-2.65,1.315),.205,.012,emissive)

# Authored details visible in the conversation view.
for x in [-5.75,5.75]:
    for offset in [-.16,-.08,0,.08,.16]:
        cylinder('Linen curtain fold',(x+offset,-4.88,1.43),.055,2.78,cream)
for x in [1.65,3.85]:
    cushion=box('Loose linen cushion',(x,-1.05,.82),(.46,.18,.42),cream,bevel=.075)
    cushion.rotation_euler.y=.15 if x<3 else -.12
box('Book on coffee table',(2.8,-.4,.455),(.30,.22,.035),sage)
cylinder('Ceramic cup',(3.25,-.4,.49),.045,.12,stone)
for x in [-5.9,5.9]:
    box('Window ceiling trim',(x,-2,2.85),(.14,6,.08),oak)
box('South ceiling trim',(0,-4.9,2.85),(12,.14,.08),oak)
for m,name in [(oak,'oak.png'),(stone,'stone.png'),(cream,'fabric.png')]:
    tex=m.node_tree.nodes.new('ShaderNodeTexImage')
    tex.image=bpy.data.images.load(str(out/name))
    m.node_tree.links.new(tex.outputs['Color'],m.node_tree.nodes['Principled BSDF'].inputs['Base Color'])

sys.path.insert(0,str(Path(__file__).parent))
from furnishing import replace_proxies, bake_normal_strength, bake_light_oak
replace_proxies(out/'models',furniture,plants,solids)

floor_material=material('Oak floor - Poly Haven wood_floor',(1,1,1),.52)
nodes=floor_material.node_tree.nodes
links=floor_material.node_tree.links
shader=nodes.get('Principled BSDF')
for channel,filename in [('Base Color','diff'),('Normal','nor_gl')]:
    tex=nodes.new('ShaderNodeTexImage')
    tex.image=bpy.data.images.load(str(out/'models'/'wood_floor'/f'wood_floor_{filename}_2k.jpg'))
    if channel=='Base Color':tex.image=bake_light_oak(tex.image,out/'wood_floor_light_oak.png')
    if channel!='Base Color':tex.image.colorspace_settings.name='Non-Color'
    if channel=='Normal':
        tex.image=bake_normal_strength(tex.image,.05,out/'wood_floor_normal_soft.png')
        normal=nodes.new('ShaderNodeNormalMap');normal.inputs['Strength'].default_value=1
        links.new(tex.outputs['Color'],normal.inputs['Color'])
        links.new(normal.outputs['Normal'],shader.inputs[channel])
    else:links.new(tex.outputs['Color'],shader.inputs[channel])
floor.data.materials[0]=floor_material
for loop in floor.data.loops:
    p=floor.data.vertices[loop.vertex_index].co
    floor.data.uv_layers.active.data[loop.index].uv=((p.x+6)/1.7,(p.y+5)/1.7)
for ob in list(shell.objects):
    if ob.name.startswith('Oak plank seam'):bpy.data.objects.remove(ob,do_unlink=True)

from stone_material import assign_stone
assign_stone()
from upholstery import assign_upholstery
assign_upholstery()
from tabletop_props import replace_tabletop_props
replace_tabletop_props()
from balcony_furniture import replace_balcony_furniture, balcony_solids
replace_balcony_furniture()
solids.extend(balcony_solids())
from dining_furniture import replace_dining_furniture
replace_dining_furniture()
from kitchen_furniture import replace_kitchen, kitchen_solids
replace_kitchen()
solids = [s for s in solids if s['id'] not in ('kitchen_cabinetry', 'island_cabinet')]
solids.extend(kitchen_solids())
from bedroom_furniture import replace_bedroom, bedroom_solids, bedroom_lights
replace_bedroom()
solids = [s for s in solids if s['id'] != 'bed_base']
solids.extend(bedroom_solids())
from workspace_furniture import replace_workspace, workspace_solids, workspace_lights, workspace_camera
replace_workspace()
solids = [s for s in solids if s['id'] != 'entry_storage']
solids.extend(workspace_solids())
from bathroom_furniture import replace_bathroom, bathroom_solids, bathroom_lights, bathroom_camera
replace_bathroom()
solids = [s for s in solids if s['id'] not in ('vanity', 'bathroom_floor')]
solids.extend(bathroom_solids())

# Georeferenced massing is batched into a small number of shared mesh/material groups.
city=collection('06 Toronto 2025 city massing')
data=json.loads((out/'massing.json').read_text())
city_mats=[]
for i,rgb in enumerate([(.36,.43,.48),(.46,.49,.50),(.36,.39,.40),(.57,.55,.49)]):
    m=material('Toronto facade '+str(i),rgb,.45)
    tex=m.node_tree.nodes.new('ShaderNodeTexImage')
    tex.image=bpy.data.images.load(str(out/f'facade-{i}.png'))
    m.node_tree.links.new(tex.outputs['Color'],m.node_tree.nodes['Principled BSDF'].inputs['Base Color'])
    rough=m.node_tree.nodes.new('ShaderNodeTexImage')
    rough.image=bpy.data.images.load(str(out/f'facade-{i}-rough.png'))
    rough.image.colorspace_settings.name='Non-Color'
    m.node_tree.links.new(rough.outputs['Color'],m.node_tree.nodes['Principled BSDF'].inputs['Roughness'])
    city_mats.append(m)
roof_material=material('Toronto roof membrane',(.20,.215,.21),.94)
batches=[([],[],[]) for _ in city_mats]
for building in data['buildings']:
    pts=building['xy']
    cx=sum(p[0] for p in pts)/len(pts);cy=sum(p[1] for p in pts)/len(pts)
    if math.hypot(cx,cy)<65: continue
    z=building['ground_msl']-167
    h=building['height']
    if h<2: continue
    facade_index=building['id']%2+(0 if h>=50 else 2)
    verts,faces,uvs=batches[facade_index]
    parts=building['parts']+[len(pts)]
    for a,b in zip(parts,parts[1:]):
        ring=pts[a:b]
        if ring[0]==ring[-1]: ring=ring[:-1]
        if len(ring)<3:continue
        area=sum(p[0]*q[1]-q[0]*p[1] for p,q in zip(ring,ring[1:]+ring[:1]))
        if area<0: ring.reverse()
        n=len(ring);base=len(verts)
        verts.extend([(x,y,z) for x,y in ring]+[(x,y,z+h) for x,y in ring])
        faces.append(tuple(base+n+i for i in range(n)))
        uvs.append([(0,0)]*n)
        for i,(x,y) in enumerate(ring):
            j=(i+1)%n;xx,yy=ring[j];length=math.hypot(xx-x,yy-y)
            faces.append((base+i,base+j,base+n+j,base+n+i))
            uvs.append([(0,0),(length/12,0),(length/12,h/12),(0,h/12)])
for i,(verts,faces,uvs) in enumerate(batches):
    mesh=bpy.data.meshes.new('Toronto city batch '+str(i));mesh.from_pydata(verts,[],faces);mesh.update()
    uv=mesh.uv_layers.new()
    for poly,coords in zip(mesh.polygons,uvs):
        for loop,coord in zip(poly.loop_indices,coords):uv.data[loop].uv=coord
    ob=bpy.data.objects.new(mesh.name,mesh);city.objects.link(ob);mesh.materials.append(city_mats[i])
    mesh.materials.append(roof_material)
    for polygon in mesh.polygons:
        if polygon.normal.z>.5: polygon.material_index=1

# Surveyed shoreline and road centreline; scenic widths and elevations.
geography=json.loads((out/'geography.json').read_text())
asphalt=material('Asphalt',(.105,.12,.13),.92)
ground=material('City paving',(.29,.30,.28),.96)
park=material('Island landscape',(.17,.26,.12),.95)

def triangle_batch(name, triangles, z, mat):
    vertices=[(x,y,z) for triangle in triangles for x,y in triangle]
    faces=[(i,i+1,i+2) for i in range(0,len(vertices),3)]
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update()
    ob=bpy.data.objects.new(name,mesh);city.objects.link(ob);mesh.materials.append(mat)

for island in [False,True]:
    triangles=[triangle for part in geography['land'] if part['island']==island for triangle in part['triangles']]
    triangle_batch('Waterfront land '+str(island),triangles,-90.5,park if island else ground)
for z in [-80.4,-90.15]:
    triangles=[triangle for road in geography['roads'] if road['z']==z for triangle in road['triangles']]
    triangle_batch('Toronto streets '+str(z),triangles,z,asphalt)
# Quay walls expose a narrow vertical edge above the animated water plane.
vertices=[];faces=[]
for part in geography['land']:
    for a,b in zip(part['shore'],part['shore'][1:]):
        i=len(vertices)
        vertices.extend([(a[0],a[1],-92.1),(b[0],b[1],-92.1),(b[0],b[1],-90.5),(a[0],a[1],-90.5)])
        faces.append((i,i+1,i+2,i+3))
mesh=bpy.data.meshes.new('Quay walls');mesh.from_pydata(vertices,[],faces);mesh.update()
ob=bpy.data.objects.new(mesh.name,mesh);city.objects.link(ob);mesh.materials.append(stone)

from landscape import build_landscape
build_landscape(out/'landscape.json', city, material)
from street_details import build_street_details
build_street_details(geography, city, material)

from scene_export import export_selection

export_selection(out/'interior-city.glb',[shell,furniture,plants,city])
vehicle=collection('07 Traffic vehicle prefab')
paint=material('Vehicle paint',(.19,.28,.36),.26)
box('Vehicle body',(0,0,.65),(1.8,4.5,.7),paint,vehicle,.18)
box('Vehicle cabin',(0,-.1,1.12),(1.55,2.4,.65),charcoal,vehicle,.25)
for x in [-.86,.86]:
    for y in [-1.45,1.45]:
        tire=cylinder('Wheel',(x,y,.38),.36,.20,charcoal,vehicle)
        tire.rotation_euler.y=math.pi/2
for x in [-.6,.6]:
    box('Headlamp',(x,-2.24,.68),(.38,.035,.16),cream,vehicle,.025)
from vehicle_lamps import prepare_lamps
prepare_lamps(vehicle)
export_selection(out/'vehicle-base.glb',[vehicle])
from vehicle_variants import add_variants
vehicle_variants=add_variants(out/'vehicle-base.glb',out/'vehicle.glb')
manifest={'schema_version':1,'id':'toronto_condo','geometry':'interior-city.glb','floor_y':0,
    'solids':solids,'cameras':[
        {'id':'living','position':[-1.4,1.35,-2.8],'look_at':[.8,1.25,3.3],'fov_deg':55},
        {'id':'balcony','position':[0,1.65,6.2],'look_at':[200,-70,650],'fov_deg':62},
        {'id':'interior','position':[.5,1.8,4.2],'look_at':[-.5,1.2,-2.5],'fov_deg':80},
        {'id':'bedroom','position':[2.7,1.7,-1.8],'look_at':[4.3,.9,-3.5],'fov_deg':70}, workspace_camera(), bathroom_camera()],
    'traffic':geography['traffic'],'vehicle_variants':vehicle_variants,'point_lights':bedroom_lights()+workspace_lights()+bathroom_lights(),
    'lake_center':[0,-92,3400],'lake_size':[18000,8000],
    'geographic_origin':data['origin'],'provenance':'City of Toronto 3D Massing 2025; authored fictional condo, Toronto waterbody shoreline and road centreline with scenic elevations/widths. See provenance.json.'}
(out/'scene.json').write_text(json.dumps(manifest,indent=2)+'\n')
bpy.context.preferences.filepaths.save_version=0
bpy.ops.wm.save_as_mainfile(filepath=str(out/'toronto-condo.blend'))
print('CONDO_ASSET_EXPORT_COMPLETE',len(solids),len(data['buildings']),flush=True)
# Avoid an audio-driver shutdown hang in headless Blender on this host.
import os
os._exit(0)
