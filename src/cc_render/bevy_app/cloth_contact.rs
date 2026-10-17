//! Read observed finger globals after propagation and the cloth's committed tick.
//! These approximate distal capsules are geometry evidence only. No closure,
//! attachment, pose correction or claim of physical skin contact is made here.
use super::*;
use crate::cc_render::{
    deformable_surface::DeformedSurface,
    office::hand::{CapturedHand, FingerRig, HandRigs, FINGERS},
    renderer::Hand,
    scene_cloth::{ClothContactWitness, ClothFingerContacts, ClothHandContacts},
};
use bevy_rapier3d::rapier::parry::{
    math::{Isometry, Point},
    shape::Capsule,
};

fn finger_contacts(
    surface: &DeformedSurface,
    name: &str,
    finger: &FingerRig,
    global: &GlobalTransform,
) -> Result<ClothFingerContacts, String> {
    let start = global.translation();
    let end = global.affine().transform_point3(finger.tip);
    if !start.is_finite()
        || !end.is_finite()
        || start.distance(end) <= 1e-6
        || !finger.radius.is_finite()
        || finger.radius <= 0.
    {
        return Err(format!("{name}: invalid observed distal geometry"));
    }
    let shape = Capsule::new(
        Point::from(start.to_array()),
        Point::from(end.to_array()),
        finger.radius,
    );
    let contacts = surface
        .contacts(&shape, &Isometry::identity())
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(ClothFingerContacts {
        finger: name.into(),
        segment_start: start.to_array(),
        segment_end: end.to_array(),
        radius_m: finger.radius,
        contacts: contacts
            .into_iter()
            .map(|c| ClothContactWitness {
                triangle: c.triangle(),
                barycentric: c.barycentric(),
                surface_revision: c.revision(),
                position: c.midsurface_position().to_array(),
                normal: c.normal().to_array(),
                separation_m: c.separation(),
            })
            .collect(),
    })
}

fn observed_hand(
    surface: &DeformedSurface,
    captured: &CapturedHand,
    globals: &Query<&GlobalTransform>,
) -> Result<Vec<ClothFingerContacts>, String> {
    FINGERS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let global = globals
                .get(captured.bones[i][2])
                .map_err(|_| format!("{name}: distal bone missing"))?;
            finger_contacts(surface, name, &captured.rig.fingers[i], global)
        })
        .collect()
}

fn unavailable(hand: Hand, status: &str) -> ClothHandContacts {
    ClothHandContacts {
        hand,
        status: status.into(),
        error: None,
        fingers: Vec::new(),
    }
}

fn hand_snapshot(
    cloth: &LiveCloth,
    hand: Hand,
    rigs: Option<&HandRigs>,
    globals: &Query<&GlobalTransform>,
) -> ClothHandContacts {
    let Some(captured) = rigs.and_then(|rigs| rigs.get(hand)) else {
        return unavailable(hand, "rig_unavailable");
    };
    match observed_hand(cloth.solver.surface(), captured, globals) {
        Ok(fingers) => ClothHandContacts {
            hand,
            status: "observed".into(),
            error: None,
            fingers,
        },
        Err(error) => ClothHandContacts {
            hand,
            status: "invalid_geometry".into(),
            error: Some(error),
            fingers: Vec::new(),
        },
    }
}

pub(super) fn observe(
    cloths: Query<&LiveCloth>,
    globals: Query<&GlobalTransform>,
    rigs: Option<Res<HandRigs>>,
    office: Option<Res<OfficeController>>,
    mut observations: ResMut<ClothObservations>,
) {
    for snapshot in &mut observations.0 {
        snapshot.hand_contacts.clear();
        let Some(cloth) = cloths.iter().find(|c| c.id == snapshot.id) else {
            continue;
        };
        let unavailable_reason = match office.as_ref() {
            None => Some("motion_unavailable"),
            Some(o)
                if o.motion_error.is_some()
                    || snapshot.error.is_some()
                    || o.observed.native_stream.is_none() =>
            {
                Some("motion_unavailable")
            }
            Some(o) if Some(o.observed.displayed_pts_ms) != snapshot.displayed_pts_ms => {
                Some("unsynchronized_pose")
            }
            Some(o)
                if o.observed.native_stream.as_ref().is_some_and(|stream| {
                    snapshot.session_id.as_deref() != Some(stream.session_id.as_str())
                }) || snapshot.surface_revision != cloth.solver.surface().revision() =>
            {
                Some("unsynchronized_pose")
            }
            Some(_) => None,
        };
        snapshot.hand_contacts = [Hand::Left, Hand::Right]
            .into_iter()
            .map(|hand| {
                unavailable_reason.map_or_else(
                    || hand_snapshot(cloth, hand, rigs.as_deref(), &globals),
                    |reason| unavailable(hand, reason),
                )
            })
            .collect();
    }
}

#[cfg(test)]
#[path = "cloth_contact_tests.rs"]
mod tests;
