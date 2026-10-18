// Isolated experiment: block incomplete LDL^T on the existing edge pattern.
// A shift changes only the preconditioner, never the matrix being solved.
static IC_STATS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];

struct IcEntry {
    row: usize,
    col: usize,
    source: usize,
    common: Vec<[usize; 2]>,
}
struct IcPattern {
    signature: Vec<[usize; 2]>,
    rows: Vec<usize>,
    entries: Vec<IcEntry>,
}
impl IcPattern {
    fn new(system: &System) -> Self {
        let signature: Vec<_> = system.couplings.iter().map(|c| c.nodes).collect();
        let mut entries: Vec<_> = signature.iter().enumerate().map(|(source, [a, b])| {
            assert!(a < b);
            IcEntry { row: *b, col: *a, source, common: Vec::new() }
        }).collect();
        entries.sort_by_key(|e| (e.row, e.col));
        let mut rows = vec![0; system.diagonal.len() + 1];
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
                    std::cmp::Ordering::Equal => {
                        common.push([a, b]); a += 1; b += 1;
                    }
                }
            }
            entries[p].common = common;
        }
        Self { signature, rows, entries }
    }
}
thread_local! {
    static IC_PATTERN: std::cell::RefCell<Option<std::sync::Arc<IcPattern>>> = const { std::cell::RefCell::new(None) };
}

struct IcFactor {
    pattern: std::sync::Arc<IcPattern>,
    lower: Vec<DMat3>,
    inverse: Vec<DMat3>,
}
impl IcFactor {
    fn new(system: &System) -> Option<Self> {
        let pattern = IC_PATTERN.with(|cache| {
            let mut cache = cache.borrow_mut();
            let matches = cache.as_ref().is_some_and(|p| {
                p.rows.len() == system.diagonal.len() + 1 &&
                p.signature.iter().copied().eq(system.couplings.iter().map(|c| c.nodes))
            });
            if !matches { *cache = Some(std::sync::Arc::new(IcPattern::new(system))); }
            cache.as_ref().unwrap().clone()
        });
        for (attempt, shift) in [0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8].into_iter().enumerate() {
            if let Some(factor) = Self::factor(system, pattern.clone(), shift) {
                IC_STATS[attempt].fetch_add(1, Ordering::Relaxed);
                return Some(factor);
            }
        }
        IC_STATS[8].fetch_add(1, Ordering::Relaxed);
        None
    }

    fn factor(system: &System, pattern: std::sync::Arc<IcPattern>, shift: f64) -> Option<Self> {
        let mut lower = vec![DMat3::ZERO; pattern.entries.len()];
        let mut weighted = lower.clone();
        let mut inverse = vec![DMat3::ZERO; system.diagonal.len()];
        for i in 0..system.diagonal.len() {
            if system.diagonal[i] == DMat3::ZERO { continue; }
            let mut d = system.diagonal[i] * (1.0 + shift);
            for p in pattern.rows[i]..pattern.rows[i + 1] {
                let entry = &pattern.entries[p];
                let mut s = system.couplings[entry.source].block.transpose();
                for [a, b] in &entry.common { s -= weighted[*a] * lower[*b].transpose(); }
                let l = s * inverse[entry.col];
                d -= l * s.transpose();
                weighted[p] = s;
                lower[p] = l;
            }
            d = (d + d.transpose()) * 0.5;
            let scale = system.diagonal[i].x_axis.x.max(system.diagonal[i].y_axis.y).max(system.diagonal[i].z_axis.z);
            if !ic_positive(d, scale * 1e-12) { return None; }
            inverse[i] = d.inverse();
            if !inverse[i].is_finite() { return None; }
        }
        Some(Self { pattern, lower, inverse })
    }

    fn apply(&self, rhs: &[DVec3]) -> Vec<DVec3> {
        let mut result = rhs.to_vec();
        for i in 0..result.len() {
            let mut value = result[i];
            for p in self.pattern.rows[i]..self.pattern.rows[i + 1] {
                value -= self.lower[p] * result[self.pattern.entries[p].col];
            }
            result[i] = value;
        }
        for (v, d) in result.iter_mut().zip(&self.inverse) { *v = *d * *v; }
        for i in (0..result.len()).rev() {
            let value = result[i];
            for p in self.pattern.rows[i]..self.pattern.rows[i + 1] {
                result[self.pattern.entries[p].col] -= self.lower[p].transpose() * value;
            }
        }
        result
    }
}

fn ic_positive(d: DMat3, floor: f64) -> bool {
    if !d.is_finite() || d.x_axis.x <= floor { return false; }
    let first = d.x_axis.x.sqrt();
    let b = d.x_axis.y / first;
    let c = d.x_axis.z / first;
    let second = d.y_axis.y - b * b;
    if second <= floor { return false; }
    let e = (d.y_axis.z - b * c) / second.sqrt();
    d.z_axis.z - c * c - e * e > floor
}
