//! Co-rotated elastic strain plus a finite-volume energy.
//! The polar rotation follows the element, so rigid rotation has zero energy.
//! Rotation-dependent strain is described in Sifakis et al.'s technical notes:
//! https://pages.cs.wisc.edu/~sifakis/papers/elasticity_skinning_technical_notes.pdf
use bevy::math::{DMat3, DVec3};

pub(super) const EDGE_PAIRS: [[usize; 2]; 6] = [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]];

#[derive(Clone, Debug)]
pub(super) struct Tet {
    pub ids: [usize; 4],
    pub inverse_rest: DMat3,
    pub volume: f64,
    /// Indices into the immutable global edge order, following EDGE_PAIRS.
    pub edge_indices: [usize; 6],
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

pub(super) fn squared_norm(matrix: DMat3) -> f64 {
    matrix.x_axis.length_squared() + matrix.y_axis.length_squared() + matrix.z_axis.length_squared()
}

pub(super) fn polar_rotation(mut matrix: DMat3) -> DMat3 {
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

    pub fn energy(&self, positions: &[DVec3], mu: f64, lambda: f64, stiffening: f64) -> f64 {
        let f = self.deformation(positions);
        let j = f.determinant();
        let strain = squared_norm(f - polar_rotation(f));
        self.volume * (mu * (strain + 0.5 * stiffening * strain * strain)
            + 0.5 * lambda * (j - 1.0).powi(2))
    }
}
