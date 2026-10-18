//! Deviatoric corotated energy for nodal pressure: use Fbar = J^(-1/3) F.
//! Separating pressure requires separating the shear energy too. Unlike the
//! unsplit corotated law, this resists a collapsing, highly distorted element.
use super::material::{self, Tet};
use bevy::math::{DMat3, DVec3};

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::DQuat;

    #[test]
    fn deviatoric_energy_ignores_uniform_dilation_but_resists_distorted_collapse() {
        let tet = Tet { ids: [0, 1, 2, 3], inverse_rest: DMat3::IDENTITY,
            volume: 1./6., edge_indices: [0; 6] };
        let rest = [DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z];
        let rotation = DQuat::from_rotation_x(0.7);
        for scale in [0.1, 0.8, 1., 2.] {
            let x = rest.map(|p| scale * (rotation * p) + DVec3::new(2., 1., -1.));
            assert!(tet.isochoric_energy(&x, 1500., 8.) < 1e-20);
        }
        let mut last = 0.;
        for compression in [0.5, 0.1, 0.01, 0.001] {
            let x = rest.map(|p| p * DVec3::new(1., compression, 1.));
            let energy = tet.isochoric_energy(&x, 1500., 8.);
            assert!(energy > last * 2.);
            last = energy;
        }
    }
}

impl Tet {
    pub fn isochoric_energy(&self, x: &[DVec3], mu: f64, stiffening: f64) -> f64 {
        let f = self.deformation(x);
        let normalized = f * f.determinant().powf(-1. / 3.);
        let q = material::squared_norm(normalized - material::polar_rotation(f));
        self.volume * mu * (q + 0.5 * stiffening * q * q)
    }

    /// Exact force and PSD tangent with rotation held fixed in the tangent,
    /// matching the existing corotated solver's local/global approximation.
    /// Tangent blocks touching inactive (fixed) nodes are omitted.
    pub fn isochoric_differential(&self, x: &[DVec3], mu: f64, stiffening: f64, active: [bool; 4])
        -> ([DVec3; 4], [[DMat3; 4]; 4])
    {
        let f = self.deformation(x);
        let scale = f.determinant().powf(-1. / 3.);
        let normalized = f * scale;
        let strain = normalized - material::polar_rotation(f);
        let q = material::squared_norm(strain);
        let contraction = strain.x_axis.dot(normalized.x_axis)
            + strain.y_axis.dot(normalized.y_axis) + strain.z_axis.dot(normalized.z_axis);
        let shape = material::nodal_gradient(self.inverse_rest.transpose());
        let inverse = f.inverse().transpose();
        let volume_grad = shape.map(|g| inverse * g);
        let mapped = shape.map(|g| f * g);
        let strain_grad: [DVec3; 4] = std::array::from_fn(|i|
            scale * strain * shape[i] - contraction / 3. * volume_grad[i]);
        let weight = 2. * mu * self.volume * (1. + stiffening * q);
        let hardening = 4. * mu * self.volume * stiffening;
        let gradient = strain_grad.map(|g| weight * g);
        let mut blocks = [[DMat3::ZERO; 4]; 4];
        for i in 0..4 {
            if !active[i] { continue; }
            for j in i..4 {
                if !active[j] { continue; }
                let metric = DMat3::IDENTITY * shape[i].dot(shape[j])
                    - (outer(mapped[i], volume_grad[j]) + outer(volume_grad[i], mapped[j])) / 3.
                    + material::squared_norm(f) / 9. * outer(volume_grad[i], volume_grad[j]);
                blocks[i][j] = weight * scale * scale * metric
                    + hardening * outer(strain_grad[i], strain_grad[j]);
                blocks[j][i] = blocks[i][j].transpose();
            }
        }
        (gradient, blocks)
    }
}
