use super::*;
use crate::cc_render::soft_tissue::{Attachment, Material, RegionAsset, VertexBinding};
use bevy::{asset::RenderAssetUsages, render::render_resource::PrimitiveTopology};

#[test]
fn tissue_surface_matches_world_displacement_and_preserves_zero_deformation() {
    let initial = vec![
        [0.0, 0.0, 0.0],
        [0.1, 0.0, 0.0],
        [0.0, 0.1, 0.0],
        [0.0, 0.0, 0.1],
    ];
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0_f32, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0_f32, 0.0, 1.0]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0_f32, 0.0, 0.0, 1.0]; 3]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(vec![[0_u16; 4]; 3]),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_WEIGHT,
        vec![[1.0_f32, 0.0, 0.0, 0.0]; 3],
    );
    let binding = SurfaceBinding {
        mesh_name: "test".into(),
        positions_sha256: "0".repeat(64),
        vertex_count: 3,
        vertices: (0..3)
            .map(|i| {
                let mut barycentric = [0.0; 4];
                barycentric[i] = 1.0;
                VertexBinding {
                    vertex: i,
                    region: 0,
                    tetrahedron: 0,
                    barycentric,
                    influence: 1.0,
                }
            })
            .collect(),
    };
    let asset = TissueAsset {
        schema_version: 1,
        contact_surfaces: vec![],
        contact_inactive_morph_targets: vec![],
        source_sha256: vec!["0".repeat(64)],
        surfaces: vec![binding.clone()],
        regions: vec![RegionAsset {
            name: "test".into(),
            anchor_bone: "test".into(),
            rest_positions: initial.clone(),
            initial_positions: initial.clone(),
            tetrahedra: vec![[0, 1, 2, 3]],
            attachments: vec![Attachment::Fixed; 4],
            material: Material {
                pressure_discretization: Default::default(),
                pressure_stabilization_pa: 0.0,
                element_pressure_quality_cutoff: 0.0,
                time_integration: Default::default(),
                viscosity: None,
                density_kg_m3: 950.0,
                shear_pa: 1500.0,
                strain_stiffening: 0.0,
                bulk_pa: 30000.0,
                strain_damping_per_second: 0.0,
                iterations: 2,
            },
        }],
    };
    let joint = Entity::from_raw(1);
    let skin = SkinnedMesh {
        inverse_bindposes: Handle::default(),
        joints: vec![joint],
    };
    let surface = TissueSurface::capture(&mesh, &binding, &skin, &[Mat4::IDENTITY]).unwrap();
    let matrix = Mat4::from_scale_rotation_translation(
        Vec3::new(1.1, 0.9, 1.0),
        Quat::from_rotation_y(0.4),
        Vec3::new(3.0, 2.0, -1.0),
    );
    let globals = HashMap::from([(joint, matrix)]);
    let mut nodes: Vec<_> = initial.iter().copied().map(DVec3::from).collect();
    surface
        .deform(
            &mut mesh,
            &asset,
            &[nodes.clone()],
            &[Frame::IDENTITY],
            &globals,
        )
        .unwrap();
    let Some(VertexAttributeValues::Float32x3(zero)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!();
    };
    for (actual, original) in zero.iter().zip(&surface.base) {
        assert!(Vec3::from(*actual).distance(*original) < 1e-7);
    }
    nodes[2].z += 0.02;
    surface
        .deform(&mut mesh, &asset, &[nodes], &[Frame::IDENTITY], &globals)
        .unwrap();
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!();
    };
    let world: Vec<_> = positions
        .iter()
        .map(|p| matrix.transform_point3((*p).into()))
        .collect();
    for i in 0..3 {
        let expected = matrix.transform_point3(surface.base[i])
            + if i == 2 { Vec3::Z * 0.02 } else { Vec3::ZERO };
        assert!(world[i].distance(expected) < 5e-7);
    }
    let expected_normal = (world[1] - world[0]).cross(world[2] - world[0]).normalize();
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!();
    };
    let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
    for n in normals {
        assert!(
            (normal_matrix * Vec3::from(*n))
                .normalize()
                .distance(expected_normal)
                < 1e-5
        );
    }
}

#[test]
fn adjacency_welds_uv_seam_duplicates() {
    // Two triangles sharing edge (1,2); the right one uses a seam copy of 2.
    let base = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1., 1., 0.), Vec3::Y];
    assert_eq!(super::adjacent_triangles(&base, &[[0, 1, 2], [1, 3, 4]]), vec![[0, 1]]);
    assert!(super::adjacent_triangles(&base, &[[0, 1, 2]]).is_empty());
}
