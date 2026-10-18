//! Position deltas decoded from the same Bevy image sampled by morph.wgsl.
//! Weights stay read-only; skinning must run after these local-space offsets.
use bevy::{prelude::*, render::render_resource::{TextureDimension, TextureFormat}};

pub(super) struct MorphGeometry {
    vertex_count: usize,
    targets: Vec<Vec<(usize, Vec3)>>,
}

impl MorphGeometry {
    pub fn capture(mesh: &Mesh, images: &Assets<Image>) -> anyhow::Result<Option<Self>> {
        mesh.morph_targets().map(|handle| {
            let image = images.get(handle).ok_or_else(|| anyhow::anyhow!("missing contact morph image"))?;
            Self::decode(image, mesh.count_vertices())
        }).transpose()
    }

    fn decode(image: &Image, vertex_count: usize) -> anyhow::Result<Self> {
        let descriptor = &image.texture_descriptor;
        let size = descriptor.size;
        let layers = size.depth_or_array_layers as usize;
        let layer_components = size.width as usize * size.height as usize;
        // Bevy 0.15 MorphAttributes packs position/normal/tangent, three f32s
        // each, and pads each target to one texture layer. No GLB re-decoding.
        anyhow::ensure!(descriptor.format == TextureFormat::R32Float
            && descriptor.dimension == TextureDimension::D3
            && layers <= bevy::render::mesh::morph::MAX_MORPH_WEIGHTS
            && vertex_count.checked_mul(9).is_some_and(|n| n <= layer_components)
            && layer_components.checked_mul(layers).and_then(|n| n.checked_mul(4)) == Some(image.data.len()),
            "unsupported contact morph image layout");
        let mut targets = Vec::with_capacity(layers);
        for layer in 0..layers {
            let mut deltas = Vec::new();
            for vertex in 0..vertex_count {
                let offset = (layer * layer_components + vertex * 9) * 4;
                let component = |i: usize| f32::from_ne_bytes(image.data[offset + i * 4..offset + (i + 1) * 4].try_into().unwrap());
                let delta = Vec3::new(component(0), component(1), component(2));
                anyhow::ensure!(delta.is_finite(), "nonfinite contact morph displacement");
                if delta != Vec3::ZERO { deltas.push((vertex, delta)); }
            }
            targets.push(deltas);
        }
        Ok(Self { vertex_count, targets })
    }

    pub fn apply(&self, base: &[Vec3], weights: &[f32]) -> anyhow::Result<Vec<Vec3>> {
        anyhow::ensure!(base.len() == self.vertex_count && weights.len() >= self.targets.len()
            && weights[..self.targets.len()].iter().all(|w| w.is_finite()),
            "invalid contact morph weights or vertex count");
        let mut result = base.to_vec();
        // Same target order and f32 operations as mesh.wgsl::morph_vertex.
        // Negative weights remain valid; no facial values are clamped.
        for (target, weight) in self.targets.iter().zip(weights) {
            if *weight == 0.0 { continue; }
            for (vertex, delta) in target { result[*vertex] += *weight * *delta; }
        }
        anyhow::ensure!(result.iter().all(|p| p.is_finite()), "nonfinite morphed contact positions");
        Ok(result)
    }

    pub fn position_target_count(&self) -> usize {
        self.targets.iter().filter(|target| !target.is_empty()).count()
    }

    pub fn validate_embedding(&self, vertices: &std::collections::BTreeSet<usize>) -> anyhow::Result<()> {
        anyhow::ensure!(self.targets.iter().flatten().all(|(vertex, _)| !vertices.contains(vertex)),
            "position morph overlaps the fixed tissue embedding or its normal stencil");
        Ok(())
    }

    pub fn position_weights(&self, weights: &[f32]) -> anyhow::Result<Vec<f32>> {
        anyhow::ensure!(weights.len() >= self.targets.len(), "missing contact morph weights");
        let result: Vec<_> = self.targets.iter().zip(weights)
            .map(|(target, weight)| if target.is_empty() { 0.0 } else { *weight }).collect();
        anyhow::ensure!(result.iter().all(|w| w.is_finite()), "nonfinite contact morph weights");
        Ok(result)
    }
}

/// Observed facial geometry on the body's presentation clock, not facial PTS.
/// Interpolation uses only two observed endpoints; no facial pose is authored.
#[derive(Default)]
pub(super) struct MorphHistory {
    samples: std::collections::VecDeque<(f64, Vec<Vec<f32>>)>,
}

impl MorphHistory {
    pub fn observe(&mut self, at: f64, weights: &[Vec<f32>], retain_from: f64) -> anyhow::Result<()> {
        anyhow::ensure!(at.is_finite() && retain_from.is_finite()
            && weights.iter().flatten().all(|w| w.is_finite()), "invalid contact morph observation");
        if let Some((previous, values)) = self.samples.back() {
            anyhow::ensure!(at >= *previous && values.len() == weights.len()
                && values.iter().zip(weights).all(|(a,b)| a.len() == b.len()), "contact morph clock or topology changed");
            if at == *previous {
                anyhow::ensure!(values == weights,
                    "contact facial geometry changed while body clock paused; no advancing simulation interval");
                return Ok(());
            }
        }
        while self.samples.len() > 1 && self.samples[1].0 <= retain_from { self.samples.pop_front(); }
        anyhow::ensure!(self.samples.len() < 256, "contact morph history backlog");
        self.samples.push_back((at, weights.to_vec()));
        Ok(())
    }

