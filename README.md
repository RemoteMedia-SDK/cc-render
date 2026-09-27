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

## Model-owned motion migration (breaking)

See [the ownership architecture](docs/design/model-owned-motion.md).
Production playback no longer registers contact IK, seated body smoothing,
finger-grasp posing or the scripted furniture-push actuator. Root collision
validation rejects an unchanged candidate instead of clipping it or moving it
onto a seat. The pelvis inclination cap has no production call path.

`CC_AVATAR_ROOT_INCLINATION_MAX_DEG` and `CC_OFFICE_CONTACT_CORRECTIONS` no
longer enable corrections. Historical A/B results are not compliant-motion
acceptance evidence.

The replacement worker accepts only closed `motion_goal` v2 requests. Submit an
initial semantic goal to `CcRenderNode`; its native request/ack driver forwards
that goal at the displayed clock, with one request in flight. Route worker
`skeletal_pose`, `motion_tick` (including request `pts_ms`), and `motion_error`
responses back to the node. The renderer no longer emits the old controller's
body-prescriptive request. Core40 playback retains complete 40-frame batches.

**Legacy action buttons, timeline edits and `office_acceptance` remain unsupported
and fail explicitly.** Do not translate their authored timelines into goals.
The adjacent SDK's `scripts/avatars/docs/native-motion-integration.md` documents
Core40 startup, explicit squat-to-stand wording, tests and deployment limits.
Rebuild the plugin and restart the application/worker to activate source changes.

## What it exports

| Node type      | Input                                                                    | Output                                                       |
|----------------|--------------------------------------------------------------------------|--------------------------------------------------------------|
| `CcRenderNode` | Json `{blendshapes \| skeletal_pose \| motion_goal \| motion_tick \| barge_in}` + `Audio` (mono f32)   | `Video {format: Rgba32, ...}` + paired `Audio` (offline pairing) or `Video`-only (`realtime_mode: true`) |

Pacing-domain: `ClockedToOutboundMedia` with a wall fallback at
`framerate` Hz.

### Authored home scene

`CC_OFFICE=1` with physics enabled now loads a furnished 10 × 12 metre home.
The roof and upper front wall are cut away for an overview; the remaining walls
and 1.4 metre doorways participate in both physics and navigation. Coordinates
are metres, Y-up; the floor is at Y=0. The avatar starts in the open work area.

| Area | Furnishings / action target IDs |
| --- | --- |
| Kitchen, rear left | `kitchen_sink`, `stove`, `fridge`, `dining_table`, `dining_chair` |
| Work area, front left | Existing `desk`, `chair`, `shelf`, `sofa`, `mug`, `book`; computer, keyboard and mouse on the desk |
| Bathroom, rear centre | `shower`, `toilet`, `bathroom_vanity` with mirror |
| Bedroom, rear right | `bed`, `nightstand` with lamp, `wardrobe` |
| Living room, front right | `living_sofa`, `coffee_table`, `television` and media console |

The original office objects and interaction coordinates are preserved. All new
furniture has reachable walk-to approach points. The dining chair and living
sofa also have sitting anchors; tables, the sink counter, vanity, bed and
nightstand expose placement surfaces. Fixtures are static geometry: appliance
operation, bathing and lying-down animations are not action verbs.

`src/cc_render/office/starter.json` defines the original office furniture;
`home.json` adds the rooms' furniture and walls. Optional `parts[].color` values
provide material overrides while older object arrays remain valid.
`home::ROOMS` defines room bounds and entrances; `home.rs` supplies floor finishes
and lighting. `CC_OFFICE_SCENE=/absolute/objects.json` replaces the bundled objects
and retains the original office floor and light instead of adding the home shell.

The default office camera frames the entire home. Override
`CC_AVATAR_CAMERA_POS=x,y,z`, `CC_AVATAR_CAMERA_LOOK_AT=x,y,z` and
`CC_AVATAR_CAMERA_FOV_DEG` for closer views. Produce a PNG using the actual renderer
without starting ARDY or a speech worker:

```sh
cargo run --example home_snapshot -- avatar.glb arkit-map.json home.png
cargo test --lib
```

The home tests check every room entrance and furniture approach with avatar
clearance, routes between rooms through doorways, enclosure, floor coverage,
spawned furniture, lights and part materials.

### Query a navigation path

Walking already routes around obstacles and supplies timed `root_waypoints` to
ARDY. `generate_waypoint(from, to)` exposes that planner as a **read-only preview**.
Both endpoints accept an exact object ID, `"avatar"`, or `[x, y, z]` world metres.
Object IDs resolve to their transformed approach points; furniture remains an
obstacle. Coordinates are projected onto the floor for 2D XZ navigation.

Send this JSON to `cc_render.in` (native JSON or the tagged Python JSON channel):

```json
{"kind":"generate_waypoint","request_id":"path-1","from":"avatar","to":"bed"}
```

The correlated `cc_render.out` reply is
`{"kind":"waypoint_path","request_id":"path-1","ok":true,"path":{...}}`.
The path contains:

- `waypoints`: ordered world `[x, floor_y, z]` endpoints and corners.
- `root_waypoints`: `[x, pelvis_y, z, seconds]`, using the same 0.8 m/s walking
  timing and 0.95 m pelvis height as the walk action. Seconds start at zero;
  an avatar origin also reserves time for its observed initial turn. An explicit
  point/object origin assumes it already faces the first segment.
- `distance_m`, `duration_sec`, `session_id`, `generation_revision`, and
  `snapshot_pts_ms` identifying the observation used for planning.

Errors return `ok:false` and `error:{code,message}`. Codes include
`invalid_request`, `invalid_position`, `scene_unavailable`, `avatar_not_ready`,
`unknown_target`, `start_blocked`, `goal_blocked`, and `no_path`. A seated avatar
may be inside its seat footprint: stand up before asking for a walking path from
that position. Queries neither stand the avatar up nor move it.

Native Rust callers can use
`renderer.scene_state().generate_waypoint("avatar", "bed")`, or supply
`[0., 0., 0.]` arrays. The web session provides
`await session.generate_waypoint("avatar", "bed")` with correlated replies,
timeout and disconnect cleanup. A loaded avatar observation is sufficient;
no motion worker is required. Query the actual rendered home from the terminal:

```sh
cargo run --example generate_waypoint -- avatar.glb arkit-map.json avatar bed path.json
```

This returns a snapshot, not a reservation or an activated motion timeline.
Use `perform_action` with `verb:"walk"` and `target:"bed"` to execute a route
against current state. Custom timeline edits still use absolute renderer
milliseconds; the preview's relative seconds cannot be copied into `pts_ms`.
