# Orientation lab (gravity and inertia, synthetic rig)

`scripts/avatars/tissue_orientation_lab.py` freezes one ARDY standing frame
(`standing-pose.json`) and turns the whole body as a rigid body about a point
between the breasts: face-down/up, upside-down, side-lying, quick turns, a
double forward tumble and three shakes (3 cm at 3 Hz side/vertical, 20° at
2 Hz twist). Motions use minimum-jerk easing; every scenario starts with 1.5 s
standing.

The packets are scripted test motion, never ARDY output, and are marked by
`synthetic-rig.json`. `tissue_lab` plays them with the office runtime off, so
office motion validators (including the 150° upside-down guard that stopped
the first attempt, `orientation-01-office-guard-stall/`) neither run on them
nor change. Lab only; nothing here reaches the office runtime.

    python scripts/avatars/tissue_orientation_lab.py list
    python scripts/avatars/tissue_orientation_lab.py run all --label NAME [--jobs 4]

Per scenario: `<scenario>.mp4` (chest-front | chest-side), `response.png`
(free-node centroid in the body frame; zero is the standing rest),
`result.json`, `source/` and the per-view tissue traces.

## orientation-01 (fixed clock, 240 Hz tissue, stabilized-volume asset)

All 20 captures complete; tissue returns to its standing rest after every
move. Directions are right (side-left moves both breasts toward her left,
face-down forward and away from the standing sag). Magnitudes are not:

| scenario | held / peak centroid shift | peak node | volume ratio |
|---|---|---|---|
| face-down | 6 mm up, 2 mm forward | 15 mm | 0.90–1.15 |
| face-up | 4 mm up, 1 mm back, 2 mm outward | 10 mm | 0.90–1.16 |
| upside-down | 9.5 mm toward the head | 21 mm | 0.86–1.15 |
| side-left / right | 5–6 mm sideways, 4–6 mm up | 16–17 mm | 0.89–1.15 |
| turn | 2 mm peak | 4 mm | 0.90–1.16 |
| tumble | 10 mm peak | 22 mm | 0.85–1.16 |
| shake side / bounce / twist | 8 / 7 / 3 mm peak | 17 / 17 / 7 mm | 0.77–1.28 |

Real breast tissue shifts several centimetres between standing, supine and
prone, and keeps moving after a bounce stops. Here the tissue moves millimetres
and settles within ~0.3 s of the shake ending: the current material and
attachment (shear 500 Pa, strain stiffening 8, bulk 500 kPa, fixed back) are
far stiffer and more damped than tissue. This is the calibration baseline.
