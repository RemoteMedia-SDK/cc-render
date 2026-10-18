//! Co-rotated elastic strain plus a finite-volume constraint, solved with XPBD.
//! The polar rotation follows the element, so rigid rotation has zero energy.
//! Rotation-dependent strain is described in Sifakis et al.'s technical notes:
//! https://pages.cs.wisc.edu/~sifakis/papers/elasticity_skinning_technical_notes.pdf
use bevy::math::{DMat3, DVec3};

#[derive(Clone, Debug)]
pub(super) struct Tet {
    pub ids: [usize; 4],
    pub inverse_rest: DMat3,
    pub volume: f64,
}

pub(super) fn edges(points: [DVec3; 4]) -> DMat3 {
    DMat3::from_cols(
        points[1] - points[0],
        points[2] - points[0],
        points[3] - points[0],
    )
}

pub(super) fn nodal_gradient(matrix: DMat3) -> [DVec3; 4] {
    [
        -(matrix.x_axis + matrix.y_axis + matrix.z_axis),
        matrix.x_axis,
        matrix.y_axis,
        matrix.z_axis,
    ]
}

#[cfg(test)]
pub(super) struct Constraints {
    pub values: [f64; 2],
    pub gradients: [[DVec3; 4]; 2],
}

fn squared_norm(matrix: DMat3) -> f64 {
    matrix.x_axis.length_squared() + matrix.y_axis.length_squared() + matrix.z_axis.length_squared()
}

fn polar_rotation(mut matrix: DMat3) -> DMat3 {
    for _ in 0..12 {
        let next = (matrix + matrix.inverse().transpose()) * 0.5;
        let change = squared_norm(next - matrix);
        matrix = next;
        if change < 1e-24 {
            break;
        }
    }
    matrix
}

impl Tet {
    pub fn deformation(&self, positions: &[DVec3]) -> DMat3 {
        edges(self.ids.map(|i| positions[i])) * self.inverse_rest
    }

    #[cfg(test)]
    pub fn constraints(&self, positions: &[DVec3]) -> Constraints {
        let f = self.deformation(positions);
        let strain = f - polar_rotation(f);
        let length = squared_norm(strain).sqrt();
        let (volume, volume_gradient) = self.volume_constraint(positions);
        Constraints {
            values: [length, volume],
            gradients: [
                nodal_gradient(if length > 1e-12 {
                    strain * self.inverse_rest.transpose() / length
                } else {
                    DMat3::ZERO
                }),
                volume_gradient,
            ],
        }
    }

    fn volume_constraint(&self, positions: &[DVec3]) -> (f64, [DVec3; 4]) {
        let f = self.deformation(positions);
        let cofactor = DMat3::from_cols(
            f.y_axis.cross(f.z_axis),
            f.z_axis.cross(f.x_axis),
            f.x_axis.cross(f.y_axis),
        );
        (
            f.determinant() - 1.0,
            nodal_gradient(cofactor * self.inverse_rest.transpose()),
        )
    }

    pub fn energy(&self, positions: &[DVec3], mu: f64, lambda: f64) -> f64 {
        let f = self.deformation(positions);
        let j = f.determinant();
        self.volume * (mu * squared_norm(f - polar_rotation(f)) + 0.5 * lambda * (j - 1.0).powi(2))
    }
}

/// Local/global co-rotated projection. Keeping the nine strain components
/// separate avoids the ill-conditioned gradient of a scalar strain norm near
/// equilibrium. Rotation is refreshed each pass; multipliers last one substep.
pub(super) fn project(
    tet: &Tet,
    positions: &mut [DVec3],
    inverse_mass: &[f64],
    compliance: [f64; 2],
    multipliers: &mut [f64; 10],
) {
    let rotation = polar_rotation(tet.deformation(positions));
    for column in 0..3 {
        let shape = tet.inverse_rest.col(column);
        let weights = [-(shape.x + shape.y + shape.z), shape.x, shape.y, shape.z];
        // The three Cartesian rows are independent with identical diagonal
        // mass, so solve them together without changing the scalar XPBD solve.
        let value: DVec3 = (0..4)
            .map(|k| positions[tet.ids[k]] * weights[k])
            .sum::<DVec3>()
            - rotation.col(column);
        let w: f64 = (0..4)
            .map(|k| inverse_mass[tet.ids[k]] * weights[k].powi(2))
            .sum();
        let offset = column * 3;
        let mut multiplier = DVec3::from_slice(&multipliers[offset..offset + 3]);
        let delta = -(value + compliance[0] * multiplier) / (w + compliance[0]);
        for k in 0..4 {
            positions[tet.ids[k]] += inverse_mass[tet.ids[k]] * weights[k] * delta;
        }
        multiplier += delta;
        multipliers[offset..offset + 3].copy_from_slice(&multiplier.to_array());
    }
    let (volume, gradients) = tet.volume_constraint(positions);
    project_scalar(
        tet.ids,
        positions,
        inverse_mass,
        volume,
        gradients,
        compliance[1],
        &mut multipliers[9],
    );
}

fn project_scalar(
    ids: [usize; 4],
    positions: &mut [DVec3],
    inverse_mass: &[f64],
    value: f64,
    gradients: [DVec3; 4],
    alpha: f64,
    multiplier: &mut f64,
) {
    let w: f64 = (0..4)
        .map(|k| inverse_mass[ids[k]] * gradients[k].length_squared())
        .sum();
    let delta = -(value + alpha * *multiplier) / (w + alpha);
    for k in 0..4 {
        positions[ids[k]] += inverse_mass[ids[k]] * gradients[k] * delta;
    }
    *multiplier += delta;
}
