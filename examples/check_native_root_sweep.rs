//! Read-only offline root-sweep admission using the actual office validator.
//! No model, renderer, pose output, action scheduling or destination exemptions.
use bevy::math::Vec3;
use cc_render_plugin::cc_render::office::{controller::OfficeController, starter_objects};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Obstacle {
    id: String,
    position: [f32; 3],
    half_extents: [f32; 3],
    navigation_yaw_deg: f32,
    navigation_obstacle: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    obstacles: Vec<Obstacle>,
    roots: Vec<[f32; 3]>,
}

fn check(value: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let input: Input = serde_json::from_value(value.clone())?;
    anyhow::ensure!((2..=161).contains(&input.roots.len()), "invalid root count");
    anyhow::ensure!(input.obstacles.len() <= 128, "too many obstacles");
    anyhow::ensure!(
        input.roots.iter().flatten().all(|v| v.is_finite()),
        "nonfinite root"
    );
    let template = starter_objects().remove(0);
    let mut controller = OfficeController::default();
    controller.objects.clear();
    let mut ids = std::collections::HashSet::new();
    for obstacle in input.obstacles {
        anyhow::ensure!(
            !obstacle.id.is_empty() && ids.insert(obstacle.id.clone()),
            "invalid identity"
        );
        anyhow::ensure!(
            obstacle.position.iter().all(|v| v.is_finite())
                && obstacle
                    .half_extents
                    .iter()
                    .all(|v| v.is_finite() && *v > 0.)
                && obstacle.navigation_yaw_deg.is_finite(),
            "invalid obstacle geometry"
        );
        if !obstacle.navigation_obstacle {
            continue;
        }
        // Only metadata footprints are consumed. No world/physics entities exist here.
        let mut object = template.clone();
        object.id = obstacle.id;
        object.position = obstacle.position;
        object.half_extents = obstacle.half_extents;
        object.yaw_deg = obstacle.navigation_yaw_deg;
        object.parts.clear();
        controller.objects.push(object);
    }
    for (index, segment) in input.roots.windows(2).enumerate() {
        // The source start is read-only validator context, not a written/rendered pose.
        controller.observed.pelvis = segment[0];
        if !controller.permits_native_root(Vec3::from_array(segment[1])) {
            return Ok(serde_json::json!({"permitted":false, "first_rejected_segment":index}));
        }
    }
    Ok(serde_json::json!({"permitted":true, "first_rejected_segment":null}))
}

fn main() -> anyhow::Result<()> {
    let value = serde_json::from_reader(std::io::stdin().lock())?;
    println!("{}", check(&value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(roots: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"obstacles":[{"id":"box", "position":[0.,0.,0.],
            "half_extents":[0.3,0.3,0.3], "navigation_yaw_deg":0.,
            "navigation_obstacle":true}], "roots":roots})
    }

    #[test]
    fn crossing_rejected_without_changing_input() {
        let value = input(serde_json::json!([[-1., 1., 0.], [1., 1., 0.]]));
        let original = value.clone();
        let result = check(&value).unwrap();
        assert_eq!(result["permitted"], false);
        assert_eq!(result["first_rejected_segment"], 0);
        assert_eq!(value, original);
    }

    #[test]
    fn clear_path_and_published_obstacle_membership() {
        assert_eq!(
            check(&input(serde_json::json!([[-1., 1., 1.], [1., 1., 1.]]))).unwrap()["permitted"],
            true
        );
        let mut value = input(serde_json::json!([[-1., 1., 0.], [1., 1., 0.]]));
        value["obstacles"][0]["navigation_obstacle"] = false.into();
        assert_eq!(check(&value).unwrap()["permitted"], true);
    }

    #[test]
    fn malformed_geometry_is_not_admitted() {
        let mut value = input(serde_json::json!([[0., 1., 0.]]));
        assert!(check(&value).is_err());
        value["roots"] = serde_json::json!([[-1., 1., 1.], [1., 1., 1.]]);
        value["obstacles"][0]["half_extents"][0] = (-0.1).into();
        assert!(check(&value).is_err());
    }
}
