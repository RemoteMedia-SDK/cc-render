# Patched Bevy crates

Copies of the crates.io sources of `bevy_mesh` 0.15.3 and `bevy_pbr` 0.15.3
(MIT / Apache-2.0, licences included), used through `[patch.crates-io]` in the
root `Cargo.toml`. One change: a mesh may carry 128 morph targets instead of
Bevy's 64 (CC5 faces carry ~400 morphs; the 52 ARKit channels map to 67).

- `bevy_mesh/src/morph.rs`: `MAX_MORPH_WEIGHTS = 128`.
- `bevy_pbr/src/render/mesh_types.wgsl`: `MorphWeights.weights` is
  `array<vec4<f32>, 32u>` (= 128 / 4); it must match the constant.

Bevy 0.17 still has 64. When upgrading Bevy, re-copy both crates at the new
version and reapply these two lines (or drop the patch if upstream lifts the
limit).

Cost: Bevy stores each target densely (position, normal and tangent offsets of
every vertex of the mesh, 36 bytes per vertex per target), so GPU memory grows
with the target count: CC5's Beautiful at level 0 needs ~100 MB for 67 targets.
