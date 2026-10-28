//! A synthetic character (two joints, a skinned CC_Base_Body quad) through the
//! body's skin lookup and the part writer, read back with the `gltf` crate.

use cc5_hair::glb_out::{self, Output, HALF_WIDTH_MAX};
use cc5_hair::head::Body;
use cc5_hair::part::{BuiltPart, Geometry, PartMaterial, PartMesh};
use cc5_hair::source::Source;
use cc5_hair::Strand;
use glam::Vec3;
use serde_json::json;

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A GLB: joints "Root" and "Head" (skin), and a 1 m quad named CC_Base_Body in
/// the y = 1.6 plane whose left edge is skinned to Root, right edge to Head.
fn head_glb() -> Vec<u8> {
    let pos = [-0.5f32, 1.1, 0.0, 0.5, 1.1, 0.0, 0.5, 2.1, 0.0, -0.5, 2.1, 0.0];
    let joints: [u8; 16] = [0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    let weights = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let idx: [u16; 6] = [0, 1, 2, 0, 2, 3];
    let ibm: Vec<f32> = [glam::Mat4::IDENTITY, glam::Mat4::from_translation(Vec3::new(0.0, -1.6, 0.0))].iter().flat_map(|m| m.to_cols_array()).collect();
    let mut bin = Vec::new();
    let mut views = Vec::new();
    for part in [f32s(&pos), joints.to_vec(), f32s(&weights), idx.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<u8>>(), f32s(&ibm)] {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        views.push(json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": part.len()}));
        bin.extend(part);
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let doc = json!({
        "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [3]}],
        "nodes": [{"name": "Root", "children": [1]}, {"name": "Head", "translation": [0.0, 1.6, 0.0]},
                  {"name": "CC_Base_Body", "mesh": 0, "skin": 0}, {"name": "Armature", "children": [0, 2]}],
        "meshes": [{"name": "CC_Base_Body", "primitives": [{"attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "indices": 3}]}],
        "skins": [{"joints": [0, 1], "inverseBindMatrices": 4}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-0.5, 1.1, 0.0], "max": [0.5, 2.1, 0.0]},
            {"bufferView": 1, "componentType": 5121, "count": 4, "type": "VEC4"},
            {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"},
            {"bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4"}],
        "bufferViews": views, "buffers": [{"byteLength": bin.len()}]
    });
    let mut js = serde_json::to_vec(&doc).unwrap();
    while js.len() % 4 != 0 {
        js.push(b' ');
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + 8 + js.len() + 8 + bin.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

#[test]
fn skin_at_weights_sum_to_one() {
    let head = Source::from_bytes(std::path::Path::new("head.glb"), head_glb()).unwrap();
    let body = Body::from_source(&head).unwrap();
    for p in [Vec3::new(-0.4, 1.6, 0.1), Vec3::new(0.0, 1.6, -0.2), Vec3::new(0.3, 2.0, 0.05), Vec3::new(2.0, 3.0, 1.0)] {
        let s = body.skin_at(p);
        let sum: f32 = s.iter().map(|e| e.1).sum();
        assert!((sum - 1.0).abs() < 1e-5, "{p}: {s:?}");
        assert!(s.iter().all(|e| e.1 >= 0.0));
    }
    // the left edge rides Root alone, the middle mixes both joints
    assert_eq!(body.skin_at(Vec3::new(-0.5, 1.6, 0.0))[0], (0, 1.0));
    let mid = body.skin_at(Vec3::new(0.0, 1.6, 0.0));
    assert!(mid[0].1 < 0.99 && mid[1].1 > 0.01, "{mid:?}");
}

fn part(head: &Source, body: &Body) -> BuiltPart {
    let strands: Vec<Strand> = (0..3)
        .map(|k| {
            let x = -0.2 + 0.2 * k as f32;
            let pts: Vec<Vec3> = (0..5).map(|i| Vec3::new(x, 2.0 - 0.1 * i as f32, 0.05)).collect();
            Strand { rgb: vec![Vec3::new(0.3, 0.2, 0.1); pts.len()], pts, id: 0.25 * k as f32, width: 1e-4, shade: 1.0 }
        })
        .collect();
    let skin = strands.iter().map(|s| body.skin_at(s.pts[0])).collect();
    let _ = head;
    BuiltPart {
        id: "test".into(),
        meshes: vec![PartMesh { name: "test_strands".into(), material: 0, geom: Geometry::Strands { strands, skin, taper: 0.6, root_fade: 0.1 } }],
        materials: vec![PartMaterial { name: "test_Hair".into(), blend: false, cc5: Some(json!({"shader": "RLHair", "maps": {}, "srgb": [], "vars": {}})), base_factor: [1.0; 4], base_image: None }],
        entry: json!({"id": "test"}),
        files: vec![],
    }
}

#[test]
fn part_glb_round_trips_through_gltf() {
    let head = Source::from_bytes(std::path::Path::new("head.glb"), head_glb()).unwrap();
    let body = Body::from_source(&head).unwrap();
    let built = part(&head, &body);
    let dir = std::env::temp_dir().join(format!("cc5_hair_roundtrip_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for triangles in [false, true] {
        let path = dir.join(format!("test_{triangles}.glb"));
        glb_out::write(&built, &head, &path, Output { triangles }).unwrap();
        let g = gltf::Gltf::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let blob = g.blob.clone().unwrap();
        let doc = g.document;
        // the head's skeleton and skin came along, joints by name
        let skin = doc.skins().next().unwrap();
        let names: Vec<_> = skin.joints().map(|j| j.name().unwrap_or("").to_string()).collect();
        assert_eq!(names, ["Root", "Head"]);
        let mesh = doc.meshes().find(|m| m.name() == Some("test_strands")).unwrap();
        let prim = mesh.primitives().next().unwrap();
        let r = prim.reader(|b| (b.index() == 0).then_some(blob.as_slice()));
        let pos: Vec<[f32; 3]> = r.read_positions().unwrap().collect();
        let extras: serde_json::Value = serde_json::from_str(prim.material().extras().as_ref().unwrap().get()).unwrap();
        if triangles {
            assert_eq!(prim.mode(), gltf::mesh::Mode::Triangles);
            assert_eq!(pos.len(), 3 * 5 * 2); // two rows per strand
            assert!(extras["cc5"].get("ribbons").is_none());
        } else {
            assert_eq!(prim.mode(), gltf::mesh::Mode::Lines);
            assert_eq!(pos.len(), 3 * 5); // one vertex per point
            let segs: Vec<u32> = r.read_indices().unwrap().into_u32().collect();
            assert_eq!(segs.len(), 3 * 4 * 2);
            assert!(segs.chunks(2).all(|s| s[1] == s[0] + 1));
            assert!((pos[0][0] + 0.2).abs() < 1e-6 && (pos[0][1] - 2.0).abs() < 1e-6);
            // half-width over HALF_WIDTH_MAX in TEXCOORD_1.x: root fade starts at 0, tapers toward the tip
            let w: Vec<f32> = r.read_tex_coords(1).unwrap().into_f32().map(|t| t[0] * HALF_WIDTH_MAX).collect();
            assert!(w[0] < 1e-7 && w[1] > 1e-5 && w[1] <= 5e-5 + 1e-7, "{w:?}");
            assert_eq!(extras["cc5"]["ribbons"], json!(true));
        }
        // weights stay normalised after quantization
        for wt in r.read_weights(0).unwrap().into_f32() {
            assert!((wt.iter().sum::<f32>() - 1.0).abs() < 0.01);
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}
