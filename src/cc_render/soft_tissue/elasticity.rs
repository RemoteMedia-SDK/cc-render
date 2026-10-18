//! Element elastic forces and PSD tangents, independent of matrix assembly.
use super::*;
use bevy::math::DMat3;

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

impl Tissue {
    pub(super) fn elastic_differential(&self, index: usize, x: &[DVec3], active: [bool; 4])
        -> ([DVec3; 4], [[DMat3; 4]; 4])
    {
        let tet = &self.tets[index];
        let isochoric = self.material.pressure_discretization != PressureDiscretization::Element;
        let (mut gradient, mut blocks) = if isochoric {
            tet.isochoric_differential(x, self.material.shear_pa, self.material.strain_stiffening, active)
        } else {
            corotated(tet, x, self.material.shear_pa, self.material.strain_stiffening, active)
        };
        if self.element_pressure[index] {
            let f = tet.deformation(x);
            let cofactor = DMat3::from_cols(f.y_axis.cross(f.z_axis), f.z_axis.cross(f.x_axis), f.x_axis.cross(f.y_axis));
            let derivative = material::nodal_gradient(cofactor * tet.inverse_rest.transpose());
            let coefficient = if isochoric { self.material.bulk_pa } else { self.material.lambda() } * tet.volume;
            for (gradient, derivative) in gradient.iter_mut().zip(derivative) {
                *gradient += coefficient * (f.determinant() - 1.) * derivative;
            }
            for i in (0..4).filter(|&i| active[i]) {
                for j in (0..4).filter(|&j| active[j]) {
                    blocks[i][j] += coefficient * outer(derivative[i], derivative[j]);
                }
            }
        }
        (gradient, blocks)
    }
}

fn corotated(tet: &Tet, x: &[DVec3], mu: f64, stiffening: f64, active: [bool; 4])
    -> ([DVec3; 4], [[DMat3; 4]; 4])
{
    let f = tet.deformation(x);
    let shape = material::nodal_gradient(tet.inverse_rest.transpose());
    let strain = f - material::polar_rotation(f);
    let strain_gradient = shape.map(|g| strain * g);
    let weight = 2. * mu * tet.volume * (1. + stiffening * material::squared_norm(strain));
    let hardening = 4. * mu * tet.volume * stiffening;
    let gradient = strain_gradient.map(|g| weight * g);
    let blocks = std::array::from_fn(|i| std::array::from_fn(|j| {
        if active[i] && active[j] {
            DMat3::IDENTITY * (weight * shape[i].dot(shape[j]))
                + hardening * outer(strain_gradient[i], strain_gradient[j])
        } else { DMat3::ZERO }
    }));
    (gradient, blocks)
}
