# Hand rest on breast skin (display only, exception-assisted)

Owner request: fingers should rest on the breast skin rather than press
through it; then “Palm too”. Recorded as the second amendment of
[`hand-ribcage-clamp-exception.md`](../../../../docs/design/hand-ribcage-clamp-exception.md).

## Design

After each tissue step, the wrist/palm push-out (arm IK) and finger bends
are solved again against the **solved breast skin plus the ribcage** (caps
dropped: the skin closes the surface) and written to the displayed joint world
transforms for that frame only (`apply_hand_rest`). The tissue is still pressed
by the generated hand under the bounded pressure, so the breast compresses;
tissue sampling, its FK-versus-display check and the next frame use the
ribcage-only correction. Verified: tissue nodes in `rest-final` are identical
in all 478 frames to the ribcage-only `hand-clamp-01/clamp-08-final`.

## Seed-739 result (`summary.ndjson`, right hand; left never touches)

- 7716–7833 ms: the palm is 3–40 mm inside the solved skin; the rest moves it
  onto the surface (4–41 mm), fingers bend up to 0.9 rad, no residual.
  `review/hand-rest-comparison-slow.mp4` (7600–8000 ms, generated /
  ribcage+fingers / rest) shows the hand lying on the breast where the other
  two hide it inside.
- 7850–7900 ms: needs 58–71 mm, above the approved 50 mm bound: withheld and
  reported unsupported, so the hand pops back inside.
- 7916–8000 ms: the generated hand is wedged between both breasts. Palm depth
  is only 44–49 mm, but the smallest rigid exit is **16–30 cm**
  (`rest-unbounded-diagnostic`). No display correction can fix these frames.
- A stiffer contact does not remove the gap: at 50 / 200 kPa the palm is still
  48 / 47 mm inside while the tissue is crushed (volume ratio 0.59 / 0.42).
- No-contact jump with the rest active: no activity.

The remaining failure is the source motion (a chest rub that drives the palm
~5 cm into the cleavage), not the renderer. Options: widen the bound to ~75 mm
(fixes through 7900 ms only), or regenerate/condition the motion.

Tests: 94 tissue, 6 clamp, 5 `tissue_lab` example; build at the pre-existing
38 warnings. Live performance not measured.
