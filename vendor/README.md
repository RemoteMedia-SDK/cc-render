# Patched Bevy crates

Copies of the crates.io sources of `bevy_mesh` 0.15.3 and `bevy_pbr` 0.15.3
(MIT / Apache-2.0, licences included), used through `[patch.crates-io]` in the
root `Cargo.toml`. Two changes: a mesh may carry 128 morph targets instead of
Bevy's 64 (CC5 faces carry ~400 morphs; the 52 ARKit channels map to 67; the
first three items below), and a skinned + morphed mesh no longer panics under
TAA (the last).

- `bevy_mesh/src/morph.rs`: `MAX_MORPH_WEIGHTS = 128`.
- `bevy_pbr/src/render/mesh_types.wgsl`: `MorphWeights.weights` is
  `array<vec4<f32>, 32u>` (= 128 / 4); it must match the constant.
- `bevy_pbr/src/render/morph.rs` (`extract_morphs`): each mesh's weights are
  padded to 128, the size the binding reads. Upstream pads to 256 bytes (64
  weights), so a mesh with 64 or fewer morphs at the end of the buffer was bound
  past it (wgpu: "would overrun the buffer").
- `bevy_pbr/src/render/mesh.rs` (`SetMeshBindGroup`): `dynamic_offsets` holds
  5 offsets, not 3. A skinned + morphed mesh drawn with motion vectors (TAA)
  binds batch, skin, morph, previous skin and previous morph offsets, and 0.15.3
  panics with index out of bounds as soon as such a mesh animates.

Bevy 0.17 still has 64. When upgrading Bevy, re-copy both crates at the new
version and reapply these changes (or drop the patch if upstream lifts the
limit).

Cost: Bevy stores each target densely (position, normal and tangent offsets of
every vertex of the mesh, 36 bytes per vertex per target), so GPU memory grows
with the target count: CC5's Beautiful at level 0 needs ~100 MB for 67 targets.
