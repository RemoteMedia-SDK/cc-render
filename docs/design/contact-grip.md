# Generic contact grip

The physics primitive is an **ideal rigid grip**, not finger animation, human force closure, or a claim that every generated reach is a successful grasp. Semantic motion_goal v2 still does not expose precise object interactions; current rendered interaction proofs use recorded native-model output.

## Admission

A semantic request identifies a hand and a specific dynamic prop. The existing 5cm wrist-to-grasp bound and 2s pursuit timeout remain. In addition, the requested hand's solid collider must intersect a solid collider belonging to that exact prop's rigid body. Compound props have child colliders; a contact with another prop cannot qualify. Static props and sensor-only intersections cannot qualify.

Office props intentionally filter animation-driven collision impulses before grasping. Therefore admission uses Rapier/Parry's actual collision shapes and current physics transforms with **zero prediction distance**, not the filtered solver-contact list. This preserves the existing anti-launch filtering. No positive separation is admitted as contact.

## Capture and carrying

At admitted contact, snapshot the existing relative rigid-body frames and create a fixed joint. Both joint frames coincide at the object's existing world pose. No object transform is written, no canonical grip orientation is imposed, and no animation body pose is changed. Collision response is disabled only between the joined pair; other collisions remain governed by their existing filters. Physics carries the object as model-owned hand motion continues. Release removes the joint.

A forced `GrabCmd.palm_offset_override` is rejected explicitly. Historical default/sidecar palm-offset settings do not reposition the new contact grip. The old canonical joint helper is retained only as a historical test fixture, not a runtime path.

## Fixed collider calibration

Fallback capsules now use the actual bind bone segment converted into bone-local coordinates, including its offset/direction and uniform avatar scale. The previous implementation discarded the measured segment center and used a centered local-Y capsule. This is action-independent collision geometry, not motion correction. Explicit authored collider overrides are unchanged.

## Evidence and limits

`physics::contact_grasp` tests check proximity rejection, world-frame preservation, root and child collider ownership, a wrong-object contact, filtered contact admission, physical carrying, and release. `fitted_capsule_matches_world_bone_endpoints` checks a rotated/translated bone at meter and centimeter scales. Synthetic hand driving in the physics test is not model-generation evidence.

The retained `grasp-handoff01` trial separately displays all440 native frames through the actual CC5 office: navigation, arrival, reach, contact-triggered mug capture, then two generated holding batches. The earlier repeated-grasp continuation failed the unchanged head-twist validator and remains retained. Neither trial proves live goal transport, general seed robustness, finger closure, grip strength, balance under arbitrary loads, or continuous skin-mesh nonpenetration. Collider approximation and ideal joint strength are explicit physical-model limitations.
