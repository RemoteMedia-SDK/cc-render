# Toronto high-rise condo

**Current priority:** visual detail work is paused at the user's request.
Focus on manipulable fridge doors, drawers, cabinets, room doors and bed sheets.
See [condo manipulation](condo-manipulation.md) for the verified asset inventory,
cloth preparation and runtime integration gates. The visual-polish queue below
is deferred while this work proceeds.

Design started 2026-10-15. Selected direction: **warm modern**. This replaces the visible office/home with a furnished Toronto condo and an animated waterfront exterior. Implementation is now an opt-in prototype; the requirements below remain the target, not a claim of completion. See the [capture gallery](../../artifacts/toronto-condo/index.html) for dated visual evidence and remaining defects, and the [capture workflow](../../scripts/condo/README.md) for reproduction commands.

Workspace/entry review: gallery 155 preserves their placeholders; 156–158 show
the authored oak desk, chair, equipment, shelves, cabinet, bench and closed entry
door, including the corrected veneer and day/night lighting. Static geometry
checks preserve the checked entry and adjacent-room approaches. These are visual
and layout reviews, not sitting, desk-use, door-opening or avatar-traversal
acceptance. Bathroom review 160 preserves the placeholders; 161 adds the oak
vanity, recessed basin, mirror/sconces, glass shower and toilet. Its static
approach checks pass. Reviews 162–163 fix diffuser bindings and restore continuous
sconce emission; mirror reflections and hard practical-light shadows remain
unfinished. Default-scene replacement, avatar floor contact and
sustained performance validation are still outstanding.

## Experience and setting

A south/southeast corner condo around the 30th floor, approximately 90 m above the street, in a fictional tower near the CityPlace/Harbourfront transition west of the CN Tower. The lake is the main view; the city supplies depth, movement and a sense of place. Oak, pale limestone, cream upholstery, muted green plants and warm lamps keep the interior comfortable against cool blue water and glass towers.

Use a plausible Toronto setting, not a claimed reconstruction of a particular address. The exact tower position and elevation must be selected against terrain/building data before finalizing sightlines. Lake Ontario and the islands belong to the southern view; the CN Tower belongs in the appropriate east/northeast side view, not artificially relocated into the lake panorama. View obstruction by nearer towers is a first-stage design constraint.

Queens Quay is the waterfront's east–west main street; Waterfront Toronto identifies its central waterfront between Bathurst and Lower Jarvis, south of Lake Shore Boulevard. Use these relationships for the composition. [Queens Quay West](https://www.waterfrontoronto.ca/our-projects/queens-quay-west), [Central Waterfront](https://www.waterfrontoronto.ca/our-projects/scope-scale/central-waterfront).

Primary views:

- Conversation: avatar in the living room, lake and moving traffic visible beyond her, face correctly exposed.
- Work: desk at the side of the main living space, with daylight arriving from the side rather than a distracting bright window directly behind the monitor.
- Balcony: seated or standing eye height reveals the harbour, islands, nearby buildings and street activity below.
- Evening: warm interior pools of light, distant windows, headlights and restrained water reflections.
- Rain: softened skyline, wet balcony and roads, rain on exterior glass; the interior remains dry.

## Layout

Target footprint: **12 × 10 m gross interior**, plus **12 × 2.4 m balcony**; approximately 120 m² before wall deductions and 29 m² outdoors. Ceiling target: 2.9 m. These are design dimensions, not a building-code certification. Keep all five existing home functions: kitchen, work area, bathroom, bedroom and living room.

| Zone | Placement and contents | Interaction intent |
| --- | --- | --- |
| Living room | South/east; low cream sofa, stone coffee table, rug, reading chair, plants | Talk, sit, read; preserve a broad lake sightline |
| Kitchen/dining | West; oak cabinetry, pale stone island, compact dining table | Food and drink props; connected route to living area |
| Work nook | North/centre; oak desk, monitor, office chair, shelves | Work and conversation without blocking circulation |
| Bedroom | Northeast; bed, wardrobe, textile accents, east-facing window | Private room with an independent entrance |
| Bathroom | Northwest; shower, vanity, toilet | Retain a reachable room and fixture approaches |
| Entry/storage | North/centre | Clear arrival area, coat storage, hidden service core |
| Balcony | South; outdoor chair pair, small table, planters | Genuine walkable extension with lake view |

