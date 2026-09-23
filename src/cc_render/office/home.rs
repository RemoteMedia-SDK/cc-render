//! Room layout for the bundled, roofless home. Furniture and walls live in home.json.
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

/// World-space room bounds and an unobstructed point just inside its entrance.
#[derive(Clone, Copy, Debug)]
pub struct HomeRoom {
    pub id: &'static str,
    pub min_xz: [f32; 2],
    pub max_xz: [f32; 2],
    pub entrance: [f32; 2],
}

pub const ROOMS: [HomeRoom; 5] = [
    HomeRoom {
        id: "kitchen",
        min_xz: [-4., -9.],
        max_xz: [0., -4.],
        entrance: [-2., -4.65],
    },
    HomeRoom {
        id: "bathroom",
        min_xz: [0., -9.],
        max_xz: [2.5, -4.],
        entrance: [0.9, -4.65],
    },
    HomeRoom {
        id: "bedroom",
        min_xz: [2.5, -9.],
        max_xz: [6., -4.],
        entrance: [4.1, -4.65],
    },
    HomeRoom {
        id: "work_desk",
        min_xz: [-4., -4.],
        max_xz: [2., 3.],
        entrance: [1.35, -1.8],
    },
    HomeRoom {
        id: "living_room",
        min_xz: [2., -4.],
        max_xz: [6., 3.],
        entrance: [3., 1.],
    },
];

/// Custom object-array scenes retain the original office floor and lighting.
#[derive(Resource)]
pub(crate) struct BundledHome(pub bool);

pub(crate) fn spawn_surfaces(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    bundled: bool,
) {
    let (center, size) = if bundled {
        (Vec3::new(1., -0.025, -3.), Vec3::new(10., 0.05, 12.))
    } else {
        (Vec3::new(0., -0.025, -1.), Vec3::new(8., 0.05, 8.))
    };
    commands.spawn((
        Name::new("Home floor"),
        Mesh3d(meshes.add(Cuboid::from_size(size))),
        MeshMaterial3d(materials.add(if bundled {
            Color::srgb(0.43, 0.32, 0.22)
        } else {
            Color::srgb(0.28, 0.23, 0.18)
        })),
        Transform::from_translation(center),
        RigidBody::Fixed,
        Collider::cuboid(size.x / 2., size.y / 2., size.z / 2.),
    ));
    if bundled {
        spawn_floor_finishes(commands, meshes, materials);
    }
    let lights: Vec<_> = if bundled {
        ROOMS
            .iter()
            .map(|r| {
                Vec3::new(
                    (r.min_xz[0] + r.max_xz[0]) / 2.,
                    2.5,
                    (r.min_xz[1] + r.max_xz[1]) / 2.,
                )
            })
            .collect()
    } else {
        vec![Vec3::new(0., 3., -1.)]
    };
    for position in lights {
        commands.spawn((
            PointLight {
                color: if bundled {
                    Color::srgb(1., 0.88, 0.73)
                } else {
                    Color::srgb(1., 0.8, 0.6)
                },
                intensity: if bundled { 90000. } else { 180000. },
                range: 10.,
                shadows_enabled: true,
                ..default()
            },
            Transform::from_translation(position),
        ));
    }
}

