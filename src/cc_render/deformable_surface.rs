//! Read-only shape contacts against observed, deforming material triangles.
//!
//! This is geometry evidence, not a cloth solver or grasp-admission policy.
//! Callers must supply the actual current collider pose. Multiple folded layers
//! remain separate contacts; this module does not choose a layer or attach it.
use std::sync::Arc;

use bevy::math::{Vec2, Vec3};
use bevy_rapier3d::rapier::parry::{
    bounding_volume::{Aabb, BoundingVolume},
    math::{Isometry, Point},
    query::{contact, PointQueryWithLocation},
    shape::{Shape, Triangle},
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SurfaceError {
    #[error("invalid material surface: {0}")]
    InvalidMesh(&'static str),
    #[error("invalid collider pose or bounds")]
    InvalidCollider,
    #[error("unsupported collider/triangle query")]
    UnsupportedShape,
    #[error("invalid contact witness")]
    InvalidWitness,
    #[error("surface revision exhausted")]
    RevisionExhausted,
}

/// Opaque material identity. A point can follow deformation after capture;
/// retaining it does not retain evidence of contact or permission to grasp.
#[derive(Clone, Debug)]
pub struct MaterialPoint {
    identity: Arc<()>,
    triangle: usize,
    barycentric: [f32; 3],
}

impl MaterialPoint {
    pub fn triangle(&self) -> usize {
        self.triangle
    }
    pub fn barycentric(&self) -> [f32; 3] {
        self.barycentric
    }
}

#[derive(Clone, Debug)]
pub struct SurfaceContact {
    material: MaterialPoint,
    revision: u64,
    hand_position: Vec3,
    midsurface_position: Vec3,
    normal: Vec3,
    separation: f32,
}

impl SurfaceContact {
    pub fn triangle(&self) -> usize {
        self.material.triangle
    }
    pub fn barycentric(&self) -> [f32; 3] {
        self.material.barycentric
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn hand_position(&self) -> Vec3 {
        self.hand_position
    }
    pub fn midsurface_position(&self) -> Vec3 {
        self.midsurface_position
    }
    /// Outward contact normal toward the querying shape, including edge contacts.
    pub fn normal(&self) -> Vec3 {
        self.normal
    }
    /// Signed separation from the authored shell, not the mid-surface.
    pub fn separation(&self) -> f32 {
        self.separation
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedMaterialPoint {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub revision: u64,
}

/// A fixed topology with replaceable observed positions in world metres.
/// Every successful observation update invalidates earlier contact evidence.
pub struct DeformedSurface {
    identity: Arc<()>,
    positions: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    material_uv: Vec<Vec2>,
    half_thickness: f32,
    revision: u64,
}

impl DeformedSurface {
    pub fn new(
        positions: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
        material_uv: Vec<Vec2>,
        physical_thickness_m: f32,
    ) -> Result<Self, SurfaceError> {
        if !physical_thickness_m.is_finite() || physical_thickness_m < 0.0 {
            return Err(SurfaceError::InvalidMesh("invalid physical thickness"));
        }
        if material_uv.len() != positions.len() || material_uv.iter().any(|v| !v.is_finite()) {
            return Err(SurfaceError::InvalidMesh("invalid material UV coordinates"));
        }
        validate_positions(&positions, &triangles)?;
        Ok(Self {
            identity: Arc::new(()),
            positions,
            triangles,
            material_uv,
            half_thickness: physical_thickness_m * 0.5,
            revision: 0,
        })
    }

    pub fn positions(&self) -> &[Vec3] {
        &self.positions
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Only accepts a new geometry observation. Does not integrate, repair,
    /// relocate a body, change topology, or preserve stale contact authority.
    pub fn update_positions(&mut self, positions: Vec<Vec3>) -> Result<(), SurfaceError> {
        if positions.len() != self.positions.len() {
            return Err(SurfaceError::InvalidMesh("topology changed"));
        }
        validate_positions(&positions, &self.triangles)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(SurfaceError::RevisionExhausted)?;
        self.positions = positions;
        self.revision = revision;
        Ok(())
    }

    /// Prepare a transactional replacement while preserving material identity.
    /// Callers can prepare every layer before committing any surface revision.
    pub(crate) fn updated(&self, positions: Vec<Vec3>) -> Result<Self, SurfaceError> {
        let mut pending = Self {
            identity: self.identity.clone(),
            positions: self.positions.clone(),
            triangles: self.triangles.clone(),
            material_uv: self.material_uv.clone(),
            half_thickness: self.half_thickness,
            revision: self.revision,
        };
        pending.update_positions(positions)?;
        Ok(pending)
    }

    /// All actual shape/shell intersections, preserving distinct material faces.
    /// Thickness is explicit geometry: there is no extra proximity margin.
    /// No exposure, pinch, force, closure, or grasp-policy decision is made here.
    pub fn contacts(
        &self,
        shape: &dyn Shape,
        pose: &Isometry<f32>,
    ) -> Result<Vec<SurfaceContact>, SurfaceError> {
        validate_pose(pose)?;
        let raw_bounds = shape.compute_aabb(pose);
        validate_bounds(&raw_bounds)?;
        let bounds = raw_bounds.loosened(self.half_thickness);
        validate_bounds(&bounds)?;
        let mut contacts = Vec::new();
        for (index, indices) in self.triangles.iter().enumerate() {
            let triangle = triangle_at(&self.positions, *indices);
            if !bounds.intersects(&triangle.local_aabb()) {
                continue;
            }
            let witness = contact(
                pose,
                shape,
                &Isometry::identity(),
                &triangle,
                self.half_thickness,
            )
            .map_err(|_| SurfaceError::UnsupportedShape)?;
            if let Some(witness) = witness {
                if !witness.dist.is_finite() {
                    return Err(SurfaceError::InvalidWitness);
                }
                if witness.dist <= self.half_thickness {
                    contacts.push(self.material_contact(index, &triangle, witness)?);
                }
            }
        }
        Ok(contacts)
    }

    /// Checks only this surface's identity and geometry observation revision.
    /// A moving/changed querying collider must be queried again before binding.
    pub fn contact_matches_surface(&self, contact: &SurfaceContact) -> bool {
        Arc::ptr_eq(&self.identity, &contact.material.identity) && self.revision == contact.revision
    }

    /// Obtain a material reference only from a contact with this current frame.
    /// This does not create an attachment or certify that grasping is feasible.
    pub fn material_point_from_contact(&self, contact: &SurfaceContact) -> Option<MaterialPoint> {
        self.contact_matches_surface(contact)
            .then(|| contact.material.clone())
    }

    pub fn resolve(&self, point: &MaterialPoint) -> Option<ResolvedMaterialPoint> {
        if !Arc::ptr_eq(&self.identity, &point.identity) {
            return None;
        }
        let indices = self.triangles[point.triangle];
        let vertices = indices.map(|i| self.positions[i as usize]);
        let position = vertices
            .iter()
            .zip(point.barycentric)
            .map(|(v, w)| *v * w)
            .sum();
        let uv = indices
            .iter()
            .zip(point.barycentric)
            .map(|(i, w)| self.material_uv[*i as usize] * w)
            .sum();
        let normal = (vertices[1] - vertices[0])
            .cross(vertices[2] - vertices[0])
            .normalize();
        Some(ResolvedMaterialPoint {
            position,
            normal,
            uv,
            revision: self.revision,
        })
    }

    pub(super) fn material_vertices(&self, point: &MaterialPoint) -> Option<[usize; 3]> {
        Arc::ptr_eq(&self.identity, &point.identity)
            .then(|| self.triangles[point.triangle].map(|i| i as usize))
    }

    fn material_contact(
        &self,
        index: usize,
        triangle: &Triangle,
        witness: bevy_rapier3d::rapier::parry::query::Contact,
    ) -> Result<SurfaceContact, SurfaceError> {
        let (projection, location) =
            triangle.project_local_point_and_get_location(&witness.point2, false);
        let barycentric = location
            .barycentric_coordinates()
            .ok_or(SurfaceError::InvalidWitness)?;
        let hand_position = Vec3::from_array(witness.point1.coords.into());
        let midsurface_position = Vec3::from_array(projection.point.coords.into());
        let normal = Vec3::from_array((*witness.normal2).into());
        if !hand_position.is_finite()
            || !midsurface_position.is_finite()
            || !normal.is_finite()
            || !witness.dist.is_finite()
            || barycentric.iter().any(|w| !w.is_finite() || *w < 0.0)
            || (barycentric.iter().sum::<f32>() - 1.0).abs() > 1e-5
            || (projection.point - witness.point2).norm() > 1e-4
        {
            return Err(SurfaceError::InvalidWitness);
        }
        Ok(SurfaceContact {
            material: MaterialPoint {
                identity: self.identity.clone(),
                triangle: index,
                barycentric,
            },
            revision: self.revision,
            hand_position,
            midsurface_position,
            normal,
            separation: witness.dist - self.half_thickness,
        })
    }
}

fn validate_bounds(bounds: &Aabb) -> Result<(), SurfaceError> {
    if !bounds
        .mins
        .iter()
        .chain(bounds.maxs.iter())
        .all(|v| v.is_finite())
        || (0..3).any(|i| bounds.mins[i] > bounds.maxs[i])
    {
        return Err(SurfaceError::InvalidCollider);
    }
    Ok(())
}

fn validate_pose(pose: &Isometry<f32>) -> Result<(), SurfaceError> {
    if !pose
        .translation
        .vector
        .iter()
        .chain(pose.rotation.coords.iter())
        .all(|x| x.is_finite())
        || (pose.rotation.norm_squared() - 1.0).abs() > 1e-4
    {
        return Err(SurfaceError::InvalidCollider);
    }
    Ok(())
}

fn triangle_at(positions: &[Vec3], indices: [u32; 3]) -> Triangle {
    let [a, b, c] = indices.map(|i| Point::from(positions[i as usize].to_array()));
    Triangle::new(a, b, c)
}

fn validate_positions(positions: &[Vec3], triangles: &[[u32; 3]]) -> Result<(), SurfaceError> {
    if positions.len() < 3 || triangles.is_empty() || positions.iter().any(|p| !p.is_finite()) {
        return Err(SurfaceError::InvalidMesh("missing or nonfinite geometry"));
    }
    for indices in triangles {
        if indices.iter().any(|i| *i as usize >= positions.len()) {
            return Err(SurfaceError::InvalidMesh("triangle index out of range"));
        }
        if triangle_at(positions, *indices)
            .normal()
            .filter(|n| n.iter().all(|v| v.is_finite()))
            .is_none()
        {
            return Err(SurfaceError::InvalidMesh("degenerate triangle"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "deformable_surface_tests.rs"]
mod tests;