Use a continuous circulation route of roughly 1.1–1.2 m where possible and a generous balcony opening. Final clearances must be checked against the actual avatar/colliders. Place the sofa and plants below the important viewing cone. Sliding door, guardrail and glazing must have actual geometry and collision semantics; a transparent surface is not automatically traversable.

The companion [Blender blockout](assets/toronto-condo/toronto-condo-blockout.blend) and [preview](assets/toronto-condo/toronto-condo-blockout.png) are an editable spatial study. They use simple furniture and cutaway walls, omit exterior geography and production glazing, and are not the finished visual target. The roofless preview is for reviewing layout; the runtime condo needs ceilings and a complete exterior shell. No avatar animation or active weather/traffic is claimed by this study.

## What exists today

Checked against this checkout, rather than relying on older office acceptance results:

| Existing component | Reuse or change |
| --- | --- |
| `Cargo.lock`: Bevy 0.15.3; Rapier 0.28 | Keep the current engine for the first implementation. Do not assume examples from newer Bevy versions apply. |
| `RendererConfig::scene_glb_path`, `bevy_app/mod.rs` asset source, `camera.rs::setup_scene` | Reuse environment GLB loading and the separate environment/avatar roots. |
| `office/systems.rs::install` and `spawn_office` | Currently load an object array and spawn primitive meshes/colliders. Introduce a scene-package path that owns visuals, colliders and metadata together. |
| `office/home.rs::spawn_surfaces` | Currently owns procedural floor and room lights. Disable these for the condo package, preventing duplicate floors/furniture/lights. |
| `bevy_app/physics/world.rs::spawn_ground` | Currently creates at least a 50 × 50 m invisible ground. Replace this for the condo with explicit floor/balcony slabs so empty air does not become walkable. Check generic play-volume walls/ceiling too. |
| `camera.rs::setup_scene` | Existing portrait lights, fixed Pisa environment lighting, short shadow range and default projection need a condo lighting/camera profile. |
| `office/metadata.rs`, `navigation.rs`, `scene_context.rs` | Reuse geometry queries, stable object identity and observed-state reporting where compatible with current motion policy. |

Loading a GLB alone does not establish matching physics or semantic interaction. `CC_OFFICE_SCENE` currently replaces only an object array; it is not a complete condo package format. Existing `CC_SCENE_KEEP_LIGHTS` controls imported lights but does not disable the renderer's own portrait rig.

Ripwire identified the Bevy/office/physics integration boundaries. Its graph does not fully resolve ECS callback registration, and the scoped scan did not discover test files; zero static callers/test reach is not proof of isolation or lack of tests. Treat startup and physics changes as a staged integration change requiring runtime evidence.

## Blender and asset workflow

Blender is available locally: **5.3.0 Beta**, build `ecaf39c37b28`. Use it for the planning study. Pin an exact production Blender build/exporter after a Bevy 0.15.3 import smoke test; do not depend on beta-only export features. QGIS was not found on PATH and is optional to install for geographic preparation.

1. **Geographic blockout:** obtain a bounded City of Toronto massing extract; select the tower site, street elevation and lake outline. Use QGIS/GDAL or a small coordinate-conversion script to crop/reproject data before Blender import. Keep the source geographic reference and one local origin in the package.
2. **Architecture:** author walls, glazing, sliding door, balcony, slabs and ceiling in meters. Use modular bays and linked mesh instances. Keep full walls in the source file even when hiding them for layout previews.
3. **Furniture/materials:** build the main furniture to fit the plan; supplement with licensed assets. Bake procedural oak/stone/fabric to glTF-compatible PBR maps. Use restrained bevels and normal maps rather than excessive mesh detail.
4. **City:** use imported massing as silhouette/scale guidance, then simplify and create façade atlases. Hand-author only visible landmark silhouettes and near buildings. Do not import all of Toronto at full detail.
5. **Export:** export separate static interior, city chunks, props and reusable vehicle assets as GLB. Bake/realize Geometry Nodes output where necessary. Export named custom properties as `extras`, then generate a versioned sidecar manifest from the same authoring data.
6. **Validate:** run a Khronos glTF validator and load representative assets in this exact renderer before full asset production. Check scale, axes, object IDs, textures, glass, lights and door pivots. Blender/Cycles beauty alone does not establish Bevy visual compatibility.

