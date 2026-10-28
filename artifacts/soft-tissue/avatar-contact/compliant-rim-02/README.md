# Compliant chest rim (owner-selected option)

The owner chose to soften the rim after the 7671.35 ms `ContactUnresolved`
showed skin samples embedded 78–89 % in Fixed rim node 52 (tet 587; edge
52–110 is 53.5 mm, so skin ~6 mm inside the rim is mostly rigid). About 10 %
of bound skin on each side has under 50 % free barycentric weight.

`scripts/avatars/soften_tissue_rim.py` changes only the fixed nodes on the
skin side of the boundary (47 per side) to `Compliant`; the posterior cap and
walls (83 / 81 nodes) stay Fixed. Mesh, rest shape, material and bindings are
unchanged. Variants: `assets/tissue/beautiful_assistant.base-compliant-rim-{1e-6,1e-5,1e-4}-prototype.json`.
The selected asset is still the fixed-rim one; nothing is default-enabled.

## Confound found and fixed (`../compliant-rim-01`)

The first sweep reached a 2.9 local volume ratio by 66 ms in the jump at every
compliance (baseline first leaves 0.85–1.2 at 516 ms) with seams of only
0.02 mm. `AverageNodalBoundary` assigned element pressure only to tetrahedra
with two **Fixed** nodes, so softened rim tets fell into the averaged
partition, freeing local volume. The partition now counts any attached
(Fixed or Compliant) node. No committed asset used Compliant attachments; the
baseline jump stays identical in every value (`baseline-jump-comparison.json`).
Test: `compliant_rim_keeps_the_fixed_rim_pressure_partition`. The first
series is kept as logs and `summary.ndjson` only.

## Results with the corrected partition (`summary.ndjson`)

| Compliance (m/N) | Jump peak | Jump max seam L/R | Contact replay stops |
|---|---|---|---|
| fixed (baseline) | 32.56 mm | 0 / 0 | 7671.35 ms ContactUnresolved |
| 1e-6 | 32.29 mm | 0.021 / 0.009 mm | 7732.04 ms ContactUnresolved |
| 1e-5 | 32.45 mm | 0.153 / 0.092 mm | 7757.94 ms nonlinear, depth 6 |
| 1e-4 | 32.90 mm | 0.927 / 0.497 mm | 7764.25 ms nonlinear, depth 6 |

Jump volume extremes match the baseline (about 0.56–1.56). The softer rim
therefore removes the attachment-dominated blocker without changing the
reviewed bounce at 1e-5 or stiffer.

In contact, from 7683 ms the right breast reaches local volume ratios of
0.05–0.22, up to 419 active samples, rim drag of 4–17 mm and node speeds of
1.5–3 m/s. Round-0 penetration per step implies 3–5 m/s closing speed between
hand and skin. The rendered frames (`lab-p-rim1e5-contact-render-front-01`,
PNGs ignored) show both hands cupping the breasts and sinking in. The
unbounded penalty escalation drives these forces; the next step is a capped
contact pressure, not a different rim. Late nonlinear failures at depth 6 sit
at a 1e-16 m correction against a halved limit (previous step also 65 us), with
kilonewton forces: a genuine precision floor caused by those forces.

`traces.tar.gz` holds the seven corrected runs (manifest lists SHA-256; lab
packet copies match the service-owned sources). Geometry snapshots were not
retained.
