//! Experimental complete block LDL^T with deterministic minimum-degree ordering.
//! Used only in the isolated profiling copy until independently validated.
use bevy::math::{DMat3, DVec3};

struct Entry {
    row: usize,
    col: usize,
    source: Option<usize>,
    transpose: bool,
    common: Vec<[usize; 2]>,
}
pub(super) struct Pattern {
    rows: Vec<usize>,
    entries: Vec<Entry>,
    order: Vec<usize>,
}
impl Pattern {
    pub fn new(count: usize, edges: impl Iterator<Item = [usize; 2]>) -> Self {
        use std::collections::{BTreeMap, BTreeSet};
        let original: BTreeMap<_, _> = edges.enumerate().map(|(i, pair)| (pair, i)).collect();
        let mut graph = vec![BTreeSet::new(); count];
        for [a, b] in original.keys() {
            assert!(a < b);
            graph[*a].insert(*b);
            graph[*b].insert(*a);
        }
        let mut remaining = vec![true; count];
        let mut order = Vec::with_capacity(count);
        let mut filled = BTreeSet::new();
        for _ in 0..count {
            let pivot = (0..count).filter(|i| remaining[*i])
                .min_by_key(|i| (graph[*i].len(), *i)).unwrap();
            let neighbors: Vec<_> = graph[pivot].iter().copied().collect();
            for (offset, a) in neighbors.iter().enumerate() {
                filled.insert([pivot.min(*a), pivot.max(*a)]);
                graph[*a].remove(&pivot);
                for b in &neighbors[offset + 1..] {
                    graph[*a].insert(*b);
                    graph[*b].insert(*a);
                }
            }
            graph[pivot].clear();
            remaining[pivot] = false;
            order.push(pivot);
        }
        let mut inverse_order = vec![0; count];
        for (i, old) in order.iter().enumerate() { inverse_order[*old] = i; }
        let mut entries: Vec<_> = filled.into_iter().map(|[a, b]| {
            let (i, j) = (inverse_order[a], inverse_order[b]);
            Entry { row: i.max(j), col: i.min(j), source: original.get(&[a,b]).copied(),
                transpose: j > i, common: Vec::new() }
        }).collect();
        entries.sort_by_key(|e| (e.row, e.col));
        let mut rows = vec![0; count + 1];
        for entry in &entries { rows[entry.row + 1] += 1; }
        for i in 1..rows.len() { rows[i] += rows[i - 1]; }
        for p in 0..entries.len() {
            let (i, j) = (entries[p].row, entries[p].col);
            let (mut a, mut b) = (rows[i], rows[j]);
            let mut common = Vec::new();
            while a < p && b < rows[j + 1] {
                match entries[a].col.cmp(&entries[b].col) {
                    std::cmp::Ordering::Less => a += 1,
                    std::cmp::Ordering::Greater => b += 1,
                    std::cmp::Ordering::Equal => { common.push([a,b]); a += 1; b += 1; }
                }
            }
            entries[p].common = common;
        }
        Self { rows, entries, order }
    }
}
pub(super) struct Factor<'a> {
    pattern: &'a Pattern,
    lower: Vec<DMat3>,
    inverse: Vec<DMat3>,
}
impl<'a> Factor<'a> {
    pub fn new(
        pattern: &'a Pattern,
        diagonal: &[DMat3],
        block: impl Fn(usize) -> DMat3,
    ) -> Option<Self> {
        // 0.1 avoids repeated failed attempts for the nearly incompressible
        // cage. Doubling retries remain bounded; failure uses block Jacobi.
        for shift in [0.0, 0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8] {
            if let Some(factor) = Self::factor(pattern, diagonal, &block, shift) {
                return Some(factor);
            }
        }
        None
    }

    fn factor(
        pattern: &'a Pattern,
        diagonal: &[DMat3],
        block: &impl Fn(usize) -> DMat3,
        shift: f64,
    ) -> Option<Self> {
        let mut lower = vec![DMat3::ZERO; pattern.entries.len()];
        let mut weighted = lower.clone();
        let mut inverse = vec![DMat3::ZERO; diagonal.len()];
        for i in 0..diagonal.len() {
            let old = pattern.order[i];
            if diagonal[old] == DMat3::ZERO {
                continue;
            }
            let mut d = diagonal[old] * (1.0 + shift);
            for p in pattern.rows[i]..pattern.rows[i + 1] {
                let entry = &pattern.entries[p];
                let mut s = entry.source.map(&block).unwrap_or(DMat3::ZERO);
                if entry.transpose { s = s.transpose(); }
                for [a, b] in &entry.common {
                    s -= weighted[*a] * lower[*b].transpose();
                }
                let l = s * inverse[entry.col];
                d -= l * s.transpose();
                weighted[p] = s;
                lower[p] = l;
            }
            d = (d + d.transpose()) * 0.5;
            let scale = diagonal[old]
                .x_axis
                .x
                .max(diagonal[old].y_axis.y)
                .max(diagonal[old].z_axis.z);
            if !ic_positive(d, scale * 1e-12) {
                return None;
            }
            inverse[i] = d.inverse();
            if !inverse[i].is_finite() {
                return None;
            }
        }
        Some(Self {
            pattern,
            lower,
            inverse,
        })
    }

