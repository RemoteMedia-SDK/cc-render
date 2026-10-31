"""SMPL-X FBX -> cc-render GLB: Blender's FBX import + glTF export, with the
22 SMPL body joints renamed to the CC5 names cc-render's bind capture looks for
(same topology; a fixed name mapping, no pose change).

--arkit-map: for the pipeline's `<op>_smplx_cc5.fbx`, whose `<op>_face` mesh
carries the CC5 expression shapes as shape keys. Exports the shape keys the map
uses as glTF morph targets (cc-render's Bevy drives at most 128 per mesh; there
are 335) and writes `<out>.arkit_map.json`: the map with each morph's `meshes`
set to the meshes that carry it, and the entries whose morph is missing dropped."""
import argparse, json, os, bpy, sys, numpy as np
ap = argparse.ArgumentParser()
ap.add_argument("fbx"); ap.add_argument("out")
ap.add_argument("--arkit-map", help="ARKit -> CC5 morph map (e.g. beautiful_assistant.arkit_map.resolved.json)")
args = ap.parse_args(sys.argv[sys.argv.index("--")+1:])
fbx, out = args.fbx, args.out
RENAME = {
    "pelvis": "CC_Base_Hip", "left_hip": "CC_Base_L_Thigh", "right_hip": "CC_Base_R_Thigh",
    "spine1": "CC_Base_Waist", "left_knee": "CC_Base_L_Calf", "right_knee": "CC_Base_R_Calf",
    "spine2": "CC_Base_Spine01", "left_ankle": "CC_Base_L_Foot", "right_ankle": "CC_Base_R_Foot",
    "spine3": "CC_Base_Spine02", "left_foot": "CC_Base_L_ToeBase", "right_foot": "CC_Base_R_ToeBase",
    "neck": "CC_Base_NeckTwist01", "left_collar": "CC_Base_L_Clavicle", "right_collar": "CC_Base_R_Clavicle",
    "head": "CC_Base_Head", "left_shoulder": "CC_Base_L_Upperarm", "right_shoulder": "CC_Base_R_Upperarm",
    "left_elbow": "CC_Base_L_Forearm", "right_elbow": "CC_Base_R_Forearm",
    "left_wrist": "CC_Base_L_Hand", "right_wrist": "CC_Base_R_Hand",
    "jaw": "CC_Base_JawRoot", "left_eye_smplhf": "CC_Base_L_Eye", "right_eye_smplhf": "CC_Base_R_Eye",
}
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=fbx)
arm = next(o for o in bpy.data.objects if o.type == 'ARMATURE')
# The skinned meshes (the CC5 FBX splits the moving face off the body); drop
# strays such as the CC5 FBX's unparented Cube.
meshes = [o for o in bpy.data.objects if o.type == 'MESH' and o.find_armature() == arm]
for o in [o for o in bpy.data.objects if o.type == 'MESH' and o not in meshes]:
    print("DROPPED unskinned mesh", o.name)
    bpy.data.objects.remove(o)
# Bevy 0.15 skins at most 256 joints. Fold every non-SMPL-X bone (the CoD
# face rig under head/jaw) into its nearest SMPL-X ancestor: at rest a child
# moves rigidly with that ancestor, so skinning is unchanged for any pose that
# leaves those bones at rest -- which is every pose cc-render can drive.
SMPLX = {"pelvis", "spine1", "spine2", "spine3", "neck", "head", "jaw",
         "left_eye_smplhf", "right_eye_smplhf"} | {
    side + j for side in ("left_", "right_") for j in
    ("hip", "knee", "ankle", "foot", "collar", "shoulder", "elbow", "wrist")} | {
    side + f + str(i) for side in ("left_", "right_")
    for f in ("index", "middle", "pinky", "ring", "thumb") for i in (1, 2, 3)}
# The pipeline's head-only rig (`--head-only`) has just spine3 -> neck -> head
# -> jaw / eyes of these; its face bones fold the same way.
HEAD = {"spine3", "neck", "head", "jaw", "left_eye_smplhf", "right_eye_smplhf"}
present = SMPLX & set(arm.data.bones.keys())
assert present in (SMPLX, HEAD), sorted(SMPLX - present)
print("SKELETON", "full" if present == SMPLX else "head-only")
SMPLX = present
fold = {}
for b in arm.data.bones:
    if b.name in SMPLX: continue
    a = b.parent
    while a.name not in SMPLX: a = a.parent
    fold[b.name] = a.name
