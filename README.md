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

## What it exports

| Node type      | Input                                                                    | Output                                                       |
|----------------|--------------------------------------------------------------------------|--------------------------------------------------------------|
| `CcRenderNode` | Json `{blendshapes \| skeletal_pose \| barge_in}` + `Audio` (mono f32)   | `Video {format: Rgba32, ...}` + paired `Audio` (offline pairing) or `Video`-only (`realtime_mode: true`) |

Pacing-domain: `ClockedToOutboundMedia` with a wall fallback at
`framerate` Hz.
