//! Volume-weighted average nodal pressure. Each nodal control volume is one
//! quarter of its incident tetrahedra. Averaging J before the bulk penalty
//! avoids the excessive per-element incompressibility constraints of P1 tets.
//! See Joldes, Wittek and Miller, doi:10.1002/cnm.1185, equations 9-13.
use super::{material, solver::TissueError};
use bevy::math::{DMat3, DVec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(super) fn mean_ratio_quality(points: [DVec3; 4]) -> f64 {
    let six_volume = material::edges(points).determinant().abs();
    let edge_sum: f64 = material::EDGE_PAIRS.iter()
        .map(|[a, b]| (points[*a] - points[*b]).length_squared()).sum();
    12. * (six_volume / 2.).powf(2./3.) / edge_sum
}

pub(super) fn element_partition(
    tets: &[material::Tet], initial: &[DVec3],
    attachments: &[super::solver::Attachment], material: super::solver::Material,
) -> Vec<bool> {
    tets.iter().map(|tet| match material.pressure_discretization {
        PressureDiscretization::Element => true,
        PressureDiscretization::AverageNodal => false,
        PressureDiscretization::AverageNodalBoundary => tet.ids.iter()
            .filter(|&&i| matches!(attachments[i], super::solver::Attachment::Fixed)).count() >= 2
            || (material.element_pressure_quality_cutoff > 0.
                && mean_ratio_quality(tet.ids.map(|i| initial[i])) < material.element_pressure_quality_cutoff),
    }).collect()
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PressureDiscretization {
    #[default]
    Element,
    AverageNodal,
    /// Use element pressure in tetrahedra with a fixed edge, retaining
    /// local volume at the chest attachment while averaging the free interior.
    AverageNodalBoundary,
}

#[derive(Clone)]
pub(super) struct Patch {
    pub volume: f64,
    pub nodes: Vec<usize>,
    // Element index, local-to-patch node indices, normalized volume weight.
    elements: Vec<(usize, [usize; 4], f64)>,
    pub couplings: Vec<(usize, usize, usize)>,
}

pub(super) struct Evaluation {
    pub ratio: f64,
    pub gradients: Vec<DVec3>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Damping {
    pub history_log_volume: f64,
    pub bulk: f64,
}

impl Patch {
    pub fn build(tets: &[material::Tet], count: usize, element_pressure: &[bool]) -> Vec<Self> {
        let mut incident = vec![Vec::new(); count];
        for (e, tet) in tets.iter().enumerate() {
            if element_pressure[e] { continue; }
            for i in tet.ids { incident[i].push(e); }
        }
        incident.into_iter().filter(|elements| !elements.is_empty()).map(|elements| {
            let nodes: Vec<_> = elements.iter().flat_map(|e| tets[*e].ids)
                .collect::<BTreeSet<_>>().into_iter().collect();
            let volume = elements.iter().map(|e| tets[*e].volume / 4.).sum::<f64>();
            let elements = elements.into_iter().map(|e| {
                (e, tets[e].ids.map(|i| nodes.binary_search(&i).unwrap()), tets[e].volume / (4. * volume))
            }).collect();
            Self { volume, nodes, elements, couplings: Vec::new() }
        }).collect()
    }

    pub fn extend_edges(&self, edges: &mut BTreeSet<[usize; 2]>) {
        for (a, &i) in self.nodes.iter().enumerate() {
            for &j in &self.nodes[a + 1..] { edges.insert([i, j]); }
        }
    }

    pub fn index_edges(&mut self, edges: &[[usize; 2]]) -> Result<(), TissueError> {
        for a in 0..self.nodes.len() {
            for b in a + 1..self.nodes.len() {
                let index = edges.binary_search(&[self.nodes[a], self.nodes[b]])
                    .map_err(|_| TissueError::InvalidInput("pressure coupling"))?;
                self.couplings.push((index, a, b));
            }
        }
        Ok(())
    }

    pub fn ratio(&self, tets: &[material::Tet], x: &[DVec3]) -> f64 {
        self.elements.iter().map(|(e, _, w)| w * tets[*e].deformation(x).determinant()).sum()
    }

    pub fn evaluate(&self, tets: &[material::Tet], x: &[DVec3]) -> Evaluation {
        let mut gradients = vec![DVec3::ZERO; self.nodes.len()];
        let mut ratio = 0.;
        for &(e, local, weight) in &self.elements {
            let tet = &tets[e];
            let f = tet.deformation(x);
            ratio += weight * f.determinant();
            let cofactor = DMat3::from_cols(f.y_axis.cross(f.z_axis), f.z_axis.cross(f.x_axis), f.x_axis.cross(f.y_axis));
            let derivative = material::nodal_gradient(cofactor * tet.inverse_rest.transpose());
            for a in 0..4 { gradients[local[a]] += weight * derivative[a]; }
        }
        Evaluation { ratio, gradients }
    }

    pub fn energy(&self, tets: &[material::Tet], x: &[DVec3], lambda: f64, damping: Damping) -> f64 {
        let ratio = self.ratio(tets, x);
        self.volume * 0.5 * (lambda * (ratio - 1.).powi(2)
            + damping.bulk * (ratio.ln() - damping.history_log_volume).powi(2))
    }
}
