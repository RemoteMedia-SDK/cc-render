//! A face mask the avatar can take off: primitives whose glTF material carries
//! `extras.face_mask = true` (written by `scripts/saluki/glb_materials.py` for
//! the Saluki operators' balaclavas, ski masks and hockey / ballistic masks)
//! are shown or hidden by [`crate::cc_render::CcRenderer::set_face_mask`].
//! Display only: the mask's geometry is untouched, and headwear stays on.
use bevy::{gltf::GltfMaterialExtras, prelude::*};
use tokio::sync::watch;

/// `true`: the mask is worn (the GLB as authored).
#[derive(Resource)]
pub(super) struct FaceMaskRx(pub watch::Receiver<bool>);

#[derive(Component)]
pub(super) struct FaceMask;

fn is_face_mask(extras: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(extras)
        .ok()
        .and_then(|v| v["face_mask"].as_bool())
        .unwrap_or(false)
}

pub(super) fn tag_face_masks(mut commands: Commands, prims: Query<(Entity, &GltfMaterialExtras), Added<GltfMaterialExtras>>) {
    for (entity, extras) in &prims {
        if is_face_mask(&extras.value) {
            commands.entity(entity).insert(FaceMask);
        }
    }
}

pub(super) fn show_face_masks(mut rx: ResMut<FaceMaskRx>, mut masks: Query<(&mut Visibility, Ref<FaceMask>)>) {
    let changed = rx.0.has_changed().unwrap_or(false);
    let worn = *rx.0.borrow_and_update();
    let want = if worn { Visibility::Inherited } else { Visibility::Hidden };
    for (mut visibility, tag) in &mut masks {
        if (changed || tag.is_added()) && *visibility != want {
            *visibility = want;
        }
    }
}

pub(super) fn plugin(app: &mut App, rx: watch::Receiver<bool>) {
    app.insert_resource(FaceMaskRx(rx))
        .add_systems(Update, (tag_face_masks, show_face_masks).chain());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tagged_primitives_toggle() {
        let (tx, rx) = watch::channel(true);
        let mut app = App::new();
        plugin(&mut app, rx);
        let extras = |value: &str| GltfMaterialExtras { value: value.into() };
        let mask = app.world_mut().spawn((extras(r#"{"face_mask":true}"#), Visibility::Inherited)).id();
        let helmet = app.world_mut().spawn((extras(r#"{"cc5":{}}"#), Visibility::Inherited)).id();
        let vis = |app: &App, e| *app.world().get::<Visibility>(e).unwrap();
        app.update();
        assert_eq!(vis(&app, mask), Visibility::Inherited);
        tx.send(false).unwrap();
        app.update();
        assert_eq!(vis(&app, mask), Visibility::Hidden);
        assert_eq!(vis(&app, helmet), Visibility::Inherited);
        // A primitive that spawns while the mask is off starts hidden.
        let late = app.world_mut().spawn((extras(r#"{"face_mask":true}"#), Visibility::Inherited)).id();
        app.update();
        assert_eq!(vis(&app, late), Visibility::Hidden);
        tx.send(true).unwrap();
        app.update();
        assert_eq!(vis(&app, mask), Visibility::Inherited);
        assert_eq!(vis(&app, late), Visibility::Inherited);
    }
}
