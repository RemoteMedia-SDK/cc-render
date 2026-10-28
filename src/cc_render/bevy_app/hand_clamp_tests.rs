use super::*;

/// A closed unit cube, outward normals, no open edges.
fn cube() -> Vec<Face> {
    let corner = |i: usize| DVec3::new((i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64);
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    quads.iter().flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]).map(|t| {
        let points = t.map(corner);
        Face { points, normal: (points[1] - points[0]).cross(points[2] - points[0]).normalize(), open_edges: [false; 3] }
    }).collect()
}

#[test]
fn cube_faces_are_outward_and_signed_distance_has_the_right_sign() {
    let faces = cube();
    for f in &faces {
        let centre = f.points.iter().sum::<DVec3>() / 3.0;
        assert!(f.normal.dot(centre - DVec3::splat(0.5)) > 0.0);
    }
    let (inside, _) = signed_distance(DVec3::new(0.5, 0.5, 0.97), &faces).unwrap();
    assert!((inside + 0.03).abs() < 1e-12);
    let (outside, normal) = signed_distance(DVec3::new(0.5, 0.5, 1.2), &faces).unwrap();
    assert!((outside - 0.2).abs() < 1e-12 && normal.distance(DVec3::Z) < 1e-12);
    // Near an edge the pseudo-normal of both faces still gives the right sign.
    let (corner, _) = signed_distance(DVec3::new(0.5, 0.99, 0.99), &faces).unwrap();
    assert!(corner < 0.0);
}

#[test]
fn points_beside_an_open_surface_are_not_inside() {
    // A single upward square: below it but beyond its open edge is outside.
    let faces: Vec<Face> = [[[0., 0., 0.], [1., 0., 0.], [1., 1., 0.]], [[0., 0., 0.], [1., 1., 0.], [0., 1., 0.]]]
        .map(|t| t.map(DVec3::from)).into_iter().zip([[true, true, false], [false, true, true]])
        .map(|(points, open_edges)| Face { points, normal: DVec3::Z, open_edges }).collect();
    assert!(signed_distance(DVec3::new(0.5, 0.5, -0.02), &faces).unwrap().0 < 0.0);
    assert!(signed_distance(DVec3::new(1.3, 0.5, -0.02), &faces).is_none());
}

#[test]
fn push_out_moves_a_rigid_point_set_just_outside_and_leaves_clear_sets_alone() {
    let faces = cube();
    let hand: Vec<DVec3> = (0..5).map(|i| DVec3::new(0.3 + 0.1 * i as f64, 0.5, 0.98 + 0.004 * i as f64)).collect();
    let push = push_out(&hand, &faces, 0.001);
    assert!((push.penetration - 0.02).abs() < 1e-12);
    assert!(push.delta.distance(DVec3::Z * 0.021) < 1e-9, "{:?}", push.delta);
    for p in &hand {
        assert!(signed_distance(*p + push.delta, &faces).unwrap().0 >= 0.001 - 1e-9);
    }
    let clear: Vec<DVec3> = hand.iter().map(|p| *p + DVec3::Z * 0.1).collect();
    let none = push_out(&clear, &faces, 0.001);
    assert_eq!(none.delta, DVec3::ZERO);
    assert_eq!(none.penetration, 0.0);
}

#[test]
fn two_bone_ik_reaches_the_target_and_keeps_the_bend_plane() {
    let (shoulder, elbow, wrist) = (DVec3::ZERO, DVec3::new(0.3, -0.05, 0.0), DVec3::new(0.55, 0.05, 0.0));
    let (upper, fore) = (DQuat::from_rotation_z(0.2), DQuat::from_rotation_y(0.4));
    let target = wrist + DVec3::new(-0.02, 0.01, 0.03);
    let (new_upper, new_fore) = two_bone(shoulder, elbow, wrist, target, upper, fore).unwrap();
    let r1 = new_upper * upper.inverse();
    let new_elbow = shoulder + r1 * (elbow - shoulder);
    let new_wrist = new_elbow + new_fore * fore.inverse() * (wrist - elbow);
    assert!(new_wrist.distance(target) < 1e-9);
    assert!((new_elbow.distance(shoulder) - elbow.distance(shoulder)).abs() < 1e-12);
    // The elbow stays on the same side of the shoulder-wrist line.
    let side = |e: DVec3, w: DVec3| (e - shoulder).cross(w - shoulder).z.signum();
    assert_eq!(side(new_elbow, target), side(elbow, wrist));
    assert!(two_bone(shoulder, elbow, wrist, DVec3::X * 0.7, upper, fore).is_none());
}