fn spawn_floor_finishes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    // Thin visual finishes share the solid floor below and never obstruct navigation.
    for (name, center, size, color) in [
        (
            "Kitchen stone floor",
            [-2., 0.002, -6.5],
            [3.84, 0.004, 4.84],
            [0.57, 0.57, 0.49],
        ),
        (
            "Bathroom grout",
            [1.25, 0.002, -6.5],
            [2.34, 0.004, 4.84],
            [0.40, 0.47, 0.46],
        ),
        (
            "Bedroom rug",
            [4.3, 0.006, -7.],
            [2.5, 0.012, 3.5],
            [0.66, 0.60, 0.50],
        ),
        (
            "Living room rug",
            [4.5, 0.006, 0.2],
            [2.7, 0.012, 3.2],
            [0.64, 0.60, 0.48],
        ),
    ] {
        commands.spawn((
            Name::new(name),
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::from(size)))),
            MeshMaterial3d(materials.add(Color::srgb(color[0], color[1], color[2]))),
            Transform::from_translation(Vec3::from(center)),
        ));
    }
    let tile = meshes.add(Cuboid::new(0.44, 0.004, 0.44));
    let tile_material = materials.add(Color::srgb(0.72, 0.79, 0.75));
    for x in 0..5 {
        for z in 0..10 {
            commands.spawn((
                Mesh3d(tile.clone()),
                MeshMaterial3d(tile_material.clone()),
                Transform::from_xyz(0.29 + x as f32 * 0.48, 0.006, -8.7 + z as f32 * 0.48),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_render::office::{
        pathfinder::find_path, scene_metadata, starter_objects, validate_objects,
    };

    #[test]
    fn all_rooms_and_furniture_are_reachable_with_avatar_clearance() {
        let objects = starter_objects();
        validate_objects(&objects).unwrap();
        let scene = scene_metadata(&objects);
        let spawn = Vec2::ZERO;
        for room in ROOMS {
            find_path(&scene, spawn, room.entrance.into(), None)
                .unwrap_or_else(|e| panic!("{} entrance: {e:?}", room.id));
        }
        for object in objects.iter().filter(|o| !o.id.starts_with("wall_")) {
            let p = object.approach_position();
            find_path(&scene, spawn, Vec2::new(p[0], p[2]), None)
                .unwrap_or_else(|e| panic!("{} approach: {e:?}", object.id));
        }
    }

    #[test]
    fn kitchen_plate_has_reachable_oriented_grasp_anchor() {
        let objects = starter_objects();
        let sink = objects.iter().find(|o| o.id == "kitchen_sink").unwrap();
        assert_eq!(sink.resolved_world_keypoints()["placement"].position, sink.placement_position().unwrap());
        let plate = objects.iter().find(|o| o.id == "plate").unwrap();
        assert!(plate.dynamic);
        let approach = Vec3::from(plate.approach_position());
        let grasp = plate.grasp_transform().unwrap();
        let forward = (grasp.translation - approach).with_y(0.).normalize();
        // Same right-wrist convention as the mug: -X reaches forward, Z is thumb-up.
        assert!((grasp.rotation * -Vec3::X).dot(forward) > 0.99);
        assert!((grasp.rotation * Vec3::Z).dot(Vec3::Y) > 0.99);
        assert!(grasp.translation.distance(approach.with_y(grasp.translation.y)) < 0.75);
        let scene = scene_metadata(&objects);
        let route = find_path(&scene, Vec2::ZERO, approach.xz(), None).unwrap();
        assert!(route.len() > 2, "must route around furniture and through the kitchen door");
        assert!(plate.position[0] > ROOMS[0].min_xz[0] && plate.position[0] < ROOMS[0].max_xz[0]);
        assert!(plate.position[2] > ROOMS[0].min_xz[1] && plate.position[2] < ROOMS[0].max_xz[1]);
    }

    #[test]
    fn walls_enclose_home_and_routes_use_doorways() {
        let scene = scene_metadata(&starter_objects());
        for outside in [[7., 0.], [-5., 0.], [1., -10.], [1., 4.]] {
            assert!(find_path(&scene, Vec2::ZERO, outside.into(), None).is_err());
        }
        for from in ROOMS {
            for to in ROOMS {
                let path = find_path(&scene, from.entrance.into(), to.entrance.into(), None)
                    .unwrap_or_else(|e| panic!("{} to {}: {e:?}", from.id, to.id));
                // Every crossing of the rear partition must use a door opening,
                // with 35 cm reserved on either side for the avatar envelope.
                for segment in path.windows(2) {
                    let a = segment[0].xz;
                    let b = segment[1].xz;
                    if (a.y + 4.) * (b.y + 4.) < 0. {
                        let x = a.x + (b.x - a.x) * (-4. - a.y) / (b.y - a.y);
                        assert!(
                            [[-2.35, -1.65], [0.55, 1.25], [3.75, 4.45]]
                                .iter()
                                .any(|range| (range[0]..=range[1]).contains(&x)),
                            "route crosses wall at x={x}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn part_materials_are_optional_and_validated() {
        let mut objects = starter_objects();
        assert!(objects
            .iter()
            .flat_map(|o| &o.parts)
            .any(|p| p.color.is_some()));
        objects[0].parts[0].color = Some([1.1, 0., 0.]);
        assert!(validate_objects(&objects).is_err());
    }
}