Blender documents glTF custom properties as extras; this supports stable IDs but does not automatically create Rapier colliders or application behavior. [Blender glTF documentation](https://docs.blender.org/manual/en/4.1/addons/import_export/scene_gltf2.html).

Recommended sources/tools:

| Tool/source | Purpose and constraint |
| --- | --- |
| Blender | Master architecture, props, city reduction, material baking and reference renders |
| City of Toronto 3D Massing | Building scale and location reference; inspect tile coverage, date, format and height datum before use. [Dataset](https://open.toronto.ca/dataset/3d-massing/) |
| QGIS/GDAL, optionally BlenderGIS | Prepare geographic data; BlenderGIS compatibility with the chosen Blender build is unverified, so it must not be a required dependency |
| Poly Haven | Candidate oak, stone, fabric and lighting assets; its asset library is CC0. Select and record actual assets during production. [Asset licence](https://polyhaven.com/license) |
| OpenStreetMap, if needed | Road/shoreline gaps; preserve attribution and assess derivative database obligations for distributed extracts. [Source terms](https://www.openstreetmap.org/copyright) |
| glTF validator; optional glTF-Transform/KTX tools | Validation, deduplication and texture processing; test compressed assets with this loader before adopting compression extensions |
| FFmpeg | Acceptance recordings; verify the selected executable supports the desired encoder |

No paid asset, add-on or service is required for the first slice. No installs or asset purchases have been made for this plan.

## Scene package and coordinates

Proposed package, not an existing configuration interface:

```text
scenes/toronto_condo/
  scene.json                 # version, asset references, geographic origin, bounds
  interior.glb               # shell and static furnished interior
  city/near.glb               # nearby roofs and towers
  city/mid_*.glb              # waterfront and selected downtown chunks
  city/far.glb                # islands and distant silhouettes
  props/*.glb                # independently identified interactable objects
  traffic/vehicles.glb        # reusable meshes, not one mesh copy per car
  traffic/lanes.json          # sampled lanes, junctions, spawn/despawn regions
  collision.json             # authored simple shapes, surfaces and zones
  environment.json           # time/weather presets, water, sky, lighting
  provenance.json            # source URLs, licences, dates, export/build hashes
```

Keep Blender source and reproducible export tooling in the asset-authoring tree; avoid embedding bulky binaries in Rust source. Determine repository/LFS or asset-store delivery before production assets grow. The small planning `.blend` is an exception stored beside this document for review.

Coordinate convention: condo floor at runtime **Y=0**, **+X east, +Z south**. Blender uses **+X east, +Y north, +Z up**; standard glTF conversion maps the two. Store physical tower elevation separately, placing city ground around runtime Y=-90 m after datum conversion. This keeps avatar coordinates near the origin and preserves real exterior parallax. Do not move or height-correct avatar motion to manufacture elevation. Use a single verified transform for mesh, collision and semantic geometry; account explicitly for survey north versus façade orientation.

Scene metadata describes stable IDs, actual surfaces, colliders, rooms, door state, weather exposure and destination regions. It must not export `seat_pelvis`, foot arrangements, timed root tracks or pose targets. Visual city geometry stays outside the avatar's interaction registry and local physics world.

## Exterior simulation

| Layer | Initial scope | Rendering approach |
| --- | --- | --- |
| Condo/balcony | Approximately 0–30 m | Detailed materials and local collision; good shadows and contact readability |
| Near city | Approximately 30–500 m | Real 3D roofs/towers, road lanes, cars, visible shoreline details |
| Mid city/waterfront | Approximately 0.5–3 km | Simplified instanced buildings, façade atlases, sparse boats and transit |
| Far scene | Beyond approximately 3 km | Island silhouettes, low-detail geometry or carefully placed impostors, atmospheric haze |
| Sky/lake | Horizon scale | Sky dome and finite/tiled water mesh sized for all approved cameras; no visible plane edge |

Distances are starting budgets, not requirements to populate every metre. Audit actual visibility from seated, standing and balcony cameras before authoring details. Keep recognizable geometry and parallax; a static panorama alone cannot provide the requested experience.

**Traffic:** deterministic lane-following simulation with vehicle spacing, speed variation, junction signals where visible, headlights/brake lights and off-camera spawn/despawn. Begin with roughly 40–100 active vehicles across visible roads, then profile. Update simulation at 10–20 Hz with rendering interpolation; batch repeated meshes/materials and remove distant shadows. Traffic is simulated ambience, not claimed live Toronto vehicle positions. Add a streetcar only on a geographically verified rail corridor. Cars must remain visible at the actual viewing height and output resolution; do not depend on tiny invisible moving dots to meet the requirement.

**Lake:** wind-driven normal layers, roughness, Fresnel response, sun glint, muted shoreline reflection and slow boat/wake motion. Wind direction and strength drive both water and vegetation. Start with environment reflections; planar reflections or screen-space reflections are an optional measured upgrade, not a baseline promise. Separate lake animation from collision; the avatar's supported world is the condo and balcony.

**Weather:** deterministic clear, overcast, rain, fog and snow presets; smoothly transition sky, sun, light probes/environment intensity, visibility, particles, exterior wetness and lake state together. Use exterior emitter regions, roof/overhang exclusion and interior masks so rain/snow never passes through the ceiling. Camera-wide distance fog alone cannot represent dense exterior fog through a window without affecting the room; test a bounded exterior haze solution where that matters. Snow accumulation and seasonal vegetation are later polish.

**Time:** Toronto timezone (`America/Toronto`), geographic solar orientation, manual time override and deterministic playback clock. Night brings emissive window patterns and vehicle lights without thousands of individual shadow-casting lamps. Handle bright windows/dark rooms with a stable exposure policy and restrained interior fill.

Optional real-weather mode can periodically fetch Open-Meteo temperature, cloud, precipitation, wind and visibility outside the render loop, then blend toward that state. Cache the last result and fall back to a named preset on failure; expose source and timestamp. This is model-derived weather data, not a street-level sensor feed. Commercial API use requires the appropriate plan and data attribution. [Open-Meteo docs](https://open-meteo.com/en/docs), [usage terms and pricing](https://open-meteo.com/en/pricing).

**Sound:** optional distant traffic, wind and rain, attenuated with the balcony door closed. Soundscape integration belongs in the host audio path and must not compete with speech or introduce echo. Visual exterior activity is the first acceptance target.

## Runtime boundaries

Add a scene-package loader and an environment subsystem alongside the office modules. Proposed responsibilities: `scene_package` (manifest/assets), `environment` (clock/weather), `traffic`, `water`, and `lighting`. Avoid packing these into the already broad `OfficeController` or `setup_scene` functions.

The package loader waits for assets, resolves stable node IDs, spawns collision/semantic state once, validates it, then publishes scene readiness. A missing required asset or malformed package is an explicit loading error, not a quiet fall back to an unrelated office. Keep the old home available as an explicit regression profile while the condo is built; make the condo the product default after acceptance.

Integration seams and required gates:

- Renderer config → package loader → Bevy asset source: missing assets, readiness, relative paths, reproducible startup.
- Blender export → node IDs/transforms → collision and world snapshot: matching extents, no duplicates, round-trip axis/scale fixtures.
- Package → physics world: explicit slabs/railings/ceiling, no oversized hidden floor and no generic wall blocking the balcony.
- Environment state → sky/lights/water/particles/traffic: fixed-seed replay, continuous transitions, no render-thread networking.
- Camera profile → environment: far-scene visibility, shadow distance, exposure and glass readability; isolate avatar and environment systems.
- Geometry → motion service: scene facts and semantic destinations only; ownership/policy tests remain intact.

Local Bevy 0.15.3 sources confirm `DistanceFog` and static prefiltered environment/reflection maps exist. They are building blocks, not a complete dynamic atmosphere or real-time GI system. New custom water/weather materials need compatibility and GPU smoke tests. Do not make an engine upgrade a hidden prerequisite.

## Avatar and interaction requirements

Use **beautiful_assistant** with its matching mesh and ARKit map for acceptance. Preserve the current voice/face pipeline. Furniture, balcony and props are designed around her actual rendered size and geometry.

Follow [model-owned motion](model-owned-motion.md) and `AGENTS.md`: the motion model generates body movement; task code supplies semantic outcomes and allowed scene geometry; validators accept/reject unchanged motion. No scripted approach poses, IK, root snapping, joint clamps or timed pelvis paths. Decorative vehicle/water animation does not authorize an alternate avatar pose writer.

The current motion migration is incomplete. Reliable sit/grab/door manipulation cannot be assumed from historical office demonstrations. Mark those as separate capability gates, retain failed attempts, and report unsupported conditioning explicitly. For the first visual slice, configure the balcony door open; later user-controlled door state must update both visual and collision/navigation state. Avatar-operated opening is a separate manipulation capability.

## Ordered delivery and acceptance

Preview ongoing work in the [screenshot and video progress gallery](../../artifacts/toronto-condo/index.html).
Every render iteration is archived with its original screenshots, capture metrics,
progress notes and known issues. See the [capture publishing workflow](../../scripts/condo/README.md).

| Stage | Deliverable | Exit evidence |
| --- | --- | --- |
| 1. Site and blockout | Geographic massing study, furniture layout, three eye-level camera compositions | Lake and road activity visible at intended height; no misplaced landmarks; reachable room openings; user can review proportions |
| 2. Small runtime slice | One furnished living-room/window/balcony segment in Bevy, beautiful_assistant, a road segment with moving cars and animated water | Recorded rendered view proves scale, import, glass, lighting, movement and exterior parallax before full asset production |
| 3. Package/physics integration | Complete shell, metadata, colliders, readiness and scene selection | No duplicate office geometry; bounded floors; reachable rooms/balcony; no motion-policy regression |
| 4. Complete condo/city | Five functional areas, balcony, nearby towers, roads, islands and landmark views | Interior/side/balcony captures; material consistency; no visible LOD jumps or horizon seams |
| 5. Time/weather | Day/night, rain, fog and snow presets; wetness and linked water/wind | Recorded clear → overcast → rain → night transitions; dry interior; stable face exposure; replayable seeds |
| 6. Interactions | Supported navigation, seating and props, with scene-conditioned generation | Separate source-motion, CC5 playback and observed physical-state evidence; unsupported actions reported honestly |
| 7. Performance/default | Tuned quality tiers and final scene switch | Sustained end-to-end recording and frame/memory report; explicit rollback profile |

Start with a **1080p/30 fps target**, subject to measurement on the actual deployment GPU with voice, body generation and encoding running. Target p95 renderer frame time ≤33.3 ms; measure capture/encoding and end-to-end latency separately. Aim for no more than roughly 8–10 ms incremental environment GPU cost versus the same avatar/camera baseline; reduce quality if the full pipeline misses its target. These are proposed budgets, not measured results.

Initial asset envelope: about 0.5–1.5 million visible environment triangles, approximately 300 visible environment draw calls, ≤512 MiB resident environment textures, 1–2K textures for most assets and 4K only where close views justify them. Treat these as profiling hypotheses. Shadow count, transparency/particle overdraw and materials may matter more than triangles. Quality tiers should reduce distant buildings, vehicle count, particles, shadow range and reflection cost while retaining the lake and perceptible traffic.

Final evidence bundle: daytime conversation, night conversation, balcony, rain and snow clips; room traversal evidence for supported motion; scene/export hashes, seeds, weather/time, GPU/backend, resolution, p50/p95/p99 frame time, dropped frames and memory. Include a ten-minute run to reveal leaking entities, traffic pileups and periodic stalls. Rendered inspection is required: passing metadata/unit tests does not prove an attractive condo or natural avatar motion.

## Recommended first implementation

Build stage 2 immediately after the geographic sightline check: **one excellent lake-facing living-room/window/balcony view with visible moving cars, animated lake water and the real avatar**. This resolves the largest visual and performance risks early. Then extend the proven asset/package pattern through the remaining rooms and weather states.

## Wrap-up checkpoint — 2026-10-16

Completed review work includes furnished living/dining, kitchen, balcony,
bedroom, workspace/entry and bathroom, plus the cream-knit/charcoal-trouser
avatar outfit. Every archived iteration remains in the progress gallery.
The latest implementation adds exterior paving wetness; its scope and evidence
are documented in the capture workflow. This remains an opt-in prototype.

Next work, in priority order:

1. Continue exterior-weather polish after reviews 166–167: the road sheen is
   restrained, the living-room clear/rain/night sequence is recorded, and the
   film persists into night. Complete drying and long-run behavior remain unchecked.
2. Fix bathroom mirror reflections and remaining hard practical-light shadows.
3. Resolve garment creases/seams and source-motion floor contact without pose
   correction; verify supported room traversal and interactions.
4. Add rain-on-glass detail and complete the remaining time/weather requirements.
5. Capture the ten-minute GPU/performance evidence, resolve outstanding test and
   quality gates, then review default-scene selection and rollback.

Current Rust library validation: 267 passed, two ignored. The Python suite still
has six passing tests and one existing motion-ownership failure; it was not
weakened. The broad dirty-worktree quality gate still reports 17 major findings.
No completion or production-readiness claim is made.
