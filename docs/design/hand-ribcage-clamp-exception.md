# Exception: display hand clamp at the ribcage

Status: **owner-approved 2026-10-18**, interactive session on
`feat/breast-soft-tissue`. Recorded by the implementing agent from the owner's
explicit choice; under [motion ownership](model-owned-motion.md) the agent
cannot approve or widen it. This file is not a security boundary.

Owner request, verbatim: “can we instead just force the wrist root to NOT pass
through the body, even when ardy requests it? go as close as possible but not
through”. Owner selection: “Approve: ribcage only”.

Amendment, same session: measurement showed the wrist and palm never enter the
ribcage on seed 739; the fingers do (up to 23 mm). Owner selection: “Bend
fingers to the surface” (penetrating renderer-owned fingers bend at the
knuckles; the wrist/palm clamp stays as the guard).
Evidence: `artifacts/soft-tissue/avatar-contact/hand-clamp-01/README.md`.

## Why model-native routes do not cover it now

The seed-739 chest rub drives ARDY's hands into the torso. Post-generation
selection (reject and regenerate) cannot guarantee a non-penetrating block,
and model-native contact conditioning (`add-model-owned-spatial-goals`) is not
implemented; the installed hand encoder also emits root constraints the
contract rejects.

## Scope and bounds

- **Transforms:** (1) renderer-side arm correction (SMPL shoulder, elbow and
  wrist of the penetrating side) moving the wrist/palm to the nearest
  non-penetrating position (minimal-norm translation, two-bone IK, hand world
  orientation preserved); (2) knuckle bend of renderer-owned CC5 finger bones
  (flexion within the rig's limits, or up to 0.5 rad knuckle extension), the
  smallest that keeps each finger outside. Both act against the **rigid
  ribcage only**: torso skin dominated by Spine01/02, RibsTwist, Clavicle and
  residual Breast weights outside the simulated patches, closed by each tissue
  region's posterior backing cap. Hands still press into breast tissue, which
  responds under the bounded contact pressure.
- `CC_AVATAR_HAND_RIBCAGE_CLAMP=whole` (rigid whole-hand translation) exists
  only for comparison; it was measured and rejected.
- **Deterministic per source time** and applied once, before both tissue
  contact sampling and display, so the tissue reacts to the hand the viewer
  sees. No temporal state, smoothing or held poses.
- **Bound:** a wrist correction above 50 mm is not applied; the step reports
  unsupported contact instead of hiding it. A bounded correction that does not
  fully converge is applied and its residual reported (withholding it makes
  the hand jump). Fingers that cannot clear report their residual.
- **Untouched:** ARDY packets, service history, root/pelvis, legs, spine and
  every other joint. Validators keep reading the raw generated motion.
- **Opt-in:** active only with the volumetric tissue path and
  `CC_AVATAR_HAND_RIBCAGE_CLAMP=1`; absent leaves motion as generated, and an
  unknown value fails closed at startup.

## Visibility

The displayed frame's correction per side (penetration, wrist correction,
residual, finger penetration, bend and residual) is in the tissue integration
report and trace; activation logs a WARN. Runs with any correction are labelled
exception-assisted, not baseline-compliant.

## Expiry and removal

Remove when ARDY can be conditioned on the contact surface, or on owner
withdrawal. Any broader use (other actions, other body parts, the office
runtime) needs a new approval.
