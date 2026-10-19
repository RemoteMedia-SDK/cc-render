//! Asset-bound read-only FK/LBS for native candidate admission and observations.
//! This module cannot write skeleton transforms or construct motion packets.
use super::super::renderer::SkeletalPose;
use bevy::math::{Mat3, Mat4, Quat, Vec3, Vec4};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Skin {
    bones: Vec<[usize; 4]>,
    weighted: Vec<[[f32; 4]; 4]>,
}
impl Skin {
    pub(crate) fn points<'a>(&'a self, world: &'a [Mat4]) -> impl Iterator<Item = Vec3> + 'a {
        self.bones
            .iter()
            .zip(&self.weighted)
            .map(|(bones, weighted)| {
                (0..4)
                    .map(|i| world[bones[i]] * Vec4::from_array(weighted[i]))
                    .fold(Vec4::ZERO, |sum, point| sum + point)
                    .truncate()
            })
    }
    fn validate(&self, nodes: usize) -> bool {
        !self.bones.is_empty()
            && self.bones.len() == self.weighted.len()
            && self.bones.iter().flatten().all(|index| *index < nodes)
            && self.weighted.iter().all(|weights| {
                weights.iter().flatten().all(|value| value.is_finite())
                    && weights.iter().all(|v| v[3] >= 0.)
                    && (weights.iter().map(|v| v[3]).sum::<f32>() - 1.).abs() < 1e-4
            })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvatarGeometry {
    version: u8,
    avatar_sha256: String,
    profile_identity: String,
    local: Vec<[f32; 16]>,
    rest: Vec<[f32; 16]>,
    parents: Vec<i32>,
    mapped: [usize; 22],
    smpl_parents: [i32; 22],
    neutral: [[f32; 9]; 22],
    inter: [[f32; 9]; 22],
    pelvis_parent: [f32; 9],
    pub(crate) meshes: Vec<Skin>,
    pub(crate) seat_patches: [Skin; 2],
}
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct SeatEvidence {
    /// Geometric distances, not proof of force-bearing support.
    pub patch_distances_m: [f32; 2],
    pub maximum_penetration_m: f32,
}

impl std::fmt::Debug for AvatarGeometry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvatarGeometry").field("profile_identity", &self.profile_identity).finish_non_exhaustive()
    }
}

impl AvatarGeometry {
    pub(crate) fn profile_identity(&self) -> &str { &self.profile_identity }
    #[cfg(test)]
    pub(crate) fn test_fixture() -> Self {
        Self::parse(
            &serde_json::to_vec(&tests::fixture()).unwrap(),
            "avatar",
            "profile",
        )
        .unwrap()
    }

    pub(crate) fn measure_seat(
        &self,
        world: &[Mat4],
        object: &super::OfficeObject,
    ) -> Result<SeatEvidence, String> {
        if world.len() != self.parents.len() || world.iter().any(|m| !m.is_finite()) {
            return Err("invalid measured world frames".into());
        }
        let part = object
            .seat_surface_part
            .and_then(|i| object.parts.get(i))
            .ok_or("missing physical seat surface")?;
        if part.half_extents[0] <= 0.025 || part.half_extents[2] <= 0.025 {
            return Err("seat surface lacks an inset contact region".into());
        }
        let transform = object.transform();
        if !transform.translation.is_finite()
            || !transform.rotation.is_finite()
            || object.parts.iter().any(|p| {
                p.position.iter().any(|v| !v.is_finite())
                    || p.half_extents.iter().any(|v| !v.is_finite() || *v <= 0.)
            })
        {
            return Err("invalid physical seat geometry".into());
        }
        let inverse = transform.rotation.inverse();
        let local = |point: Vec3| inverse * (point - transform.translation);
        let center = Vec3::from_array(part.position) + Vec3::Y * part.half_extents[1];
        let half = Vec3::from_array(part.half_extents) - Vec3::new(0.025, 0., 0.025);
        let patch_distances_m = std::array::from_fn(|side| {
            self.seat_patches[side]
                .points(world)
                .map(|point| {
                    let delta = (local(point) - center).abs();
                    Vec3::new(
                        (delta.x - half.x).max(0.),
                        delta.y,
                        (delta.z - half.z).max(0.),
                    )
                    .length()
                })
                .fold(f32::INFINITY, f32::min)
        });
        let mut maximum_penetration_m = 0f32;
        for mesh in &self.meshes {
            for point in mesh.points(world).map(local) {
                for solid in &object.parts {
                    let depth = (Vec3::from_array(solid.half_extents)
                        - (point - Vec3::from_array(solid.position)).abs())
                    .min_element();
                    maximum_penetration_m = maximum_penetration_m.max(depth);
                }
            }
        }
        Ok(SeatEvidence {
            patch_distances_m,
            maximum_penetration_m,
        })
    }

