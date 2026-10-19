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

## Surface creases and displacement smoothing

The squashed elements in the shakes are interior free tetrahedra, not the
attachment seam. The visible creases come from the embedding: skin follows
each cage tetrahedron linearly, so large swings kink it along cage faces (and
the unsmoothed s40-k32 flips skin triangles upside-down and in the tumble,
face alignment -0.28/-0.31). `bevy_app/tissue_smoothing.rs` Taubin-smooths the
displacement over the welded skin graph (fixed skin stays put, UV seams move
together, nodes/contacts/physics unchanged); opt-in via
`CC_AVATAR_TISSUE_SURFACE_SMOOTHING=<passes>`, default off.

| s40-k32 | no smoothing | 8 passes |
|---|---|---|
| upside-down / tumble face alignment | -0.28 / -0.31 | 0.30 / 0.28 |
| shake side / bounce | 0.32 / 0.29 | 0.59 / 0.57 |

Evidence: `orientation-lab/smooth8-s40-01`, `smooth24-s40-01` (8 is as good as
24), `smooth8-s40-k32-01`; traces in `smoothing-traces.tar.gz`. The s40
side-shake crease that survives is a real fold from a ~9 cm swing into fixed
side skin.

ARDY jump (`orientation-lab/jump-01`, `jump-front-side-comparison.mp4`):
s40-k32 + smoothing doubles peak skin motion (33 -> 58 mm) and stays at face
alignment >= 0.85 until the final frame (8000 ms), where the outer edge of her
right breast distorts while rising with arms out (alignment 0.30, J 0.27).
The chest cameras are fixed and only suit the orientation rig.

Next: a finer, more uniform cage (element volumes span about 70-7,600 mm3,
p5-p95) to remove the side-edge distortion; live 120 Hz cost unmeasured.
(Superseded below: finer cages are too slow, and the distortion was the metric.)

## Cage resolution (`../cage-resolution/`)

`build_skin_tissue_volume.py` takes `--surface-points`, `--max-volume` and
`--node-budget` (defaults reproduce the current cage exactly). Fitted and
measured with s40-k32 material (per breast):

| cage | nodes / tets | upside-down nipple | prone–supine | shake ms/step |
|---|---|---|---|---|
| current (128 surface points) | 241 / 657 | 30–41 mm | 16–29 mm | ~3.2 |
| surf440 | ~630 / 2100 | 65–71 mm | 30–37 mm | 82–94 |
| surf884 | ~1170 / 3950 | 73–75 mm | 30–37 mm | 190–280 |

Finer cages move about twice as far with the same material and reach the
literature range, but cost 25–90x per step (the fits took 343 s
and 1136 s) and surf440's left side shake did not converge (force 0.0047 N).
Not usable in real time. The CG iteration cap now grows with the system
(`256.max(3 n)`); solves that converged before are unchanged.

## Smooth display layer (MLS embedding)

The coarse cage stays the physics; the skin is the fine layer. Opt-in
`CC_AVATAR_TISSUE_SURFACE_EMBEDDING=mls` (`bevy_app/tissue_embedding.rs`)
moves each skin vertex with linear-basis moving least squares over its 16
nearest cage nodes (Wendland C2 weights, support 1.5x the 16th distance)
instead of one tetrahedron's four nodes. It reproduces affine motion,
including large rotations, exactly, and fixed nodes still carry zero
displacement. Contacts and physics are unchanged.

Crease metric: `maximum_crease_deg` in the tissue trace is the largest change
of dihedral angle between adjacent skin triangles from the rest pose (welded
across UV seams), so whole-breast rotation is not counted. The earlier
face-alignment metric counted rotation of small nipple triangles as damage;
the "outer-edge distortion" reported at the end of `jump-01` is not visible in
frames.

Peak crease (degrees), s40-k32, orientation rig
(`orientation-lab/embed-*-01`) and the ARDY jump (`orientation-lab/jump-02`):

| embedding | upside-down | tumble | shake side / bounce | face-down | side-left | jump (p95) |
|---|---|---|---|---|---|---|
| linear | 81.8 | 84.6 | 67.4 / 72.1 | 64.3 | 44.1 | 169.7 |
| linear + smoothing 8 | 37.7 | 40.0 | 29.7 / 30.7 | 29.2 | 28.0 | 56.2 |
| mls | 68.5 | 70.7 | 51.2 / 61.3 | 53.6 | 26.9 | 147.6 |
| **mls + smoothing 4** | **26.4** | **27.7** | **31.3 / 23.4** | **19.8** | **18.3** | **44.3 (28)** |

On the selected asset the jump peak falls from 47.8 (p95 26.2) with the
current embedding to 16.9 (p95 9.8) with mls + smoothing 4. s40-k32 with
mls + smoothing 4 creases about as much as the selected asset does today
while moving about twice as far (58 vs 33 mm). Review video:
`orientation-lab/jump-02/jump-current-vs-candidate{,-third-speed}.mp4`.
Traces: `orientation-lab/embedding-traces.tar.gz`. Defaults are unchanged;
live 120 Hz cost of the embedding is unmeasured.