into = set()
for mesh in meshes:
    vg = mesh.vertex_groups
    idx_to_name = {g.index: g.name for g in vg}
    acc = {}
    for v in mesh.data.vertices:
        for g in v.groups:
            n = idx_to_name[g.group]
            if n in fold and g.weight > 0:
                acc.setdefault(fold[n], {}).setdefault(v.index, 0.0)
                acc[fold[n]][v.index] += g.weight
    for target, wmap in acc.items():
        tg = vg.get(target) or vg.new(name=target)
        for vi, w in wmap.items():
            tg.add([vi], w, 'ADD')
    for n in fold:
        if n in vg: vg.remove(vg[n])
    into |= set(acc)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode='EDIT')
for n in fold:
    arm.data.edit_bones.remove(arm.data.edit_bones[n])
bpy.ops.object.mode_set(mode='OBJECT')
print("FOLDED", len(fold), "bones into", sorted(into), "; joints now", len(arm.data.bones))

RENAME = {old: new for old, new in RENAME.items() if old in SMPLX}
for old, new in RENAME.items():
    arm.data.bones[old].name = new
groups = {g.name for m in meshes for g in m.vertex_groups}
missing = [n for n in RENAME.values() if n not in groups and n not in ("CC_Base_Hip", "CC_Base_L_Eye", "CC_Base_R_Eye", "CC_Base_JawRoot")]
print("RENAMED; vgroups missing:", missing)

# Morph targets: keep only the shape keys the ARKit map drives.
morph_meshes = {}
if args.arkit_map:
    amap = json.load(open(args.arkit_map))
    wanted = {t["morph"] for ts in amap["mapping"].values() for t in ts}
    for m in meshes:
        keys = m.data.shape_keys
        if not keys:
            continue
        for kb in list(keys.key_blocks)[1:]:  # [0] is the basis
            if kb.name not in wanted:
                m.shape_key_remove(kb)
        kept = [kb.name for kb in m.data.shape_keys.key_blocks[1:]]
        assert len(kept) <= 128, (m.name, len(kept))
        for k in kept:
            morph_meshes.setdefault(k, []).append(m.name)
        print("MORPHS", m.name, len(kept))
    print("MORPHS missing:", sorted(wanted - set(morph_meshes)))

# Alpha: keep blending only where the texture has real transparency.
alpha_cache = {}
for mat in bpy.data.materials:
    nt = mat.node_tree
    bsdf = next((n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED'), None)
    link = next((l for l in nt.links if l.to_node == bsdf and l.to_socket.name == 'Alpha'), None)
    if link is None:
        continue
    img = link.from_node.image
    if img.filepath not in alpha_cache:
        px = np.empty(len(img.pixels), dtype=np.float32)
        img.pixels.foreach_get(px)
        a = px[3::4]
        alpha_cache[img.filepath] = (float((a < 0.05).mean()), float((a > 0.95).mean()), float(a.mean()))
    zero, one, mean = alpha_cache[img.filepath]
    # Cutout: a few percent of ~0 texels, nearly everything else ~1 (the
    # necklaces). Mid values or mostly-zero alpha are packed data, not opacity.
    if 0.01 < zero < 0.3 and zero + one > 0.9:
        mode = "BLEND"
    else:
        nt.links.remove(link)
        mode = "OPAQUE"
    frac = zero
    print("ALPHA", mat.name, img.size[:], "zero=%.3f one=%.3f mean=%.3f" % alpha_cache[img.filepath], mode)

bpy.ops.export_scene.gltf(filepath=out, export_format='GLB', export_yup=True,
    export_animations=False, export_skins=True,
    export_morph=bool(morph_meshes), export_morph_normal=True, export_morph_tangent=False,
    export_image_format='AUTO', export_materials='EXPORT')
print("EXPORTED", out)

if args.arkit_map:
    mapping = {}
    for arkit, targets in amap["mapping"].items():
        kept = [{"morph": t["morph"], "weight": t.get("weight", 1.0), "meshes": morph_meshes[t["morph"]]}
                for t in targets if t["morph"] in morph_meshes]
        if kept:
            mapping[arkit] = kept
    map_out = os.path.splitext(out)[0] + ".arkit_map.json"
    json.dump({
        "_comment": "ARKit-52 -> CC5 expression shapes transferred onto the operator (smplx_pipeline cc5_transfer.py), "
                    "baked into the GLB by smplx_to_glb.py; each morph is driven on the meshes listed.",
        "_source": args.arkit_map,
        "_glb_path": os.path.basename(out),
        "mapping": mapping,
    }, open(map_out, "w"), indent=2)
    print("WROTE", map_out, len(mapping), "ARKit keys; dropped:", sorted(set(amap["mapping"]) - set(mapping)))
