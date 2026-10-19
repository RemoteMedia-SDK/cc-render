# Material calibration checkpoint (gravity and inertia)

`examples/tissue_calibrate.rs` applies material overrides
(`material.path=value`), refits the unloaded reference so standing gravity
still reproduces the authored shape (the `tissue_fit` loop, 0.25 mm), then
measures in the anchor (body) frame, without contacts:

- static nipple and free-node centroid shift from standing for face-down/up,
  upside-down and either side down (`equilibrate` with rotated gravity);
- a 4 cm, 0.15 s jump-landing ringdown (frequency, damping ratio);
- the orientation rig's 3 cm, 3 Hz side and vertical shakes (worst element
  volume ratio, peak node offset).

It agrees with the Bevy orientation lab to about 1 mm (statics) and 0.02
(shake volume ratio). `--write=fitted.json` saves the refitted asset.
`table.py DIR` prints the per-sweep tables in `tables.txt` (nipple mm; PS =
prone-to-supine nipple travel; shJ/shNode = worst shake volume ratio/peak node).

## Targets (literature)

- Prone-to-supine nipple displacement about 24–91 mm (median groups, MRI).
- Free-fall spring/damper fit: about 73.5 N/m above and 658 N/m below rest,
  about 2 N s/m: roughly 1.5–5 Hz and damping ratio 0.05–0.13 for 0.5–1 kg.

## Findings

- Shear modulus is the main static lever, but response saturates (about
  mu^-0.3 below 60 Pa) while the worst element volume ratio falls.
- Shear viscosity sets damping (5 Pa s is over-damped; 0.3–0.5 Pa s gives
  0.08–0.10). Rate (240 vs 960 Hz) and iterations (32 vs 64) change nothing.
- Lower bulk modulus adds sag by losing volume (J down to 0.08): rejected.
- Making the attachment compliant beyond the skin rim breaks the reference
  fit or the linear solve (`sweep5`); gliding needs a sliding formulation.
- Strain stiffening trades sag for shake integrity.

| material | upside-down nipple | prone–supine | ringdown | shake worst J |
|---|---|---|---|---|
| selected (500 Pa, 5 Pa s, k 8) | 17 mm | 8 mm | 6.5 Hz, 0.15 | 0.78 |
| s40 (40 Pa, 0.4 Pa s, k 8) | 46 mm | 30 mm | 4.4 Hz, 0.10 | 0.40 |
| s40-k32 (40 Pa, 0.4 Pa s, k 32) | 36 mm | 22 mm | 5.2 Hz, 0.08 | 0.50 |

Both candidates are saved as refitted prototype assets
(`assets/tissue/beautiful_assistant.base-calibrated-s40{,-k32}-prototype.json`)
and rendered through all ten orientation scenarios
(`orientation-lab/calibrated-s40-01`, `calibrated-s40-k32-01`). Held poses look
more natural; in the side shake s40 forms a visible crease at the upper inner
edge of her right breast where the simulated region meets fixed skin, and
s40-k32 a smaller dent. The selected asset is unchanged.

Next: the seam. The coarse cage (about 240 nodes per breast, more than half
attached) cannot spread large motion across the boundary.
