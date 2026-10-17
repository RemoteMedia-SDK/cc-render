//! Night-time vehicle lenses and local headlight pools; no traffic control.
use bevy::prelude::*;
use std::collections::HashMap;

pub(super) fn spawn_headlight(parent: &mut ChildBuilder, night: bool) {
    parent.spawn((
        Name::new("Vehicle headlight road pool"),
        super::weather_transition::NightSpot(12_000.0 * 4.0 * std::f32::consts::PI),
        if night { Visibility::Inherited } else { Visibility::Hidden },
        SpotLight {
            // Bevy 0.15 divides spot intensity by 4*pi, independently of
            // cone angle. Specify the authored central beam in candela;
            // 2500 in this field previously yielded only about 200 cd.
            intensity: if night { 12_000.0 * 4.0 * std::f32::consts::PI } else { 0.0 },
            range: 28.0,
            radius: 0.2,
            inner_angle: 0.13,
            outer_angle: 0.22,
            color: Color::srgb(0.93, 0.96, 1.0),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.0, 0.68, 2.26).looking_to(Vec3::new(0.0, -0.11, 1.0), Vec3::Y),
    ));
}

fn lens_emission(name: &str) -> Option<LinearRgba> {
    if name.starts_with("Vehicle headlamp lens ") {
        Some(LinearRgba::rgb(4000.0, 4200.0, 4500.0))
    } else if name.starts_with("Vehicle taillamp lens ") {
        Some(LinearRgba::rgb(1300.0, 8.0, 2.0))
    } else {
        None
    }
}

pub(super) fn prepare_vehicle_lamps(
    weather: Res<super::weather_transition::AppliedWeather>,
    mut emission_registry: ResMut<super::weather_transition::NightEmission>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut added: Query<
        (&Name, &mut MeshMaterial3d<StandardMaterial>),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
    mut prepared: Local<HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>>,
) {
    for (name, mut handle) in &mut added {
        let Some(emission) = lens_emission(name.as_str()) else {
            continue;
        };
        let id = handle.0.id();
        if !assets.get_path(id).is_some_and(|p| {
            p.source().as_str() == Some("scene")
                && p.path().file_name().is_some_and(|f| f == "vehicle.glb")
        }) {
            continue;
        }
        let material = if let Some(material) = prepared.get(&id) {
            material.clone()
        } else {
            let Some(original) = materials.get(&handle.0) else {
                continue;
            };
            let mut lit = original.clone();
            lit.emissive = emission * weather.current.sky.z;
            lit.emissive_exposure_weight = 1.0;
            let material = materials.add(lit);
            emission_registry.0.push((material.clone(), emission));
            prepared.insert(id, material.clone());
            tracing::info!(target: "cc_render", "night vehicle lens material prepared: {}", name);
            material
        };
        handle.0 = material;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_named_vehicle_lenses_emit_distinct_front_and_rear_colours() {
        let front = lens_emission("Vehicle headlamp lens left").unwrap();
        let rear = lens_emission("Vehicle taillamp lens right.0").unwrap();
        assert!(front.blue > front.red && front.green > 1000.0);
        assert!(rear.red > rear.green * 100.0);
        for name in ["Headlamp", "Vehicle body", "Hair.0", "Warm lamp diffuser"] {
            assert!(lens_emission(name).is_none());
        }
    }
}
