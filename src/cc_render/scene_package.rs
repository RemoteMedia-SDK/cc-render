//! Authored environment assets and geometry. Contains no avatar motion channels.
use anyhow::{ensure, Context, Result};
use bevy::prelude::Resource;
use serde::Deserialize;
use std::{collections::HashSet, path::{Path, PathBuf}};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSolid {
    pub id: String,
    pub position: [f32; 3],
    pub half_extents: [f32; 3],
    #[serde(default)]
    pub yaw_deg: f32,
    /// Rounded-box radius; half_extents include the rounded border.
    #[serde(default)]
    pub border_radius_m: f32,
    #[serde(default)]
    pub navigation_obstacle: bool,
    /// A free-space region representative, never a prescribed body pose.
    pub approach: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneCamera {
    pub id: String,
    pub position: [f32; 3],
    pub look_at: [f32; 3],
    pub fov_deg: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficLane {
    pub start: [f32; 3],
    pub end: [f32; 3],
    #[serde(default)]
    pub path: Vec<[f32; 3]>,
    pub speed_mps: f32,
    pub count: usize,
}

#[derive(Clone, Debug, Deserialize, Resource)]
#[serde(deny_unknown_fields)]
pub struct ScenePackage {
    pub schema_version: u32,
    pub id: String,
    pub geometry: PathBuf,
    pub floor_y: f32,
    pub solids: Vec<SceneSolid>,
    pub cameras: Vec<SceneCamera>,
    pub traffic: Vec<TrafficLane>,
    #[serde(default = "default_vehicle_variants")]
    pub vehicle_variants: usize,
    pub lake_center: [f32; 3],
    pub lake_size: [f32; 2],
    pub geographic_origin: [f64; 2],
    pub provenance: String,
    #[serde(default)]
    pub lightmaps: Vec<SceneLightmap>,
    #[serde(default)]
    pub point_lights: Vec<ScenePointLight>,
    #[serde(default)]
    pub mechanisms: Vec<super::scene_mechanism::SceneMechanism>,
    #[serde(default)]
    pub cloths: Vec<super::scene_cloth::SceneCloth>,
    #[serde(default)]
    pub water_fixtures: Vec<super::scene_water::WaterFixture>,
    #[serde(default)]
    pub props: Vec<super::scene_props::SceneProp>,
    /// Package-relative file of declared zones and interactions by object id (the office
    /// overlay format), kept apart from geometry.
    #[serde(default)]
    pub interactions: Option<PathBuf>,
    #[serde(skip)]
    overlays: super::office::InteractionOverlays,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenePointLight {
    pub id: String,
    pub position: [f32; 3],
    pub color_srgb: [f32; 3],
    pub lumens: f32,
    pub range_m: f32,
    pub radius_m: f32,
    pub shadows: bool,
    pub diffuser: Option<SceneLightDiffuser>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLightDiffuser {
    /// Exact mesh name within this package's environment GLB, never an avatar mesh.
    pub mesh: String,
    /// Linear emissive colour multiplier, with camera exposure applied.
    pub emissive_strength: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneLightmap {
    pub mesh: String,
    pub image: String,
    /// Gain on a normalized sky bake, scaled by the active sky illumination.
    pub sky_strength: f32,
}

fn default_vehicle_variants() -> usize { 1 }

impl ScenePackage {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read scene package {}", path.display()))?;
        let mut package: Self = serde_json::from_slice(&bytes).context("decode scene package")?;
        package.validate()?;
        package.geometry = path.parent().unwrap_or(Path::new(".")).join(&package.geometry)
            .canonicalize().context("scene geometry is missing")?;
        ensure!(package.geometry.is_file(), "scene geometry must be a file");
        for asset in ["vehicle.glb", "water_normal.png", "env/pisa_diffuse_rgb9e5_zstd.ktx2", "env/pisa_specular_rgb9e5_zstd.ktx2"] {
            ensure!(package.geometry.with_file_name(asset).is_file(), "scene asset missing: {asset}");
        }
        image::image_dimensions(package.geometry.with_file_name("water_normal.png"))
            .context("decode scene water normal texture")?;
        for map in &package.lightmaps {
            image::image_dimensions(package.geometry.parent().unwrap().join(&map.image))
                .with_context(|| format!("decode scene lightmap {}", map.image))?;
        }
        for part in package.mechanisms.iter().flat_map(|m| &m.parts) {
            part.verify_asset(package.geometry.parent().unwrap())?;
        }
        for prop in &package.props { prop.verify_asset(package.geometry.parent().unwrap())?; }
        for cloth in &mut package.cloths { cloth.load(package.geometry.parent().unwrap())?; }
        if let Some(file) = &package.interactions {
            ensure!(file.components().all(|c| matches!(c, std::path::Component::Normal(_))),
                "interactions path must be package-relative");
            let path = path.parent().unwrap_or(Path::new(".")).join(file);
            package.overlays = serde_json::from_slice(&std::fs::read(&path)
                .with_context(|| format!("read scene interactions {}", path.display()))?)
                .context("decode scene interactions")?;
            let mut objects = package.base_objects();
            let unknown = super::office::apply_overlays(&mut objects, &package.overlays);
            ensure!(unknown.is_empty(), "scene interactions name unknown objects: {unknown:?}");
            super::office::validate_objects(&objects).map_err(|e| anyhow::anyhow!("scene interactions: {e}"))?;
        }
        Ok(package)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported scene package version");
        ensure!(!self.id.is_empty() && self.floor_y == 0.0, "scene uses a local finished floor at y=0");
        ensure!(!self.solids.is_empty() && !self.cameras.is_empty(), "scene needs solids and cameras");
        let mut ids = HashSet::new();
        for solid in &self.solids {
            ensure!(!solid.id.is_empty() && ids.insert(&solid.id), "duplicate/empty solid id");
            ensure!(finite(&solid.position) && positive(&solid.half_extents) && solid.yaw_deg.is_finite(), "invalid solid {}", solid.id);
            ensure!(solid.approach.as_ref().is_none_or(|p| finite(p)), "invalid approach");
        }
        let mut cameras = HashSet::new();
        for camera in &self.cameras {
            ensure!(cameras.insert(&camera.id) && !camera.id.is_empty(), "duplicate/empty camera id");
            ensure!(finite(&camera.position) && finite(&camera.look_at) && camera.position != camera.look_at
                && (15.0..=110.0).contains(&camera.fov_deg), "invalid camera");
        }
        for lane in &self.traffic {
            ensure!(finite(&lane.start) && finite(&lane.end) && lane.start != lane.end
                && lane.speed_mps.is_finite() && lane.speed_mps > 0.0 && lane.count <= 100, "invalid traffic lane");
            if !lane.path.is_empty() {
                ensure!(lane.path.len() >= 2 && lane.path.first() == Some(&lane.start)
                    && lane.path.last() == Some(&lane.end)
                    && lane.path.iter().all(|p| finite(p))
                    && lane.path.windows(2).all(|p| p[0] != p[1]), "invalid traffic path");
            }
        }
        ensure!(self.traffic.iter().map(|l| l.count).sum::<usize>() <= 500, "traffic capacity exceeded");
        ensure!((1..=8).contains(&self.vehicle_variants), "vehicle variants must be between 1 and 8");
        ensure!(finite(&self.lake_center) && positive(&self.lake_size), "invalid lake bounds");
        ensure!(self.geographic_origin.iter().all(|v| v.is_finite()), "invalid geographic origin");
        let mut mapped_meshes = HashSet::new();
        for map in &self.lightmaps {
            ensure!(!map.mesh.is_empty() && mapped_meshes.insert(&map.mesh), "duplicate/empty lightmap mesh");
            ensure!(map.sky_strength.is_finite() && (0.0..=100.0).contains(&map.sky_strength), "invalid lightmap strength");
            ensure!(!map.image.is_empty() && Path::new(&map.image).components().all(|c|
                matches!(c, std::path::Component::Normal(_))), "lightmap path must be package-relative");
        }
        for solid in &self.solids {
            ensure!(solid.border_radius_m.is_finite() && solid.border_radius_m >= 0.0
                && solid.half_extents.iter().all(|h| solid.border_radius_m < *h), "invalid solid border radius");
        }
        validate_point_lights(&self.point_lights)?;
        super::scene_water::validate(&self.water_fixtures)?;
        super::scene_mechanism::validate_mechanisms(&self.mechanisms, &self.solids)?;
        ensure!(self.props.len() <= 128, "scene prop capacity exceeded");
        for prop in &self.props {
            prop.validate()?;
            ensure!(!self.solids.iter().any(|s| s.id == prop.object.id), "prop duplicates solid id");
        }
        for source in self.water_fixtures.iter().filter_map(|f| f.container.as_ref()) {
            ensure!(self.props.iter().any(|p| p.object.id == source.prop && p.object.dynamic),
                "pour source must name a dynamic scene prop");
        }
        super::office::validate_objects(&self.office_objects()).map_err(anyhow::Error::msg)?;
        super::scene_cloth::validate(self)?;
        Ok(())
    }

    /// Office objects with the package's declared zones and interactions.
    pub(crate) fn office_objects(&self) -> Vec<super::office::OfficeObject> {
        let mut objects = self.base_objects();
        super::office::apply_overlays(&mut objects, &self.overlays);
        objects
    }

    fn base_objects(&self) -> Vec<super::office::OfficeObject> {
        self.static_solids().filter(|s| s.navigation_obstacle).map(|s| super::office::OfficeObject {
            id: s.id.clone(), label: s.id.replace('_', " "), position: s.position,
            half_extents: s.half_extents, yaw_deg: s.yaw_deg, color: [0.5; 3],
            parts: Vec::new(), dynamic: false, pushable: false, rotation_xyzw: None,
            keypoints: Default::default(), approach: s.approach.unwrap_or([0.0; 3]),
            seat: None, seat_surface_part: None, grasp: None, surface: None,
            zones: Vec::new(), interactions: Vec::new(),
        }).chain(self.mechanisms.iter().flat_map(|m| m.office_objects()))
            .chain(self.props.iter().map(|p| p.object.clone())).collect()
    }

    pub(crate) fn static_solids(&self) -> impl Iterator<Item = &SceneSolid> {
        self.solids.iter().filter(|solid|
            !self.mechanisms.iter().any(|mechanism| mechanism.replaces_solid == solid.id))
    }
}

fn finite(values: &[f32]) -> bool { values.iter().all(|v| v.is_finite()) }
pub(super) fn positive(values: &[f32]) -> bool { values.iter().all(|v| v.is_finite() && *v > 0.0) }

fn validate_point_lights(lights: &[ScenePointLight]) -> Result<()> {
    ensure!(lights.len() <= 32, "point light capacity exceeded");
    let mut ids = HashSet::new();
    let mut diffusers = HashSet::new();
    for light in lights {
        ensure!(!light.id.is_empty() && ids.insert(&light.id), "duplicate/empty point light id");
        ensure!(finite(&light.position) && positive(&[light.lumens, light.range_m])
            && light.radius_m.is_finite() && (0.0..light.range_m).contains(&light.radius_m),
            "invalid point light geometry or intensity: {}", light.id);
        ensure!(light.color_srgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && light.color_srgb.iter().any(|v| *v > 0.0), "invalid point light colour: {}", light.id);
        if let Some(diffuser) = &light.diffuser {
            ensure!(!diffuser.mesh.is_empty() && diffusers.insert(&diffuser.mesh)
                && positive(&[diffuser.emissive_strength]), "invalid/duplicate light diffuser: {}", light.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ScenePackage {
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"id":"test","geometry":"missing.glb","floor_y":0,
            "solids":[{"id":"floor","position":[0,-0.1,0],"half_extents":[6,0.1,5]}],
            "cameras":[{"id":"living","position":[0,1,-3],"look_at":[0,1,0],"fov_deg":60}],
            "traffic":[],"lake_center":[0,-90,1000],"lake_size":[6000,6000],
            "geographic_origin":[43.64,-79.39],"provenance":"fixture"
        })).unwrap()
    }

    #[test]
    fn rejects_invalid_geometry_and_camera_bounds() {
        let mut p=fixture(); assert!(p.validate().is_ok());
        p.solids[0].half_extents[0]=f32::NAN; assert!(p.validate().is_err());
        p=fixture(); p.cameras[0].look_at=p.cameras[0].position; assert!(p.validate().is_err());
        p=fixture(); p.solids.push(p.solids[0].clone()); assert!(p.validate().is_err());
    }

    #[test]
    fn package_rejects_body_motion_channels() {
        let raw=r#"{"schema_version":1,"root_pos":[0,1,0]}"#;
        assert!(serde_json::from_str::<ScenePackage>(raw).is_err());
    }

    #[test]
    fn vehicle_variants_preserve_legacy_default_and_reject_invalid_counts() {
        let mut p = fixture();
        assert_eq!(p.vehicle_variants, 1);
        for count in [0, 9, usize::MAX] {
            p.vehicle_variants = count;
            assert!(p.validate().is_err());
        }
        p.vehicle_variants = 6;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn point_lights_default_empty_and_reject_invalid_configuration() {
        let mut p = fixture();
        assert!(p.point_lights.is_empty());
        let light: ScenePointLight = serde_json::from_value(serde_json::json!({
            "id":"lamp", "position":[1,1,1], "color_srgb":[1,0.8,0.5],
            "lumens":600, "range_m":3.5, "radius_m":0.08, "shadows":true,
            "diffuser":{"mesh":"Shade","emissive_strength":50}
        })).unwrap();
        p.point_lights = vec![light.clone()];
        assert!(p.validate().is_ok());
        p.point_lights.push(light.clone());
        assert!(p.validate().is_err());
        for invalid in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            p.point_lights = vec![ScenePointLight { lumens: invalid, ..light.clone() }];
            assert!(p.validate().is_err());
        }
        p.point_lights = vec![ScenePointLight { radius_m: 3.5, ..light.clone() }];
        assert!(p.validate().is_err());
        p.point_lights = vec![ScenePointLight { color_srgb: [1.2, 0.8, 0.5], ..light.clone() }];
        assert!(p.validate().is_err());
        p.point_lights = vec![ScenePointLight { position: [f32::NAN,0.0,0.0], ..light.clone() }];
        assert!(p.validate().is_err());
        p.point_lights = vec![ScenePointLight { id:"second".into(), ..light.clone() }, light];
        assert!(p.validate().is_err(), "a diffuser cannot belong to two lights");
        assert!(serde_json::from_str::<SceneLightDiffuser>(r#"{"mesh":"Shade","emissive_strength":50,"root_pos":[0,1,0]}"#).is_err());
    }

    #[test]
    fn missing_geometry_is_an_error() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("scene.json");
        let raw=include_str!("../../scenes/toronto_condo/scene.json");
        std::fs::write(&path,raw).unwrap();
        assert!(ScenePackage::load(&path).is_err());
    }

    #[test]
    fn rejects_ambiguous_or_invalid_lightmaps() {
        let mut p = fixture();
        let map = SceneLightmap { mesh: "Floor".into(), image: "floor.png".into(), sky_strength: 2.0 };
        p.lightmaps = vec![map.clone()];
        assert!(p.validate().is_ok());
        p.lightmaps.push(map.clone());
        assert!(p.validate().is_err());
        for path in ["", "../outside.png", "/absolute.png"] {
            p.lightmaps = vec![SceneLightmap { image: path.into(), ..map.clone() }];
            assert!(p.validate().is_err(), "accepted {path}");
        }
        for strength in [f32::NAN, -1.0, 101.0] {
            p.lightmaps = vec![SceneLightmap { sky_strength: strength, ..map.clone() }];
            assert!(p.validate().is_err());
        }
    }

    #[test]
    fn missing_lightmap_fails_during_package_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("env")).unwrap();
        for asset in ["interior-city.glb", "vehicle.glb", "env/pisa_diffuse_rgb9e5_zstd.ktx2", "env/pisa_specular_rgb9e5_zstd.ktx2"] {
            std::fs::write(dir.path().join(asset), []).unwrap();
        }
        image::RgbaImage::new(1, 1).save(dir.path().join("water_normal.png")).unwrap();
        let mut raw: serde_json::Value = serde_json::from_str(include_str!("../../scenes/toronto_condo/scene.json")).unwrap();
        raw["lightmaps"] = serde_json::json!([{"mesh":"Floor","image":"missing.png","sky_strength":2}]);
        let path = dir.path().join("scene.json");
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let error = ScenePackage::load(&path).unwrap_err().to_string();
        assert!(error.contains("decode scene lightmap missing.png"), "{error}");
    }
}

impl SceneSolid {
    pub(crate) fn collider(&self) -> bevy_rapier3d::prelude::Collider {
        let [x,y,z]=self.half_extents;
        let r=self.border_radius_m;
        if r==0.0 { bevy_rapier3d::prelude::Collider::cuboid(x,y,z) }
        else { bevy_rapier3d::prelude::Collider::round_cuboid(x-r,y-r,z-r,r) }
    }
    pub(crate) fn isometry(&self) -> bevy_rapier3d::rapier::parry::math::Isometry<f32> {
        use bevy_rapier3d::rapier::na::{Isometry3,Vector3};
        Isometry3::new(Vector3::from(self.position),Vector3::y()*self.yaw_deg.to_radians())
    }
}
