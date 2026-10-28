//! Vertex normals of a reshaped mesh.
//!
//! A shape delta moves positions only; the character's normals are CC5's, authored
//! for the captured shape. They are carried along with the surface: per vertex, the
//! rotation that takes the bind mesh's smooth normal to the shaped mesh's smooth
//! normal turns the authored normal (and tangent). Smooth normals are area-weighted
//! face normals accumulated per *global* vertex (CC5 vertex index), so seam-split
//! glTF copies of one CC5 vertex get the same rotation and seams stay closed.
//! Where the surface did not move the rotation is the identity and the authored
//! normal is kept bit for bit. A twist of the surface about its own normal is not
//! seen (the arc rotation has none); it does not change the normal.

use glam::{Quat, Vec3};

/// Area-weighted smooth normals (unnormalized) per global vertex of a surface made of
/// several primitives: `(global id per vertex, positions, triangle list indices)`.
/// `count` is the size of the global vertex space.
pub fn welded_normals<'a>(
    count: usize,
    prims: impl IntoIterator<Item = (&'a [u32], &'a [[f32; 3]], &'a [u32])>,
) -> Vec<Vec3> {
    let mut acc = vec![Vec3::ZERO; count];
    for (global, pos, tris) in prims {
        for t in tris.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(pos[i as usize]));
            let n = (b - a).cross(c - a); // length = twice the triangle's area
            for &i in t {
                acc[global[i as usize] as usize] += n;
            }
        }
    }
    acc
}

/// Turn each vertex's authored normal (and tangent, `w` kept) by the rotation from its
/// bind smooth normal to its shaped smooth normal (both from [`welded_normals`]).
pub fn reorient(
    bind: &[Vec3],
    shaped: &[Vec3],
    global: &[u32],
    normals: &mut [[f32; 3]],
    mut tangents: Option<&mut [[f32; 4]]>,
) {
    for (v, &g) in global.iter().enumerate() {
        let (a, b) = (bind[g as usize], shaped[g as usize]);
        if a == b {
            continue;
        }
        let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
        if a == Vec3::ZERO || b == Vec3::ZERO {
            continue;
        }
        let q = Quat::from_rotation_arc(a, b);
        normals[v] = (q * Vec3::from(normals[v])).normalize_or_zero().into();
        if let Some(t) = tangents.as_deref_mut() {
            let r = q * Vec3::new(t[v][0], t[v][1], t[v][2]);
            t[v] = [r.x, r.y, r.z, t[v][3]];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit quad in the XY plane as two primitives sharing the diagonal; vertices 1
    /// and 3 seam-split (each primitive has its own copy).
    fn quad() -> (Vec<[u32; 3]>, Vec<Vec<[f32; 3]>>) {
        let p = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        (vec![[0, 1, 2], [0, 2, 3]], vec![vec![p[0], p[1], p[2]], vec![p[0], p[2], p[3]]])
    }

    fn smooth(pos: &[Vec<[f32; 3]>]) -> Vec<Vec3> {
        let (glob, _) = quad();
        welded_normals(4, glob.iter().zip(pos).map(|(g, p)| (&g[..], &p[..], &[0u32, 1, 2][..])))
    }

    #[test]
    fn unchanged_surface_keeps_authored_normals_exactly() {
        let (glob, pos) = quad();
        let s = smooth(&pos);
        let authored = [[0.1f32, 0.2, 0.97], [0.0, 0.0, 1.0], [0.3, 0.0, 0.95]];
        let mut n = authored;
        let mut t = [[1.0f32, 0.0, 0.0, -1.0]; 3];
        reorient(&s, &s, &glob[0], &mut n, Some(&mut t));
        assert_eq!(n, authored);
        assert_eq!(t, [[1.0, 0.0, 0.0, -1.0]; 3]);
    }

    #[test]
    fn rotated_surface_turns_normals_and_tangents() {
        let (glob, pos) = quad();
        // Rotate the quad 90° about X: +Z normals become -Y.
        let r = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let moved: Vec<Vec<[f32; 3]>> = pos.iter().map(|p| p.iter().map(|v| (r * Vec3::from(*v)).into()).collect()).collect();
        let (a, b) = (smooth(&pos), smooth(&moved));
        let authored = [0.0f32, 0.6, 0.8];
        let mut n = [authored; 3];
        let mut t = [[1.0f32, 0.0, 0.0, 1.0]; 3];
        reorient(&a, &b, &glob[1], &mut n, Some(&mut t));
        let want = r * Vec3::from(authored);
        for v in 0..3 {
            assert!(Vec3::from(n[v]).abs_diff_eq(want, 1e-6), "{:?} vs {want:?}", n[v]);
            assert!(Vec3::new(t[v][0], t[v][1], t[v][2]).abs_diff_eq(Vec3::X, 1e-6) && t[v][3] == 1.0, "{:?}", t[v]);
        }
    }

    #[test]
    fn seam_copies_share_the_welded_normal() {
        let (_, mut pos) = quad();
        // Lift vertex 2 (shared by both primitives) in z: both triangles tilt.
        pos[0][2][2] = 0.5;
        pos[1][1][2] = 0.5;
        let s = smooth(&pos);
        // Vertex 0 and 2 belong to both triangles: their normal is the sum of both
        // faces, not just the face of one primitive.
        let f0 = (Vec3::from(pos[0][1]) - Vec3::from(pos[0][0])).cross(Vec3::from(pos[0][2]) - Vec3::from(pos[0][0]));
        let f1 = (Vec3::from(pos[1][1]) - Vec3::from(pos[1][0])).cross(Vec3::from(pos[1][2]) - Vec3::from(pos[1][0]));
        assert!(s[0].abs_diff_eq(f0 + f1, 1e-6) && s[2].abs_diff_eq(f0 + f1, 1e-6));
        assert!(s[1].abs_diff_eq(f0, 1e-6) && s[3].abs_diff_eq(f1, 1e-6));
    }
}