    pub fn apply(&self, rhs: &[DVec3]) -> Vec<DVec3> {
        let mut result: Vec<_> = self.pattern.order.iter().map(|i| rhs[*i]).collect();
        for i in 0..result.len() {
            let mut value = result[i];
            for p in self.pattern.rows[i]..self.pattern.rows[i + 1] {
                value -= self.lower[p] * result[self.pattern.entries[p].col];
            }
            result[i] = value;
        }
        for (v, d) in result.iter_mut().zip(&self.inverse) {
            *v = *d * *v;
        }
        for i in (0..result.len()).rev() {
            let value = result[i];
            for p in self.pattern.rows[i]..self.pattern.rows[i + 1] {
                result[self.pattern.entries[p].col] -= self.lower[p].transpose() * value;
            }
        }
        let mut output = vec![DVec3::ZERO; result.len()];
        for (value, old) in result.into_iter().zip(&self.pattern.order) { output[*old] = value; }
        output
    }

}

fn ic_positive(d: DMat3, floor: f64) -> bool {
    if !d.is_finite() || d.x_axis.x <= floor {
        return false;
    }
    let first = d.x_axis.x.sqrt();
    let b = d.x_axis.y / first;
    let c = d.x_axis.z / first;
    let second = d.y_axis.y - b * b;
    if second <= floor {
        return false;
    }
    let e = (d.y_axis.z - b * c) / second.sqrt();
    d.z_axis.z - c * c - e * e > floor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_factor_solves_dense_system_with_nonsymmetric_couplings_and_fixed_node() {
        // Three free blocks form a complete pattern. Node 1 is eliminated.
        // The block diagonally dominant SPD matrix has a known manufactured
        // solution; its dense product is independent of the factor traversal.
        let edges = [[0, 2], [0, 3], [2, 3]];
        let pattern = Pattern::new(4, edges.into_iter());
        let blocks = [
            DMat3::from_cols_array(&[0.2, 0.1, -0.3, 0.4, -0.1, 0.2, 0.1, 0.3, 0.2]),
            DMat3::from_cols_array(&[-0.1, 0.3, 0.2, 0.1, 0.2, 0.3, -0.2, 0.1, 0.1]),
            DMat3::from_cols_array(&[0.3, -0.2, 0.1, 0.2, 0.1, -0.1, 0.3, 0.1, 0.2]),
        ];
        let diagonal = [
            DMat3::IDENTITY * 3.0,
            DMat3::ZERO,
            DMat3::IDENTITY * 4.0,
            DMat3::IDENTITY * 5.0,
        ];
        let expected = [
            DVec3::new(0.2, -0.3, 0.7),
            DVec3::ZERO,
            DVec3::new(-0.5, 0.1, 0.4),
            DVec3::new(0.8, 0.3, -0.2),
        ];
        let rhs = [
            diagonal[0] * expected[0] + blocks[0] * expected[2] + blocks[1] * expected[3],
            DVec3::ZERO,
            blocks[0].transpose() * expected[0]
                + diagonal[2] * expected[2]
                + blocks[2] * expected[3],
            blocks[1].transpose() * expected[0]
                + blocks[2].transpose() * expected[2]
                + diagonal[3] * expected[3],
        ];
        let factor = Factor::factor(&pattern, &diagonal, &|i| blocks[i], 0.0).unwrap();
        for (actual, expected) in factor.apply(&rhs).iter().zip(expected) {
            assert!(actual.distance(expected) < 1e-12);
        }
    }

    #[test]
    fn incomplete_factor_is_symmetric_positive_and_rejects_bad_pivots() {
        let pattern = Pattern::new(4, [[0, 1], [0, 3], [1, 2], [2, 3]].into_iter());
        let diagonal = [DMat3::IDENTITY * 3.0; 4];
        let factor = Factor::new(&pattern, &diagonal, |_| DMat3::IDENTITY * -0.7).unwrap();
        let a = [DVec3::X, DVec3::Y, DVec3::Z, DVec3::ONE];
        let b = [DVec3::ONE, DVec3::X, -DVec3::Y, DVec3::Z];
        let dot = |x: &[DVec3], y: &[DVec3]| x.iter().zip(y).map(|(a, b)| a.dot(*b)).sum::<f64>();
        let ma = factor.apply(&a);
        let mb = factor.apply(&b);
        assert!(dot(&a, &ma) > 0.0);
        assert!((dot(&a, &mb) - dot(&b, &ma)).abs() < 1e-12);
        let invalid = DMat3::from_cols_array(&[1.0, 2.0, 0.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert!(!ic_positive(invalid, 1e-12));
        let singleton = Pattern::new(1, [].into_iter());
        assert!(Factor::new(&singleton, &[invalid], |_| unreachable!()).is_none());
    }
}
