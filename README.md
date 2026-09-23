# cc-render — CC5 / glTF avatar renderer (Bevy 0.15)

Standalone Path 3 Rust cdylib that registers `CcRenderNode` into the
[RemoteMedia SDK](https://github.com/RemoteMedia-SDK/remotemedia-sdk)
streaming pipeline registry.

This plugin owns the full Bevy 0.15 + bevy_rapier3d + wgpu stack for
rendering CC5 (Reallusion Character Creator 5) avatars exported through
the `FBX2glTF → prune_morphs → bake_alpha` pipeline. It consumes:

- `{kind: "blendshapes", arkit_52, pts_ms, turn_id?}` — ARKit-52
  blendshape envelopes from an upstream lip-sync node (e.g.
  `Audio2FaceLipSyncNode`, ported in
  [`audio2face`](https://github.com/RemoteMedia-SDK/audio2face)).
- `{kind: "skeletal_pose", joint_quats_xyzw[22], root_pos[3], pts_ms}`
  — SMPL-22 skeletal poses from `KimodoMotionNode` for body motion.
- `{kind: "barge_in"}` / `__aux_port__: "barge_in"` envelopes — snap
  back to rest pose, clear buffered audio.
- `RuntimeData::Audio` (mono f32) — buffered and emitted paired with
  each rendered Video frame so downstream consumers receive
  content-time-aligned A/V pairs.

Body poses use `CC_AVATAR_RETARGET_MODE=calibrated` by default. The renderer
calibrates canonical target frames once from the avatar bind pose, then transfers
complete source world rotations and converts them to CC5 parent-local rotations.
This preserves shoulder roll and independent wrist motion across different bind
axes. The pelvis keeps its anatomical bind orientation. For comparison with the
previous solver, set `CC_AVATAR_RETARGET_MODE=axis_match_chained`;
`CC_AVATAR_FREEZE_LEAF_BONES` applies only to the legacy axis-match modes.
Retargeting preserves the generated pose; it does not correct an implausible
source reach or enforce chair contact.

The Bevy thread free-runs at `framerate` and produces frames into a
ring; the SDK's pacer drives `tick()` at the bound wire clock (typically
the outbound audio media clock, falling back to `framerate` Hz at idle)
which drains the latest frame and emits `RuntimeData::Video {format:
Rgba32, ...}` stamped with the configured `video_stream_id` (default
`"avatar.video"`). The first time the scene fully settles the node
emits a one-shot `{kind: "renderer_ready"}` Json envelope so the upstream
driver can gate its input gate on actual readiness.

## Use from a manifest

```json
{
  "version": "v1",
  "plugins": ["cc-render@v0.1.0"],
  "nodes": [
    {
      "id": "avatar",
      "node_type": "CcRenderNode",
      "params": {
        "glb_path":        "avatars/assistant.alphabaked.glb",
        "arkit_map_path":  "avatars/assistant.arkit_map.resolved.json",
        "framerate":       30,
        "video_stream_id": "avatar.video",
        "width":           1280,
        "height":          1280,
        "scene_glb_path":  null,
        "realtime_mode":   false
      }
    }
  ]
}
```

The SDK resolver expands `cc-render@v0.1.0` to
`github.com/RemoteMedia-SDK/cc-render`, fetches `plugin.toml`, then
falls through to `release-manifest.json` for the platform-specific
prebuilt `.so` / `.dylib` / `.dll` asset.

### Required assets

The renderer expects a CC5 avatar GLB + matching ARKit blendshape map.
See [`avatars/README.md`](https://github.com/RemoteMedia-SDK/remotemedia-sdk/blob/main/avatars/README.md)
upstream for the `FBX2glTF → prune_morphs → bake_alpha` preprocessing
pipeline that produces:

- `*.alphabaked.glb` — opacity baked into diffuse alpha, morph targets
  pruned to the renderer's working set.
- `*.arkit_map.resolved.json` — per-mesh morph-name lookup that maps
  the ARKit-52 channel names to the GLB's morph target indices.

## Build the cdylib locally

```bash
git clone https://github.com/RemoteMedia-SDK/cc-render
cd cc-render
cargo build --release
# → target/release/libcc_render_plugin.so
```

First build pulls in the Bevy 0.15 + bevy_rapier3d + wgpu stack and
takes 3–5 minutes; subsequent incremental builds are seconds.

### Runtime tunables (environment variables, all optional)

| Var                          | Purpose                                                    |
|------------------------------|------------------------------------------------------------|
| `CC_RENDER_FAST=1`           | Drop `ScheduleRunnerPlugin` tick to ZERO (offline only).   |
| `CC_PHYSICS_DISABLE=1`       | Skip Rapier install — disables hair/skirt/breast jiggle.   |
| `AVATAR_BEVY_GPU_INDEX=N`    | Pin Bevy to the Nth wgpu adapter (multi-GPU hosts).        |
| `CC_AVATAR_ENVMAP_DIR=path`  | Override the `env://` asset source for KTX2 envmaps.       |
| `CC_AVATAR_FOLLOW=1`         | Camera follows the pelvis bone.                            |
| `CC_AVATAR_FIT_FRAME=1`      | Auto-fit camera to avatar AABB.                            |
| `CC_AVATAR_DEBUG_JOINTS=1`   | Overlay joint skeleton + labels.                           |
| `CC_SCENE_POS=x,y,z`         | Scene GLB translation (default `0,0,0`).                   |
| `CC_SCENE_ROT_DEG_Y=<deg>`   | Scene rotation around Y (default `0`).                     |
| `CC_SCENE_SCALE=<f>`         | Scene uniform scale (default `1.0`).                       |
| `CC_SCENE_KEEP_LIGHTS=0`     | Strip authored lights from the environment scene.          |

## Renderer correction A/B diagnostics

Run fresh `office_acceptance` sessions with the same avatar, scene, seed,
choreography, camera and speech settings. These switches are independent:

| Switch | Default | Disabled behavior |
| --- | --- | --- |
| `CC_AVATAR_ROOT_INCLINATION_MAX_DEG` | `0` (disabled) | `0` bypasses the renderer pelvis-inclination cap; `30` restores the former cap. |
| `CC_OFFICE_CONTACT_CORRECTIONS` | enabled | `0` skips post-retarget hand/foot contact IK, including its release residual. Read at renderer startup. |

For the two arms, prefix the normal acceptance command with:

```sh
# A: former pelvis cap plus contact IK
CC_AVATAR_ROOT_INCLINATION_MAX_DEG=30 CC_OFFICE_CONTACT_CORRECTIONS=1 \
  target/debug/examples/office_acceptance avatar.glb arkit-map.json corrections-on.mp4
# B: pelvis cap and contact IK disabled
CC_AVATAR_ROOT_INCLINATION_MAX_DEG=0 CC_OFFICE_CONTACT_CORRECTIONS=0 \
  target/debug/examples/office_acceptance avatar.glb arkit-map.json corrections-off.mp4
```

Both arms retain ARDY guidance/source-pose enforcement, renderer seat-root
support, collision handling, seated idle smoothing, finger grips and secondary
motion. This is not an uncorrected/raw-ARDY mode. Keep future handoffs disabled.
Matching seeds control random draws but do not make live runs identical: changed
displayed contacts can change action completion, requests and subsequent motion.
For attribution, also test each switch separately; exact renderer-only comparisons
require replaying identical poses and contact/controller state.

The selected default is **pelvis cap OFF, contact IK ON**. In the seed-73129
`beautiful_assistant` transition comparison with concurrent Audio2Face, that
combination completed all eight actions; pickup took about 4.4 seconds. Contact
IK OFF alone took about 20.7 seconds for pickup, and both corrections OFF timed
out at pickup. These are single-run results, not a reliability guarantee. User
visual review preferred the uncapped pelvis. Source support and the remaining
renderer corrections are still enabled.

## What it exports

| Node type      | Input                                                                    | Output                                                       |
|----------------|--------------------------------------------------------------------------|--------------------------------------------------------------|
| `CcRenderNode` | Json `{blendshapes \| skeletal_pose \| barge_in}` + `Audio` (mono f32)   | `Video {format: Rgba32, ...}` + paired `Audio` (offline pairing) or `Video`-only (`realtime_mode: true`) |

Pacing-domain: `ClockedToOutboundMedia` with a wall fallback at
`framerate` Hz.
