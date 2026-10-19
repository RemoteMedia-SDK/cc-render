//! Test-only, bit-exact fixed-contact query capture. This is a query reproducer,
//! not a solver checkpoint: it cannot resume the coupled cloth simulation.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SweepCase {
    error: String,
    old: Vec<[u64; 3]>,
    end: Vec<[u64; 3]>,
    radius: u64,
    center: [u64; 3],
    rotation: [u64; 4],
    half_extents: [u64; 3],
    border: u64,
    friction: u64,
    bounds_min: [u64; 3],
    bounds_max: [u64; 3],
}

fn bits(v: DVec3) -> [u64; 3] {
    v.to_array().map(f64::to_bits)
}
fn vector(v: [u64; 3]) -> DVec3 {
    DVec3::from_array(v.map(f64::from_bits))
}

impl SweepCase {
    fn new(
        c: &FixedClothCollider,
        old: &[DVec3],
        end: &[DVec3],
        radius: f64,
        error: &ClothError,
    ) -> Self {
        Self {
            error: error.to_string(),
            old: old.iter().copied().map(bits).collect(),
            end: end.iter().copied().map(bits).collect(),
            radius: radius.to_bits(),
            center: bits(c.center),
            rotation: c.rotation.to_array().map(f64::to_bits),
            half_extents: bits(c.half_extents),
            border: c.border.to_bits(),
            friction: c.friction.to_bits(),
            bounds_min: bits(c.bounds_min),
            bounds_max: bits(c.bounds_max),
        }
    }

    fn collider(&self) -> FixedClothCollider {
        let half = vector(self.half_extents);
        let border = f64::from_bits(self.border);
        let shape = if border == 0. {
            SharedShape::cuboid(half.x as f32, half.y as f32, half.z as f32)
        } else {
            SharedShape::round_cuboid(half.x as f32, half.y as f32, half.z as f32, border as f32)
        };
        // Do not pass through new(): it normalizes rotation and recomputes
        // bounds. Preserve the exact query inputs, including their roundoff.
        FixedClothCollider {
            shape,
            sweep_shape: parry_ccd::shape::SharedShape::cuboid(half.x, half.y, half.z),
            center: vector(self.center),
            rotation: DQuat::from_array(self.rotation.map(f64::from_bits)),
            half_extents: half,
            border,
            friction: f64::from_bits(self.friction),
            bounds_min: vector(self.bounds_min),
            bounds_max: vector(self.bounds_max),
        }
    }

    fn replay(&self) -> Result<(), ClothError> {
        let collider = self.collider();
        let radius = f64::from_bits(self.radius);
        match (self.old.as_slice(), self.end.as_slice()) {
            ([old], [end]) => {
                collider
                    .project_swept_particle(&mut vector(*end), vector(*old), radius)
                    .map(|_| ())
            }
            ([a, b, c], [x, y, z]) => collider
                .swept_triangle([*a, *b, *c].map(vector), [*x, *y, *z].map(vector), radius)
                .map(|_| ()),
            _ => Err(ClothError::InvalidInput),
        }
    }
}

pub(super) fn capture(
    c: &FixedClothCollider,
    old: &[DVec3],
    end: &[DVec3],
    radius: f64,
    error: &ClothError,
) {
    let Some(directory) = std::env::var_os("CC_CLOTH_FAILURE_DIR") else {
        return;
    };
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "sweep-{}-{}.json",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let path = std::path::PathBuf::from(directory).join(name);
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        serde_json::to_writer_pretty(&mut file, &SweepCase::new(c, old, end, radius, error))?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    })();
    // Capture errors must not replace the original physics error.
    eprintln!("CLOTH_SWEEP_CAPTURE {}: {result:?}", path.display());
}

#[test]
#[ignore = "diagnostic: set CC_CLOTH_REPLAY to a captured query file"]
fn replay_captured_fixed_sweep() {
    let path = std::env::var_os("CC_CLOTH_REPLAY").expect("CC_CLOTH_REPLAY is required");
    let case: SweepCase = serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap();
    let result = case.replay();
    eprintln!("Captured error: {}; replay result: {result:?}", case.error);
    result.expect("captured query still fails; this is a diagnostic, not full-bed acceptance");
}

#[test]
fn fixed_sweep_capture_preserves_rotated_collider_and_query_bits() {
    let collider = FixedClothCollider::new(
        SharedShape::round_cuboid(0.5, 0.1, 0.5, 0.03),
        Isometry::new(
            bevy_rapier3d::rapier::na::Vector3::new(1.1, 2.2, -3.3),
            bevy_rapier3d::rapier::na::Vector3::new(0.1, 0.2, 0.3),
        ),
        0.4,
    )
    .unwrap();
    let old = [
        DVec3::new(-0.2, 0.5, -0.2),
        DVec3::new(0.2, 0.5, -0.2),
        DVec3::new(0., 0.5, 0.2),
    ]
    .map(|p| collider.center + collider.rotation * p);
    let end = old.map(|p| p - collider.rotation * DVec3::Y);
    let case = SweepCase::new(
        &collider,
        &old,
        &end,
        0.001,
        &ClothError::SweptContactUnresolved,
    );
    let restored: SweepCase = serde_json::from_slice(&serde_json::to_vec(&case).unwrap()).unwrap();
    let rebuilt = restored.collider();
    assert_eq!(
        case,
        SweepCase::new(
            &rebuilt,
            &old,
            &end,
            0.001,
            &ClothError::SweptContactUnresolved
        )
    );
    assert_eq!(restored, case);
    let original = collider.swept_triangle(old, end, 0.001).unwrap().unwrap();
    let replayed = rebuilt
        .swept_triangle(
            [0, 1, 2].map(|i| vector(restored.old[i])),
            [0, 1, 2].map(|i| vector(restored.end[i])),
            f64::from_bits(restored.radius),
        )
        .unwrap()
        .unwrap();
    assert_eq!(bits(original.point), bits(replayed.point));
    assert_eq!(bits(original.normal), bits(replayed.normal));
    assert_eq!(original.distance.to_bits(), replayed.distance.to_bits());
    restored.replay().unwrap();
}