    pub(crate) fn load(
        path: &std::path::Path,
        capability: &super::scene_signing::SceneSamplerCapability,
    ) -> Result<Self, String> {
        let expected = capability
            .renderer_geometry_sha256
            .as_ref()
            .ok_or("sampler has no renderer geometry binding")?;
        Self::load_bound(path, expected, &capability.avatar_sha256, &capability.profile_identity)
    }

    /// Load read-only geometry with explicit content, avatar and profile bindings.
    /// Diagnostic callers use the same validation as registered scene samplers;
    /// this does not authorize a seat or alter motion/collision admission.
    pub fn load_bound(
        path: &std::path::Path,
        expected: &str,
        avatar: &str,
        profile: &str,
    ) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
            return Err("invalid renderer geometry file".into());
        }
        let mut bytes = Vec::new();
        file.take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let mut hash = hmac_sha256::Hash::new();
        hash.update(&bytes);
        let actual: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if actual != expected {
            return Err("renderer geometry content binding mismatch".into());
        }
        Self::parse(&bytes, avatar, profile)
    }

    fn parse(bytes: &[u8], avatar: &str, profile: &str) -> Result<Self, String> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("avatar geometry exceeds bound".into());
        }
        let geometry: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if geometry.version != 1
            || geometry.avatar_sha256 != avatar
            || geometry.profile_identity != profile
        {
            return Err("avatar geometry identity mismatch".into());
        }
        geometry.validate()?;
        Ok(geometry)
    }

    fn validate(&self) -> Result<(), String> {
        let n = self.parents.len();
        if !(22..=512).contains(&n)
            || self.local.len() != n
            || self.rest.len() != n
            || self.mapped.iter().any(|i| *i >= n)
            || self
                .mapped
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != 22
            || self.smpl_parents[0] != -1
            || self
                .smpl_parents
                .iter()
                .enumerate()
                .skip(1)
                .any(|(i, p)| *p < 0 || *p as usize >= i)
            || self.parents.iter().any(|p| *p < -1 || *p >= n as i32)
            || self.parents[self.mapped[0]] < 0
        {
            return Err("invalid avatar geometry hierarchy".into());
        }
        for node in 0..n {
            let mut parent = node as i32;
            for _ in 0..=n {
                if parent < 0 {
                    break;
                }
                parent = self.parents[parent as usize];
            }
            if parent >= 0 {
                return Err("cyclic avatar geometry hierarchy".into());
            }
        }
        let matrices_valid = self.local.iter().chain(&self.rest).all(|m| {
            let matrix = Mat4::from_cols_array(m);
            matrix.is_finite() && matrix.determinant().abs() > 1e-12
        }) && self
            .neutral
            .iter()
            .chain(&self.inter)
            .chain(std::iter::once(&self.pelvis_parent))
            .all(|m| Mat3::from_cols_array(m).is_finite());
        let count: usize = self.meshes.iter().map(|mesh| mesh.bones.len()).sum();
        if !matrices_valid
            || self.meshes.is_empty()
            || count > 500_000
            || self.seat_patches.iter().any(|p| p.bones.len() > 4096)
            || !self
                .meshes
                .iter()
                .chain(&self.seat_patches)
                .all(|skin| skin.validate(n))
        {
            return Err("invalid avatar geometry matrices or skin".into());
        }
        Ok(())
    }

    /// World position of SMPL-22 joint `i` in the matrices `world` returned for a pose.
    pub(crate) fn smpl_joint(&self, world: &[Mat4], i: usize) -> Vec3 {
        world[self.mapped[i]].w_axis.truncate()
    }

    pub(crate) fn world(&self, pose: &SkeletalPose) -> Result<Vec<Mat4>, String> {
        let mut source = [Quat::IDENTITY; 22];
        let mut target = [Mat3::IDENTITY; 22];
        for i in 0..22 {
            let q = Quat::from_array(pose.joint_quats[i]);
            if !q.is_finite() || q.length_squared() < 1e-8 {
                return Err("invalid native rotation".into());
            }
            source[i] = if i == 0 {
                q.normalize()
            } else {
                source[self.smpl_parents[i] as usize] * q.normalize()
            };
            target[i] = Mat3::from_quat(source[i]) * Mat3::from_cols_array(&self.neutral[i]);
        }
        let root = Vec3::from_array(pose.root_pos);
        if !root.is_finite() {
            return Err("invalid native root".into());
        }
        let mut local: Vec<_> = self.local.iter().map(Mat4::from_cols_array).collect();
        for i in 0..22 {
            let node = self.mapped[i];
            let parent = if i == 0 {
                Mat3::from_cols_array(&self.pelvis_parent)
            } else {
                target[self.smpl_parents[i] as usize] * Mat3::from_cols_array(&self.inter[i])
            };
            let rotation = parent.transpose() * target[i];
            let rest = local[node];
            let translation = if i == 0 {
                Mat4::from_cols_array(&self.rest[self.parents[node] as usize])
                    .inverse()
                    .transform_point3(root)
            } else {
                rest.w_axis.truncate()
            };
            local[node] = Mat4::from_cols(
                (rotation.x_axis * rest.x_axis.truncate().length()).extend(0.),
                (rotation.y_axis * rest.y_axis.truncate().length()).extend(0.),
                (rotation.z_axis * rest.z_axis.truncate().length()).extend(0.),
                translation.extend(1.),
            );
        }
        let mut world = vec![None; local.len()];
        for node in 0..local.len() {
            visit(node, &local, &self.parents, &mut world);
        }
        Ok(world.into_iter().map(Option::unwrap).collect())
    }
}

