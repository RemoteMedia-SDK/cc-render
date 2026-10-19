//! Apply world-space tissue displacement before ordinary Bevy skinning. This
//! keeps the existing PBR, morph, depth and shadow vertex paths in use.
use crate::cc_render::soft_tissue::{Frame, SurfaceBinding, TissueAsset};
use bevy::{
    math::DVec3,
    prelude::*,
    render::mesh::{skinning::SkinnedMesh, VertexAttributeValues},
};
use std::collections::{BTreeSet, HashMap};

/// Surface quality diagnostics, separate from the cage's volume metrics.
#[derive(serde::Serialize)]
pub(super) struct SurfaceMetrics {
    pub mesh_name: String,
    pub maximum_displacement_m: f64,
    pub minimum_face_alignment: f32,
    pub worst_triangle: Option<[usize; 3]>,
}

#[cfg(test)]
#[path = "tissue_surface_tests.rs"]
mod tests;

pub(super) fn positions_hash(positions: &[[f32; 3]]) -> String {
    let bytes: Vec<_> = positions
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    hmac_sha256::Hash::hash(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(super) struct TissueSurface {
    pub entity: Option<Entity>,
    pub mesh: Handle<Mesh>,
    pub joints: Vec<Entity>,
    inverse_bind: Vec<Mat4>,
    base: Vec<Vec3>,
    morph_geometry: Option<super::tissue_morph::MorphGeometry>,
    normals: Vec<Vec3>,
    tangents: Option<Vec<Vec4>>,
    weights: Vec<[f32; 4]>,
    indices: Vec<[u16; 4]>,
    triangles: Vec<[usize; 3]>,
    pub collision_triangles: Vec<[usize; 3]>,
    affected: Vec<usize>,
    sampled: BTreeSet<usize>,
    binding: SurfaceBinding,
    smoothing: Option<super::tissue_smoothing::DisplacementSmoothing>,
}

/// Linear-blend skinning of selected vertices of one surface, unmorphed.
pub(super) struct SkinSubset {
    joints: Vec<Entity>,
    inverse_bind: Vec<Mat4>,
    base: Vec<Vec3>,
    weights: Vec<[f32; 4]>,
    indices: Vec<[u16; 4]>,
}

impl SkinSubset {
    pub fn len(&self) -> usize {
        self.base.len()
    }

    pub fn positions(&self, globals: &HashMap<Entity, Mat4>) -> anyhow::Result<Vec<DVec3>> {
        let palette: Vec<Mat4> = self.joints.iter().zip(&self.inverse_bind)
            .map(|(joint, inverse)| globals.get(joint).map(|world| *world * *inverse)
                .ok_or_else(|| anyhow::anyhow!("missing hand clamp skin joint")))
            .collect::<anyhow::Result<_>>()?;
        self.base.iter().zip(self.weights.iter().zip(&self.indices)).map(|(p, (w, j))| {
            let skin: Mat4 = (0..4).map(|k| palette[j[k] as usize] * w[k]).sum();
            anyhow::ensure!(skin.is_finite(), "nonfinite hand clamp skin transform");
            Ok(skin.transform_point3(*p).as_dvec3())
        }).collect()
    }
}

impl TissueSurface {
    pub fn subset(&self, vertices: &[usize]) -> SkinSubset {
        SkinSubset {
            joints: self.joints.clone(),
            inverse_bind: self.inverse_bind.clone(),
            base: vertices.iter().map(|i| self.base[*i]).collect(),
            weights: vertices.iter().map(|i| self.weights[*i]).collect(),
            indices: vertices.iter().map(|i| self.indices[*i]).collect(),
        }
    }

    /// The joint with the largest skin weight on each vertex.
    pub fn dominant_joints(&self) -> Vec<Entity> {
        self.weights.iter().zip(&self.indices).map(|(w, j)| {
            let k = (0..4).max_by(|a, b| w[*a].total_cmp(&w[*b])).unwrap();
            self.joints[j[k] as usize]
        }).collect()
    }

    pub fn bind_positions(&self) -> &[Vec3] {
        &self.base
    }

    pub fn capture_morphs(&mut self, mesh: &Mesh, images: &Assets<Image>) -> anyhow::Result<()> {
        self.morph_geometry = super::tissue_morph::MorphGeometry::capture(mesh, images)?;
        if let Some(morphs) = &self.morph_geometry {
            morphs.validate_embedding(&self.sampled)?;
            tracing::info!(mesh=%self.binding.mesh_name, position_targets=morphs.position_target_count(),
                "contact position morphs captured from render image");
        }
        Ok(())
    }

    pub fn kinematic_morphed_positions(&self, globals: &HashMap<Entity, Mat4>, weights: &[f32]) -> anyhow::Result<Vec<DVec3>> {
        match &self.morph_geometry {
            Some(morphs) => self.skin_positions(globals, &morphs.apply(&self.base, weights)?),
            None => self.kinematic_positions(globals),
        }
    }

    pub fn contact_morph_weights(&self, weights: Option<&[f32]>) -> anyhow::Result<Vec<f32>> {
        match &self.morph_geometry {
            Some(morphs) => morphs.position_weights(weights.ok_or_else(|| anyhow::anyhow!("missing contact morph weights"))?),
            None => Ok(Vec::new()),
        }
    }

    pub fn has_deformation(&self) -> bool {
        !self.affected.is_empty()
    }

    pub fn binding(&self) -> &SurfaceBinding {
        &self.binding
    }

    pub fn sample_areas(&self) -> Vec<f64> {
        let mut areas = vec![0.0; self.base.len()];
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| self.base[i].as_dvec3());
            let share = (b - a).cross(c - a).length() / 6.0;
            for i in t {
                areas[*i] += share;
            }
        }
        areas
    }

    pub fn kinematic_positions(
        &self,
        globals: &HashMap<Entity, Mat4>,
    ) -> anyhow::Result<Vec<DVec3>> {
        self.skin_positions(globals, &self.base)
    }

    fn skin_positions(&self, globals: &HashMap<Entity, Mat4>, positions: &[Vec3]) -> anyhow::Result<Vec<DVec3>> {
        let palette = self.palette(globals)?;
        positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let skin: Mat4 = (0..4)
                    .map(|k| palette[self.indices[i][k] as usize] * self.weights[i][k])
                    .sum();
                anyhow::ensure!(
                    skin.is_finite(),
                    "nonfinite tissue collision skin transform"
                );
                Ok(skin.transform_point3(*p).as_dvec3())
            })
            .collect()
    }

    fn palette(&self, globals: &HashMap<Entity, Mat4>) -> anyhow::Result<Vec<Mat4>> {
        self.joints
            .iter()
            .zip(&self.inverse_bind)
            .map(|(joint, inverse)| {
                globals
                    .get(joint)
                    .map(|world| *world * *inverse)
                    .ok_or_else(|| anyhow::anyhow!("missing tissue skin joint"))
            })
            .collect()
    }

    /// Read-only geometry evidence at the displayed source timestamp. Kept out
    /// of the ordinary frame path unless a one-shot snapshot was requested.
    pub fn diagnostic(
        &self,
        mesh: &Mesh,
        globals: &HashMap<Entity, Mat4>,
    ) -> anyhow::Result<serde_json::Value> {
        let palette = self.palette(globals)?;
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(VertexAttributeValues::as_float3)
            .ok_or_else(|| anyhow::anyhow!("missing diagnostic surface positions"))?;
        let mut before = Vec::with_capacity(self.base.len());
        let mut after = Vec::with_capacity(self.base.len());
        for (i, position) in positions.iter().enumerate() {
            let skin: Mat4 = (0..4)
                .map(|k| palette[self.indices[i][k] as usize] * self.weights[i][k])
                .sum();
            before.push(skin.transform_point3(self.base[i]).to_array());
            after.push(skin.transform_point3((*position).into()).to_array());
        }
        Ok(serde_json::json!({"mesh_name":self.binding.mesh_name,
            "before_world":before,"after_world":after,"affected_triangles":self.triangles,
            "skin_palette":palette.iter().map(Mat4::to_cols_array).collect::<Vec<_>>(),
            "skin_weights":self.weights,"skin_indices":self.indices,
            "base_positions":self.base.iter().map(|p|p.to_array()).collect::<Vec<_>>() }))
    }

    pub fn capture(
        mesh: &Mesh,
        binding: &SurfaceBinding,
        skin: &SkinnedMesh,
        inverse_bind: &[Mat4],
    ) -> anyhow::Result<Self> {
        let Some(VertexAttributeValues::Float32x3(base)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            anyhow::bail!("tissue mesh positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            anyhow::bail!("tissue mesh normals");
        };
        let Some(VertexAttributeValues::Float32x4(weights)) =
            mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
        else {
            anyhow::bail!("tissue mesh weights");
        };
        let Some(VertexAttributeValues::Uint16x4(indices)) =
            mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
        else {
            anyhow::bail!("tissue mesh joints");
        };
        anyhow::ensure!(
            base.len() == binding.vertex_count
                && base.len() == normals.len()
                && base.len() == weights.len()
                && base.len() == indices.len(),
            "tissue surface array sizes"
        );
        anyhow::ensure!(
            inverse_bind.len() == skin.joints.len()
                && indices
                    .iter()
                    .flatten()
                    .all(|i| (*i as usize) < inverse_bind.len()),
            "tissue skin palette bounds"
        );
        let tangents = match mesh.attribute(Mesh::ATTRIBUTE_TANGENT) {
            Some(VertexAttributeValues::Float32x4(t)) => {
                Some(t.iter().copied().map(Vec4::from).collect())
            }
            None => None,
            _ => anyhow::bail!("tissue tangent format"),
        };
        let affected: BTreeSet<_> = binding.vertices.iter().map(|v| v.vertex).collect();
        let order: Vec<_> = mesh
            .indices()
            .map(|i| i.iter().collect())
            .unwrap_or_else(|| (0..base.len()).collect());
        anyhow::ensure!(
            order.len() % 3 == 0 && order.iter().all(|i| *i < base.len()),
            "tissue triangle indices"
        );
        let triangles: Vec<[usize; 3]> = order
            .chunks_exact(3)
            .filter(|t| t.iter().any(|i| affected.contains(i)))
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        let collision_triangles = order
            .chunks_exact(3)
            .filter(|t| t.iter().all(|i| !affected.contains(i)))
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        let base: Vec<Vec3> = base.iter().copied().map(Vec3::from).collect();
        let smoothing = super::tissue_smoothing::DisplacementSmoothing::from_env(&base, &triangles, &affected)?;
        let sampled = triangles
            .iter()
            .flatten()
            .copied()
            .chain(affected.iter().copied())
            .collect();
        Ok(Self {
            entity: None,
            mesh: Handle::default(),
            joints: skin.joints.clone(),
            inverse_bind: inverse_bind.to_vec(),
            base,
            morph_geometry: None,
            normals: normals.iter().copied().map(Vec3::from).collect(),
            tangents,
            weights: weights.clone(),
            indices: indices.clone(),
            triangles,
            collision_triangles,
            affected: affected.into_iter().collect(),
            sampled,
            binding: binding.clone(),
            smoothing,
        })
    }

    /// World displacement of each embedded vertex from its kinematic skin.
    fn displacement(&self, asset: &TissueAsset, nodes: &[Vec<DVec3>], frames: &[Frame]) -> Vec<Vec3> {
        let mut delta = vec![Vec3::ZERO; self.base.len()];
        for binding in &self.binding.vertices {
            let region = &asset.regions[binding.region];
            let ids = region.tetrahedra[binding.tetrahedron];
            let movement: DVec3 = (0..4)
                .map(|k| {
                    binding.barycentric[k]
                        * (nodes[binding.region][ids[k]]
                            - frames[binding.region].point(region.initial_positions[ids[k]].into()))
                })
                .sum();
            delta[binding.vertex] += (movement * binding.influence).as_vec3();
        }
        if let Some(smoothing) = &self.smoothing {
            smoothing.apply(&mut delta);
        }
        delta
    }

    /// Triangles touching simulated skin (mesh vertex indices).
    pub fn tissue_triangles(&self) -> &[[usize; 3]] {
        &self.triangles
    }

    /// World positions of the deformed skin, as `deform` displays it; only
    /// vertices of `tissue_triangles` are meaningful (others are zero).
    pub fn deformed_positions(
        &self,
        asset: &TissueAsset,
        nodes: &[Vec<DVec3>],
        frames: &[Frame],
        globals: &HashMap<Entity, Mat4>,
    ) -> anyhow::Result<Vec<DVec3>> {
        let palette = self.palette(globals)?;
        let delta = self.displacement(asset, nodes, frames);
        let mut positions = vec![DVec3::ZERO; self.base.len()];
        for i in &self.sampled {
            let skin: Mat4 = (0..4).map(|k| palette[self.indices[*i][k] as usize] * self.weights[*i][k]).sum();
            positions[*i] = (skin.transform_point3(self.base[*i]) + delta[*i]).as_dvec3();
        }
        Ok(positions)
    }

    pub fn deform(
        &self,
        mesh: &mut Mesh,
        asset: &TissueAsset,
        nodes: &[Vec<DVec3>],
        frames: &[Frame],
        globals: &HashMap<Entity, Mat4>,
    ) -> anyhow::Result<SurfaceMetrics> {
        let palette = self.palette(globals)?;
        let delta = self.displacement(asset, nodes, frames);
        let mut matrices = vec![Mat3::IDENTITY; self.base.len()];
        let mut before = vec![Vec3::ZERO; self.base.len()];
        for i in &self.sampled {
            let skin: Mat4 = (0..4)
                .map(|k| palette[self.indices[*i][k] as usize] * self.weights[*i][k])
                .sum();
            matrices[*i] = Mat3::from_mat4(skin);
            anyhow::ensure!(
                skin.is_finite() && matrices[*i].determinant().abs() > 1e-8,
                "singular tissue skin transform"
            );
            before[*i] = skin.transform_point3(self.base[*i]);
        }
        let mut positions = self.base.clone();
        for i in &self.affected {
            positions[*i] += matrices[*i].inverse() * delta[*i];
        }
        let mut old_normal = vec![Vec3::ZERO; self.base.len()];
        let mut new_normal = old_normal.clone();
        let mut metrics = SurfaceMetrics {
            mesh_name: self.binding.mesh_name.clone(),
            maximum_displacement_m: delta.iter().map(|d| d.length() as f64).fold(0.0, f64::max),
            minimum_face_alignment: 1.0,
            worst_triangle: None,
        };
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| before[i]);
            let [da, db, dc] = t.map(|i| before[i] + delta[i]);
            let old = (b - a).cross(c - a);
            let new = (db - da).cross(dc - da);
            if old.length_squared() > 1e-20 && new.length_squared() > 1e-20 {
                let alignment = old.normalize().dot(new.normalize());
                if alignment < metrics.minimum_face_alignment {
                    metrics.minimum_face_alignment = alignment;
                    metrics.worst_triangle = Some(*t);
                }
            }
            for i in t {
                old_normal[*i] += old;
                new_normal[*i] += new;
            }
        }
        let mut normals = self.normals.clone();
        let mut tangents = self.tangents.clone();
        // Transport the authored smooth normals by the surface-normal change.
        // At zero displacement this is identity, retaining imported seams.
        for i in &self.sampled {
            if old_normal[*i].length_squared() < 1e-16 || new_normal[*i].length_squared() < 1e-16 {
                continue;
            }
            let rotation =
                Quat::from_rotation_arc(old_normal[*i].normalize(), new_normal[*i].normalize());
            let linear = matrices[*i];
            let normal = rotation * (linear.inverse().transpose() * self.normals[*i]).normalize();
            normals[*i] = (linear.transpose() * normal).normalize();
            if let Some(ref mut values) = tangents {
                let t = values[*i];
                let world = rotation * (linear * t.truncate()).normalize();
                let world = (world - normal * normal.dot(world)).normalize_or_zero();
                values[*i] = (linear.inverse() * world).normalize_or_zero().extend(t.w);
            }
        }
        anyhow::ensure!(
            positions.iter().chain(&normals).all(|v| v.is_finite()),
            "nonfinite tissue surface"
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            positions.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            normals.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
        );
        if let Some(t) = tangents {
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_TANGENT,
                t.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
            );
        }
        Ok(metrics)
    }
}
