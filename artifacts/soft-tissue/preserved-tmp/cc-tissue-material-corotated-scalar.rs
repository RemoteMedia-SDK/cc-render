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

    pub fn constraints(&self, positions: &[DVec3]) -> Constraints {
        let f = self.deformation(positions);
        let strain = f - polar_rotation(f);
        let length = squared_norm(strain).sqrt();
        let cofactor = DMat3::from_cols(
            f.y_axis.cross(f.z_axis),
            f.z_axis.cross(f.x_axis),
            f.x_axis.cross(f.y_axis),
        );
        let rest_t = self.inverse_rest.transpose();
        Constraints {
            values: [length, f.determinant() - 1.0],
            gradients: [
                nodal_gradient(if length > 1e-12 {
                    strain * rest_t / length
                } else {
                    DMat3::ZERO
                }),
                nodal_gradient(cofactor * rest_t),
            ],
        }
    }

    pub fn energy(&self, positions: &[DVec3], mu: f64, lambda: f64) -> f64 {
        let f = self.deformation(positions);
        let j = f.determinant();
        self.volume * (mu * squared_norm(f - polar_rotation(f)) + 0.5 * lambda * (j - 1.0).powi(2))
    }
}

/// Coupled XPBD normal equations. Compliance includes the material volume and
/// h^2 scaling. Multipliers persist across iterations, reset each substep.
pub(super) fn project(
    tet: &Tet,
    positions: &mut [DVec3],
    inverse_mass: &[f64],
    compliance: [f64; 2],
    multipliers: &mut [f64; 2],
) {
    let c = tet.constraints(positions);
    let inner = |a: usize, b: usize| -> f64 {
        (0..4)
            .map(|k| inverse_mass[tet.ids[k]] * c.gradients[a][k].dot(c.gradients[b][k]))
            .sum()
    };
    let a = inner(0, 0) + compliance[0];
    let b = inner(0, 1);
    let d = inner(1, 1) + compliance[1];
    let residual = [
        c.values[0] + compliance[0] * multipliers[0],
        c.values[1] + compliance[1] * multipliers[1],
    ];
    let determinant = a * d - b * b;
    let dl = [
        (-d * residual[0] + b * residual[1]) / determinant,
        (b * residual[0] - a * residual[1]) / determinant,
    ];
    for k in 0..4 {
        positions[tet.ids[k]] +=
            inverse_mass[tet.ids[k]] * (c.gradients[0][k] * dl[0] + c.gradients[1][k] * dl[1]);
    }
    multipliers[0] += dl[0];
    multipliers[1] += dl[1];
}