fn visit(node: usize, local: &[Mat4], parents: &[i32], world: &mut [Option<Mat4>]) -> Mat4 {
    if let Some(matrix) = world[node] {
        return matrix;
    }
    let result = if parents[node] < 0 {
        local[node]
    } else {
        visit(parents[node] as usize, local, parents, world) * local[node]
    };
    world[node] = Some(result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> serde_json::Value {
        let m4 = Mat4::IDENTITY.to_cols_array();
        let m3 = Mat3::IDENTITY.to_cols_array();
        let skin = serde_json::json!({"bones":[[2,0,0,0]],"weighted":[[[1.,0.,0.,1.],[0.,0.,0.,0.],[0.,0.,0.,0.],[0.,0.,0.,0.]]]});
        let mut parents = vec![1; 23];
        parents[0] = -1;
        parents[1] = 0;
        let mut smpl = vec![0; 22];
        smpl[0] = -1;
        serde_json::json!({"version":1,"avatar_sha256":"avatar","profile_identity":"profile",
            "local":vec![m4;23],"rest":vec![m4;23],"parents":parents,
            "mapped":(1..23).collect::<Vec<_>>(),"smpl_parents":smpl,
            "neutral":vec![m3;22],"inter":vec![m3;22],"pelvis_parent":m3,
            "meshes":[skin.clone()],"seat_patches":[skin.clone(),skin]})
    }
    fn parse(value: &serde_json::Value) -> Result<AvatarGeometry, String> {
        AvatarGeometry::parse(&serde_json::to_vec(value).unwrap(), "avatar", "profile")
    }
    #[test]
    fn fixed_fk_lbs_does_not_mutate_native_pose() {
        let geometry = parse(&fixture()).unwrap();
        let mut pose = SkeletalPose::rest();
        pose.root_pos = [2., 3., 4.];
        pose.joint_quats[1] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        let before = pose.clone();
        let world = geometry.world(&pose).unwrap();
        let point = geometry.meshes[0].points(&world).next().unwrap();
        assert!(point.distance(Vec3::new(2., 4., 4.)) < 1e-5);
        assert_eq!(before.joint_quats, pose.joint_quats);
        assert_eq!(before.root_pos, pose.root_pos);
    }
    #[test]
    fn rejects_identity_hierarchy_and_skin_corruption() {
        let mut value = fixture();
        value["avatar_sha256"] = "other".into();
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value["parents"][0] = 2.into();
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value["meshes"][0]["bones"][0][0] = 999.into();
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value["seat_patches"][0]["weighted"][0][0][3] = 0.5.into();
        assert!(parse(&value).is_err());
    }
    #[test]
    fn file_requires_exact_sampler_content_binding() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        file.write_all(&bytes).unwrap();
        let mut hash = hmac_sha256::Hash::new();
        hash.update(&bytes);
        let digest = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let mut capability = super::super::scene_signing::SceneSamplerCapability {
            protocol: 3,
            context_version: 1,
            policy: "model-owned-scene-v1".into(),
            avatar_sha256: "avatar".into(),
            profile_identity: "profile".into(),
            authenticated_context: true,
            server_instance: "test".into(),
            grip_capture: None,
            renderer_geometry_sha256: Some(digest),
        };
        assert!(AvatarGeometry::load(file.path(), &capability).is_ok());
        let pin = capability.renderer_geometry_sha256.as_deref().unwrap();
        assert!(AvatarGeometry::load_bound(file.path(), pin, "avatar", "profile").is_ok());
        assert!(AvatarGeometry::load_bound(file.path(), pin, "wrong", "profile").is_err());
        assert!(AvatarGeometry::load_bound(file.path(), pin, "avatar", "wrong").is_err());
        assert!(AvatarGeometry::load_bound(file.path(), "wrong", "avatar", "profile").is_err());
        file.write_all(b" ").unwrap();
        assert!(AvatarGeometry::load(file.path(), &capability).is_err());
        capability.renderer_geometry_sha256 = None;
        assert!(AvatarGeometry::load(file.path(), &capability).is_err());
    }
    #[test]
    #[ignore = "requires independently exported avatar/native-pose geometry fixture"]
    fn exported_native_geometry_matches_python() {
        let path = std::env::var("CC_GEOMETRY_FIXTURE").unwrap();
        let fixture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let bytes = std::fs::read(fixture["asset"].as_str().unwrap()).unwrap();
        let geometry = AvatarGeometry::parse(
            &bytes,
            fixture["avatar"].as_str().unwrap(),
            fixture["profile"].as_str().unwrap(),
        )
        .unwrap();
        for frame in fixture["frames"].as_array().unwrap() {
            let pose = SkeletalPose {
                stream: None,
                pts_ms: 0,
                joint_quats: serde_json::from_value(frame["quaternions"].clone()).unwrap(),
                root_pos: serde_json::from_value(frame["root"].clone()).unwrap(),
            };
            let actual = geometry.world(&pose).unwrap();
            let expected: Vec<[f32; 16]> = serde_json::from_value(frame["world"].clone()).unwrap();
            assert_eq!(actual.len(), expected.len());
            let error = actual
                .iter()
                .zip(expected)
                .flat_map(|(m, e)| {
                    m.to_cols_array()
                        .into_iter()
                        .zip(e)
                        .map(|(a, b)| (a - b).abs())
                })
                .fold(0f32, f32::max);
            assert!(error < 2e-5, "world error {error}");
            if frame.get("seat_distances").is_some() {
                let chair = serde_json::from_value(fixture["chair"].clone()).unwrap();
                let evidence = geometry.measure_seat(&actual, &chair).unwrap();
                for side in 0..2 {
                    assert!(
                        (evidence.patch_distances_m[side]
                            - frame["seat_distances"][side].as_f64().unwrap() as f32)
                            .abs()
                            < 2e-5
                    );
                }
                assert!(
                    (evidence.maximum_penetration_m
                        - frame["penetration"].as_f64().unwrap() as f32)
                        .abs()
                        < 2e-5
                );
            }
            for side in 0..2 {
                let minimum = geometry.seat_patches[side]
                    .points(&actual)
                    .map(|p| p.y)
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    (minimum - frame["seat_minima"][side].as_f64().unwrap() as f32).abs() < 2e-5
                );
            }
        }
    }
}