#[test]
fn push_out_takes_the_shortest_exit_from_a_concave_corner() {
    // Two inward-facing walls meeting at a right angle, like the gap between
    // sternum and a cap: x >= 0 and y >= 0 are outside. A point 10 mm into
    // both needs exactly (11, 11) mm with a 1 mm margin, not a zigzag.
    let wall = |normal: DVec3, u: DVec3| {
        let v = normal.cross(u);
        let points = [-u - v, u - v, u + v].map(|p| p * 2.0);
        let points2 = [-u - v, u + v, -u + v].map(|p| p * 2.0);
        [points, points2].map(|mut pts| {
            if (pts[1] - pts[0]).cross(pts[2] - pts[0]).dot(normal) < 0.0 { pts.swap(1, 2); }
            Face { points: pts, normal, open_edges: [false; 3] }
        })
    };
    let faces: Vec<Face> = wall(DVec3::X, DVec3::Y).into_iter().chain(wall(DVec3::Y, DVec3::X)).collect();
    let point = DVec3::new(-0.01, -0.01, 0.0);
    let push = push_out(&[point], &faces, 0.001);
    assert!(signed_distance(point + push.delta, &faces).unwrap().0 >= 0.001 - 1e-5);
    // Exact up to the 10 µm stopping tolerance; never beyond the shortest exit.
    assert!(push.delta.distance(DVec3::new(0.011, 0.011, 0.0)) < 2e-5, "{:?}", push.delta);
    assert!(push.delta.length() <= DVec3::new(0.011, 0.011, 0.0).length() + 1e-9);
}

#[test]
fn finger_bends_just_enough_to_rest_on_the_surface() {
    // A straight finger along +Y; the surface y = 0.09 faces -Y (the finger's side).
    let rest = [0.03, 0.04, 0.03].map(|y| Transform::from_translation(Vec3::Y * y));
    let finger = FingerRig { axis: Vec3::Z, ..FingerRig::new(rest, false) };
    let n = -DVec3::Y;
    let square = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let mut points = [a, b, c].map(|[x, z]| DVec3::new(x, 0.09, z));
        if (points[1] - points[0]).cross(points[2] - points[0]).dot(n) < 0.0 { points.swap(1, 2); }
        Face { points, normal: n, open_edges: [false; 3] }
    };
    let faces = vec![square([-1., -1.], [1., -1.], [1., 1.]), square([-1., -1.], [1., 1.], [-1., 1.])];
    let fit = conform_finger(&finger, Mat4::IDENTITY, &faces, 0.001);
    assert!(fit.penetration > 0.03 && fit.bend > 0.0 && fit.residual == 0.0, "{:?}", (fit.penetration, fit.bend));
    let clearance = |bend: f32| {
        let chain = finger.chain(1.0, finger_angles(&finger, bend));
        chain.iter().map(|p| 0.09 - p.y as f64).fold(f64::INFINITY, f64::min)
    };
    assert!(clearance(fit.bend as f32) >= finger.radius as f64 + 0.001 - 1e-6);
    // Minimal: a slightly smaller bend still penetrates.
    assert!(clearance(fit.bend as f32 * 0.97) < finger.radius as f64 + 0.001);
    // A finger already clear keeps its rest pose.
    let far = conform_finger(&finger, Mat4::from_translation(Vec3::NEG_Y * 0.2), &faces, 0.001);
    assert_eq!((far.bend, far.penetration, far.angles), (0.0, 0.0, [0.0; 3]));
}