    pub fn sample(&self, at: f64) -> anyhow::Result<Vec<Vec<f32>>> {
        let first = self.samples.front().ok_or_else(|| anyhow::anyhow!("missing contact morph history"))?;
        let last = self.samples.back().unwrap();
        anyhow::ensure!(at.is_finite() && at >= first.0 - 1e-6 && at <= last.0 + 1e-6,
            "contact morph observations do not bracket {at} ms");
        let right = self.samples.partition_point(|sample| sample.0 < at);
        if right == 0 { return Ok(first.1.clone()); }
        if right == self.samples.len() { return Ok(last.1.clone()); }
        let a = &self.samples[right-1];
        let b = &self.samples[right];
        if at == b.0 { return Ok(b.1.clone()); }
        let fraction = ((at - a.0) / (b.0 - a.0)) as f32;
        Ok(a.1.iter().zip(&b.1).map(|(a,b)| a.iter().zip(b)
            .map(|(a,b)| *a + fraction * (*b - *a)).collect()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::{mesh::morph::{MorphAttributes, MorphTargetImage}, render_asset::RenderAssetUsages};

    fn image() -> Image {
        let first = vec![MorphAttributes::new(Vec3::X, Vec3::splat(99.), Vec3::splat(77.)),
            MorphAttributes::new(Vec3::Y * 2., Vec3::splat(88.), Vec3::splat(66.))];
        let second = vec![MorphAttributes::new(Vec3::Z * 3., Vec3::ZERO, Vec3::ZERO), MorphAttributes::default()];
        MorphTargetImage::new([first.into_iter(), second.into_iter()].into_iter(), 2, RenderAssetUsages::default()).unwrap().0
    }

    #[test]
    fn official_bevy_target_image_decodes_positions_in_layer_order() {
        let geometry = MorphGeometry::decode(&image(), 2).unwrap();
        assert_eq!(geometry.position_target_count(), 2);
        let base = [Vec3::new(10., 20., 30.), Vec3::new(-1., -2., -3.)];
        let points = geometry.apply(&base, &[0.5, -0.25]).unwrap();
        assert_eq!(points, [base[0] + Vec3::new(0.5, 0., -0.75), base[1] + Vec3::Y]);
        let skin = Mat4::from_rotation_translation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::splat(4.));
        assert!(skin.transform_point3(points[0]).abs_diff_eq(
            skin.transform_point3(base[0]) + skin.transform_vector3(Vec3::new(0.5, 0., -0.75)), 1e-5));
        assert_eq!(geometry.apply(&base, &[0., 0.]).unwrap(), base);
        geometry.validate_embedding(&[2].into_iter().collect()).unwrap();
        assert!(geometry.validate_embedding(&[0].into_iter().collect()).is_err());
    }

    #[test]
    fn malformed_morph_images_and_weights_fail_without_mutating_the_base() {
        let mut image = image();
        let geometry = MorphGeometry::decode(&image, 2).unwrap();
        let base = [Vec3::X, Vec3::Y];
        assert!(geometry.apply(&base, &[1.]).is_err());
        assert!(geometry.apply(&base, &[f32::NAN, 0.]).is_err());
        image.data.pop();
        assert!(MorphGeometry::decode(&image, 2).is_err());
        assert_eq!(base, [Vec3::X, Vec3::Y]);
    }

    #[test]
    fn morph_history_brackets_substeps_and_retains_the_uncommitted_remainder() {
        let mut history = MorphHistory::default();
        history.observe(50., &[vec![0., 1.]], 50.).unwrap();
        history.observe(66., &[vec![1., 0.]], 50.).unwrap();
        assert_eq!(history.sample(58.).unwrap(), [vec![0.5, 0.5]]);
        history.observe(83., &[vec![0., 0.]], 58.333).unwrap();
        assert_eq!(history.sample(58.).unwrap(), [vec![0.5, 0.5]]);
        assert!(history.sample(49.).is_err());
        assert!(history.sample(84.).is_err());
        history.observe(83., &[vec![0., 0.]], 75.).unwrap();
        assert!(history.observe(83., &[vec![1., 0.]], 75.).is_err());
        assert_eq!(history.sample(83.).unwrap(), [vec![0., 0.]]);

        let mut endpoints = MorphHistory::default();
        endpoints.observe(0., &[vec![1e10]], 0.).unwrap();
        endpoints.observe(10., &[vec![1.]], 0.).unwrap();
        assert_eq!(endpoints.sample(10.).unwrap(), [vec![1.]]);
    }
}
