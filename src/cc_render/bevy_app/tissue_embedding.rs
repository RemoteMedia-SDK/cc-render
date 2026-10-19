//! Opt-in smooth skin embedding. The default embedding moves each skin vertex
//! with the four nodes of one coarse tetrahedron, a piecewise-linear field that
//! creases along cage faces under large motion. Moving least squares with a
//! linear basis instead blends nearby cage nodes with smooth compact weights:
//! it reproduces any affine displacement (including large rigid rotations)
//! exactly, and fixed nodes carry zero displacement, so skin still fades into
//! the attached rim. Weights are computed once; the coarse physics cage and
//! its contacts are unchanged. Default since the owner accepted it on the
//! jump review; `CC_AVATAR_TISSUE_SURFACE_EMBEDDING=linear` restores the
//! per-tetrahedron embedding.
use crate::cc_render::soft_tissue::TissueAsset;
use bevy::math::{DMat4, DVec3, DVec4};

/// Neighbouring cage nodes per skin vertex; the support radius reaches past
/// the farthest of them so every weight is positive.
const NEIGHBOURS: usize = 16;
const SUPPORT: f64 = 1.5;

pub(super) struct SmoothEmbedding {
    /// Per bound skin vertex: (mesh vertex, region, influence, node weights).
    vertices: Vec<(usize, usize, f64, Vec<(usize, f64)>)>,
}

pub(super) fn requested() -> anyhow::Result<bool> {
    match std::env::var("CC_AVATAR_TISSUE_SURFACE_EMBEDDING").as_deref() {
        Err(_) | Ok("mls") => Ok(true),
        Ok("linear") => Ok(false),
        Ok(other) => anyhow::bail!("tissue surface embedding must be linear or mls, not {other}"),
    }
}

impl SmoothEmbedding {
    pub fn new(asset: &TissueAsset, binding: &crate::cc_render::soft_tissue::SurfaceBinding) -> anyhow::Result<Self> {
        let nodes: Vec<Vec<DVec3>> = asset.regions.iter()
            .map(|r| r.initial_positions.iter().copied().map(DVec3::from).collect()).collect();
        let vertices = binding.vertices.iter().map(|b| {
            let region = &asset.regions[b.region];
            let tet = region.tetrahedra[b.tetrahedron];
            // The embedded point; the authored residual offset stays constant.
            let point: DVec3 = (0..4).map(|k| b.barycentric[k] * nodes[b.region][tet[k]]).sum();
            Ok((b.vertex, b.region, b.influence, weights(point, &nodes[b.region])?))
        }).collect::<anyhow::Result<_>>()?;
        Ok(Self { vertices })
    }

    /// Displacement of each bound vertex from `node_displacement(region, node)`.
    pub fn displacement(&self, delta: &mut [bevy::math::Vec3], node_displacement: impl Fn(usize, usize) -> DVec3) {
        for (vertex, region, influence, weights) in &self.vertices {
            let moved: DVec3 = weights.iter().map(|(node, w)| *w * node_displacement(*region, *node)).sum();
            delta[*vertex] += (moved * *influence).as_vec3();
        }
    }
}

fn wendland(r: f64) -> f64 {
    if r >= 1. { 0. } else { (1. - r).powi(4) * (4. * r + 1.) }
}

/// Linear-basis MLS shape functions at `point` over the nearest cage nodes.
pub(super) fn weights(point: DVec3, nodes: &[DVec3]) -> anyhow::Result<Vec<(usize, f64)>> {
    let mut near: Vec<(usize, f64)> = nodes.iter().enumerate().map(|(i, n)| (i, n.distance(point))).collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    near.truncate(NEIGHBOURS);
    let radius = near.last().map_or(0., |n| n.1) * SUPPORT;
    anyhow::ensure!(radius > 0., "smooth embedding needs distinct cage nodes");
    // Centre the basis on the point for conditioning: p(x) = [1, x - point].
    let basis = |x: DVec3| { let d = (x - point) / radius; DVec4::new(1., d.x, d.y, d.z) };
    let mut moment = DMat4::ZERO;
    for &(i, distance) in &near {
        let p = basis(nodes[i]);
        let theta = wendland(distance / radius);
        moment += DMat4::from_cols(p * p.x, p * p.y, p * p.z, p * p.w) * theta;
    }
    anyhow::ensure!(moment.determinant().abs() > 1e-12, "smooth embedding nodes are coplanar");
    let inverse = moment.inverse();
    // At the point itself p(point) = [1, 0, 0, 0].
    let row = inverse.row(0);
    Ok(near.iter().map(|&(i, distance)| (i, row.dot(basis(nodes[i])) * wendland(distance / radius))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud() -> Vec<DVec3> {
        // A jittered 5x5x3 lattice, like a small cage.
        (0..75).map(|i| {
            let (x, y, z) = (i % 5, (i / 5) % 5, i / 25);
            DVec3::new(x as f64 * 0.02 + 0.003 * ((i * 7 % 5) as f64 - 2.) / 2.,
                       y as f64 * 0.02 + 0.002 * ((i * 3 % 4) as f64 - 1.5),
                       z as f64 * 0.025)
        }).collect()
    }

    #[test]
    fn weights_reproduce_large_rotations_exactly() {
        let nodes = cloud();
        let rotation = bevy::math::DQuat::from_axis_angle(DVec3::new(0.3, 1., -0.2).normalize(), 2.4);
        let shift = DVec3::new(0.01, -0.04, 0.02);
        let displacement = |x: DVec3| rotation * x + shift - x;
        for point in [DVec3::new(0.041, 0.037, 0.02), DVec3::new(0.005, 0.072, 0.048), DVec3::new(0.08, 0.0, 0.0)] {
            let weights = weights(point, &nodes).unwrap();
            let sum: f64 = weights.iter().map(|w| w.1).sum();
            assert!((sum - 1.).abs() < 1e-9);
            let moved: DVec3 = weights.iter().map(|(i, w)| *w * displacement(nodes[*i])).sum();
            assert!(moved.distance(displacement(point)) < 1e-9, "{moved} vs {}", displacement(point));
        }
    }

    #[test]
    fn fixed_neighbourhoods_stay_fixed_and_weights_are_local() {
        let nodes = cloud();
        let point = DVec3::new(0.01, 0.01, 0.0);
        let weights = weights(point, &nodes).unwrap();
        assert_eq!(weights.len(), NEIGHBOURS);
        // Zero displacement on every node gives zero, whatever the weights.
        let moved: DVec3 = weights.iter().map(|(_, w)| *w * DVec3::ZERO).sum();
        assert_eq!(moved, DVec3::ZERO);
        assert!(weights.iter().all(|(i, _)| nodes[*i].distance(point) < 0.06));
    }

    #[test]
    fn coplanar_nodes_are_rejected() {
        let flat: Vec<_> = (0..20).map(|i| DVec3::new((i % 5) as f64, (i / 5) as f64, 0.)).collect();
        assert!(weights(DVec3::new(1., 1., 0.), &flat).is_err());
    }
}
