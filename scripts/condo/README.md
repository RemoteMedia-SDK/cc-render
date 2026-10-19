# Condo capture progress

Open [the progress gallery](../../artifacts/toronto-condo/index.html) in a browser.

## Additive condo outfit

The isolated scene-conditioning profile for v13 is
`scenes/toronto_condo/calibration-v13/profile.json`, with its corresponding
`renderer-geometry.json`. It includes the original body/heel calibration and both
garment skins. Its exact avatar hash differs from the original shared SDK profile;
use the matching profile when configuring a scene-conditioned service. It has
been exported and checked but is not installed by the standard capture driver.

`package_outfit_calibration.py SOURCE_PROFILE_DIR ORIGINAL_AVATAR OUTFIT CORE_BIND OUTPUT_DIR`
rebuilds an additive outfit profile in a new directory, rejecting overwritten
outputs, changed original assets and mismatched original calibration. It verifies
and preserves the original contact patches and exports the extended hierarchy
and added meshes. The profile covers body, heels and added garments; it does not
add other original hair/eye meshes or morph deformation to collision admission.
`check_avatar_calibration.py PROFILE PACKETS OUTPUT --avatar OUTFIT` checks its
native-packet parity and writes a fixture for the Rust
`exported_native_geometry_matches_python` test using `CC_GEOMETRY_FIXTURE`.

The selected outfit is a cream sleeveless knit top and charcoal trousers.
`build_outfit.py` authors garment meshes in Blender and `outfit_package.py`
appends them as separate skinned nodes to a new avatar GLB. It retains every
original mesh, material, morph target, node transform, skin, animation and
embedded binary byte. It does not re-export the original avatar through Blender.
The original `beautiful_assistant.smpl_aligned.glb` and resolved ARKit map remain
the canonical inputs. No body motion, calibration or runtime pose writer changes
are needed for these additional render meshes.

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/build_outfit.py -- \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  scenes/toronto_condo/beautiful_assistant.condo-outfit-NEW.glb
```

Use a new output name on each iteration: the builder refuses to overwrite an
avatar. It saves the editable garment-only `.blend`, a `.json` preservation
report, and the combined `.glb`. Pass that variant as the capture command's
avatar argument with the **same original ARKit map**. Original avatar source hash:
`23060843839654603f26d7aae6a955cf09c86bb2b39de8bf70bfc5e70d1062a9`.

The top copies source skin weights before reshaping; Blender propagates that
correspondence through its garment cuts and thickness. Trousers use interpolation
on the nearest rest-mesh triangle. Both normalize to four influences for the
existing renderer. The top's rest geometry receives
clearance from the source body before export. This is static asset authoring,
not an animated collision solver or body correction. The current garment is
skinned, without cloth simulation, runtime collision or body-morph fitting.
Original V-stitch colour/normal textures are embedded in the GLB and packed in
the editable Blender material. Existing source-body contact profiles do not include its
new outer surface; this is a visual variant, not a newly certified interaction
profile. Floor contact remains an independent unresolved issue.

Gallery 121 preserves the first rejected chest/hem fit. Gallery 122 preserves
the looser front and the remaining shoulder/back-hem intersections during a
full unchanged 21.8-second turn-and-conversation recording. Each trial includes
the exact authoring source and source/variant hashes. Do not treat binary
preservation alone as evidence of visually acceptable clothing.

Run the independent integrity audit on an exported variant:

```sh
python scripts/condo/check_outfit.py \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  scenes/toronto_condo/beautiful_assistant.condo-outfit-v13.glb \
  artifacts/toronto-condo-captures/outfit-integrity.json
```

It checks original-content preservation, unchanged bind transforms, valid
four-weight skinning, nondegenerate triangles, embedded texture decoding and
negative controls that must reject modified source data. It does not certify
garment/body collision or arbitrary future motions. Gallery 123 fixes the large
back intersections; 124–125 introduce the knit finish and expose the high
armhole pattern that is subsequently lowered in variant 5.

Gallery 126 records all 436 original packets over 21.8 seconds with variant 5;
the maximum observed pelvis-coordinate error against source interpolation is
`1.11e-7 m`, with no stale packets. The underrun counter reaches six after
the last 21800 ms source pose. Gallery 127 provides a closer daylight material
and armhole view. The original avatar's 116,228,124 embedded binary bytes are
unchanged. The knit top and trousers add 43,558 triangles, with no degenerate
triangles found by the asset audit. This is still a review variant: shoulder
drape, full-motion cloth intersection and the independent floor-contact issue
are not declared solved.
Gallery 128 checks the same outfit under the night preset; its eight-second
capture has no stale packets or underruns. Night face exposure and noisy distant
window lighting remain scene-polish items.

The current review asset is **`beautiful_assistant.condo-outfit-v13.glb`**
(SHA-256 `aaca98681c5c1e950be0830503d8803e8c00d33932e7c49a891559ca94e564c7`).
Its editable garment source is the adjacent `.blend`; use the original resolved
ARKit map. The top now uses a continuous body-derived shell, shaped front,
curved armholes, 6 mm shoulder ease, larger torso ease and consistent cylindrical
knit UVs. This is clothing authoring in rest space, not a body or motion repair.
The builder checks four closed, unbranched boundary loops before adding fabric
thickness, then the independent export audit checks skinning, textures, original
content preservation and triangle areas. Variant 13 adds 43,846 triangles.

Iterations 133–136 preserve rejected cuts and a controlled weight-only comparison;
the comparison showed that the visible side opening came from the cut geometry.
Iteration 137 closes the side panels. Iterations 138–139 expose excess rear-arm
fabric, and 140 preserves a failed audit for three tiny cut-edge triangles.
The cleanup in variant 13 passes the unchanged audit with zero degenerate
triangles. Iterations 141–142 provide close and full-room motion review. Fine
side creases and the back hem seam still need polish; these recordings do not
certify arbitrary motions, cloth collision or floor contact.

## Shared sky and lake reflection

Gallery 116 is the pre-change balcony baseline; 117–118 preserve the first day
and night trials, and 119–120 review softened wave normals. `sky_common.wgsl`
shares the procedural sky colour calculation between the dome and lake.
The water uses a nonmetallic surface with approximately 0.02 normal-incidence
reflectance, plus an artistic Fresnel blend with that sky approximation. It does
not reflect the city, shoreline or other scene geometry. Wind strength controls
both ripple drift and normal amplitude. These captures establish the reviewed
day/night appearances, not physically calibrated radiance. Runtime transitions
are separately recorded in iterations 130–132 below.

## Progress archive

It works directly from disk, including screenshots and video playback. Refresh
after publishing another iteration. Entries are newest first, with UTC capture
times derived from the first screenshot's modification time.

After **every render iteration**, archive its screenshots before changing the
scene again:

```sh
python scripts/condo/publish_progress.py /tmp/my-condo-capture \
  --title 'Living room — lighting refinement' \
  --progress 'Describe the change visible in this capture.' \
  --remaining 'Describe known defects and acceptance still outstanding.'
```

The source is a capture directory from `condo_capture` or a single PNG from
Blender. Use repeatable `--progress` / `--remaining` flags for multiple notes.
For a Blender concept add `--status Concept --medium 'Blender concept render'`.
The default status is Prototype; use Accepted only after actual visual review.

The publisher copies **every PNG**, plus `view.mp4` and `capture.json` when
present, and all JSON/NDJSON/log evidence files. It keeps dated iteration folders, an `entry.json` with SHA-256 hashes,
the cumulative `progress.json`, and an offline `index.html`. Existing iteration
folders are never overwritten. Original capture metrics and evidence limits
are retained; arrival timings must not be presented as GPU frame timings.

The gallery is evidence, not the editable scene source. Blender sources and
scene packages live elsewhere in this project. Keep early, failed and rough
captures so later improvements remain comparable.

For a failed run with no screenshots, put its original log in the capture
directory and publish with `--status Failed`. Do not label a recording Accepted
merely because the encoder completed: inspect the log for asset/shader errors,
review the rendered frames, and check motion evidence separately.

## Rebuild and capture

From `cc-render`, generate textures with `python scripts/condo/textures.py`, then
run Blender in background mode with `--python scripts/condo/build.py`. The export
checks that textured meshes have usable UV coordinates. Build the capture tool:

```sh
cargo build --offline --example condo_capture
AVATAR_BEVY_GPU_INDEX=1 CC_CONDO_VIEW=living CC_CONDO_SECONDS=8 \
  target/debug/examples/condo_capture \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  ../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json \
  scenes/toronto_condo/scene.json /tmp/my-condo-capture
```

Use a new output directory for every run. Available authored views are `living`,
`balcony`, `interior`, `bedroom`, `workspace`, and `bathroom`. `CC_CONDO_NATIVE_PACKETS` accepts an original
native recording for diagnostic playback; without it, the avatar stays in bind
pose. It does not synthesize or repair motion. The capture archives scene/input
metadata and asset hashes before starting the renderer. Package environment maps
resolve automatically; an explicit `CC_AVATAR_ENVMAP_DIR` overrides them.

`CC_CONDO_WEATHER` selects `clear` (default), `overcast`, `rain`, `fog`, `snow`,
or `night`. Presets control sun/ambient light, exposure, environment reflection
intensity, distance haze, cloud cover and water drift. Rain and snow add exterior
particles with an exclusion around the interior and covered balcony. These are
prototype presets: exterior paving has a cosmetic wet film; rain on glass and
snow accumulation are not implemented.
Night adds scenic facade window emission as described below. Their visual acceptance must be established by captures,
not by the parsing and particle-boundary tests. `CC_CONDO_EV100` overrides a
preset's exposure for diagnostic comparison.

`CC_CONDO_AO=1` enables a diagnostic Ultra-quality screen-space ambient
occlusion pass with FXAA, replacing the package camera's 4× MSAA. Gallery
iteration 33 shows only a subtle contact-shading improvement, so it remains
opt-in; thin geometry and noise still need broader review. The portrait
camera is unaffected. This pass does not change avatar motion.

The package renderer clones the avatar's blended hair/scalp/brow materials
and uses alpha-to-coverage with the default 4× MSAA. It changes material
handles once during loading, rather than mutating imported material factors
in place. The source avatar, textures, skeleton and motion are unchanged.
Gallery iterations 50–52 isolate the old speckling, show the improved wide
view, and check the face from the front. Fine skin grain remains in the close
view; this is not complete avatar acceptance. Bevy falls back to a 0.5 alpha
mask when MSAA is off, so the optional AO/FXAA path needs its own hair review.

Clear-day exposure is EV100 9.6 with ambient brightness 160, selected from
the paired living-room and balcony reviews in gallery iterations 46–47.
Lower ambient preserves the baked contact shading; brighter exposure lifts
the room without clipping the sampled floor, rug, ceiling or facade regions.
Iterations 46–47 use the diagnostic lightmapped package described below;
iteration 49 verifies the standard package with the same default lighting.
This is a lighting refinement, not finished-scene acceptance; avatar hair,
material depth and the other weather presets still need work.

For a slow software-rendered composition check, set `WGPU_BACKEND=vulkan`,
`AVATAR_BEVY_GPU_INDEX=0`, `CC_CONDO_WIDTH=960`, `CC_CONDO_HEIGHT=540`,
`CC_CONDO_FRAMES=3`, and `CC_CONDO_TIMEOUT=600`. Select the actual available
adapter; index 0 is not guaranteed to be a CPU on another host. Defaults remain
1920 × 1080 and 30 encoded frames per requested second. A short or software
capture is visual diagnostic evidence, not a real-time performance or motion
acceptance test. The capture report records actual frame count, dimensions and
wall time. Existing output directories are rejected to preserve previous runs.

Public geography can be regenerated with `geography.py water.geojson roads.geojson`
using Shapely and PyProj. Exact source URLs, attribution and the distinction
between surveyed geometry and scenic approximations are in
[`provenance.json`](../../scenes/toronto_condo/provenance.json).

Regenerate the park layer before a Blender export with:

```sh
python scripts/condo/prepare_landscape.py scenes/toronto_condo/source/parks.geojson
```

This uses mapped Park/Open Green Space boundaries and the existing land,
road and building geometry. Seed 731 produces scenic trees with park/shore,
road and building clearances. Tree locations and species are not surveyed.
The source crop, query URL, attribution and clearance validation are preserved
in `scenes/toronto_condo/source/`.

The 45 trees within 600 m use deterministic branches and individual leaves;
farther trees retain simpler canopies. These are scenic broadleaf forms, not
surveyed species. They currently have no wind animation. Facade textures are
also original scenic approximations: two glass finishes for taller massing,
limestone/brick finishes for lower massing, and a separate rough roof material.
`textures.py` regenerates these finishes; `facades.py` regenerates only the
facade maps without changing the other textures.

Bevy 0.15.3 does not implement glTF `normalTexture.scale`; Blender therefore
bakes the floor's gentle normal amplitude into `wood_floor_normal_soft.png`.
Original Poly Haven textures are retained unchanged. Clear architectural glass
is excluded from opaque shadow maps, whose alpha threshold otherwise makes
these panes cast solid shadows. Local window and ceiling fills approximate
indirect light; this renderer does not perform diffuse global illumination.

Iterations 151–154 review the living-room ceiling-bounce approximation. The
existing fixed light is now 6500 lumens at runtime `(0, 2.4, -1.5)` metres,
with the same 6 m range, 1 m radius and warm colour. It remains an unshadowed
approximation of reflected room light, not a physical ceiling emitter or an
avatar-following light. Compared with the former 2400-lumen placement, the
matched frame-120 face crop is about 39% brighter in displayed luminance with
no clipped channels; the distant-facade crop is essentially unchanged. The
rectangles and raw measurements are archived in 152. This improves readability
without certifying garment fit, arbitrary avatar positions or physical GI.

The lake shader samples two moving normal layers in world metres (160 m and
93 m tiles, with a rotated second layer). It combines slopes in world X/Z,
keeping wave scale independent of the rectangular lake mesh. Gallery iteration
48 replaces the visible grid of the single stretched layer. The existing
weather wind multiplier drives both layers; this is normal animation without
geometry displacement, wakes or shoreline reflection.

## Vehicle paint variants

The Blender build preserves its single-vehicle export as `vehicle-base.glb`,
then creates six paint scenes in `vehicle.glb`: slate blue, pearl, graphite,
burgundy, silver and forest. Regenerate only the palette with:

```sh
python scripts/condo/vehicle_variants.py scenes/toronto_condo/vehicle-base.glb scenes/toronto_condo/vehicle.glb
python -m unittest discover -s tests -p 'test_condo_vehicle_variants.py' -v
```

The conversion shares the unchanged binary geometry and textures, copying
only scene/node descriptors and painted mesh/material descriptors. It rejects
an already-expanded input. The asset tests check transforms, primitive data,
non-paint materials and binary preservation; regeneration is deterministic.

The manifest's `vehicle_variants` selects `Scene0` through `SceneN-1` from
`vehicle.glb`. It accepts 1–8 and defaults to 1 for existing packages. The
Toronto manifests declare 6. Each lane assigns colours deterministically;
routes, spacing and speeds are unchanged. Readiness waits for the selected
scene dependencies. Gallery entries 53–54 review the broad balcony view and
a 30-degree detail view from the same position. These are paint variants of
one body model, not different vehicle types or live traffic data.

`vehicle_lamps.py` authors separate front and rear lenses in Blender and
regenerates the six paint scenes. The night preset clones their materials for
cool front and red rear emission, and attaches one short, unshadowed headlight
spot per car. Day presets retain non-emissive lenses and spawn no headlight
spots in the original captures. Runtime transitions now preallocate those spots,
hide them at zero intensity in daylight, and fade them in at night. The headlight
uses an authored central intensity of 12,000 candela,
converted to Bevy 0.15's intensity convention (`4 * pi * cd`). Gallery 60
records the lens-only version and 61 the initially weak beam. Entries 62–63
check the corrected beam at condo height and near the road. The latter confirms
forward road pools in both traffic directions. Junction behaviour, braking
signals, occlusion and real-time GPU cost remain unverified.

`street_lights.rs` adds scenic fixtures along the existing traffic routes:
40 m spacing, 8.4 m poles, a shoulder offset and a 1400 m radius around the condo.
These are authored approximations, not Toronto's surveyed lamp locations. Meshes
and materials are shared. Only the night preset enables warm diffuser emission
and 160,000-lumen unshadowed road spots; fixtures remain visible in daylight.
The 16.5 m light range reaches the deck but stops above the lower ground beneath
the elevated highway. It does not provide general occlusion at other obstacles.
The startup test checks that day keeps the geometry but has no active local lights or
emission, that the complete streetlight cone stays below the horizon, and that
elevated fixtures cannot reach the authored lower ground plane.
Visual review, light spill at obstructions and GPU cost remain separate
acceptance requirements.
Gallery 64 records the initial weak lighting, 65 the stronger version with
unwanted lower-ground spill, and 66 the bounded correction. Entry 67 checks
daylight: sampled ceiling, sofa, floor and table regions match entry 51 exactly,
and the avatar/map and interior GLB hashes remain unchanged. These captures
are not full avatar or scene acceptance.

## Night city windows

The night preset applies `city_windows.wgsl` through a cloned extended PBR
material to the four authored facade primitives. Runtime transitions retain the
extended material in daylight with zero window emission. Roofs, roads, interior
and avatar materials are excluded.
The shader follows the facade's existing window cells, leaves frame areas
dark, and uses a stable hash of the window cell and wall origin for occupancy.
Warm/cool colour, brightness variation and a small pane gradient avoid identical
flat panels. Distant emission fades with distance; no per-window lights are
spawned. The capture manifest records the shader hash.

Gallery 55 is the unlit baseline; 56 shows the initial coarse wall seed;
57 refines the curved tower pattern and checks the balcony; 58 checks daylight
isolation and 59 reviews the final living-room view for this pass. This is scenic
static occupancy, not real building activity. Emission is visible through the
condo glass but does not add diffuse room lighting or reflections on the lake.
Vehicle/streetlight refinement and broader weather polish remain outstanding.

Iterations 147–150 review filtering of the occupied-window pattern. The shader
now averages neighbouring panes over a two-pixel footprint and approaches their
expected radiance when cells become subpixel. The first one-pixel trial (147–148)
increased distant contrast and is retained for comparison. In iteration 149,
the fixed distant-city image region has about 7% lower spatial total variation
than baseline 146; the nearby facade remains distinct. This display-image metric
includes intended pattern contrast: it does not certify temporal antialiasing or
GPU performance. Distant windows still look busy and need a moving-camera review.

## Runtime weather transitions

`CC_CONDO_WEATHER` selects the initial preset. On the concrete `CcRenderer`,
`transition_weather(WeatherPreset, seconds)` returns a request ID and
`weather_state()` returns the latest applied observation. This requires a scene
package; it is not yet exposed through the SDK/node command protocol. Durations
must be finite and between 0 and 600 seconds. A newer request replaces an active
transition starting from its current blended atmosphere. Wind phase is integrated
continuously instead of resetting when wind speed changes.

Sky/clouds, water wind, sun/ambient/environment intensity, exposure, distance fog,
rain/snow density, window emission, vehicle/street lamps and baked skylight gain
follow the transition. An explicit `CC_CONDO_EV100` override remains fixed.
Particle pools and night lights are allocated once, then hidden or faded.
The baked irradiance texture itself remains static; this is not dynamic GI.

For reproducible captures, set `CC_CONDO_SECONDS=21.8` and select a schedule:

```sh
export CC_CONDO_WEATHER=clear
export CC_CONDO_SECONDS=21.8
export CC_CONDO_WEATHER_SEQUENCE=scripts/condo/weather-day-rain-night.json
# Run condo_capture with the normal avatar, map, package and new output directory.
python scripts/condo/audit_weather.py artifacts/toronto-condo-captures/NEW_CAPTURE
```

The schedule is a JSON array of `at_seconds`, `weather` and `transition_seconds`
objects. Starts use encoded video time at 30 fps; renderer transitions advance
with renderer time. The capture archives the exact schedule, requests and applied
weather observations. Observations can lead pixels by pipeline latency; the audit
checks transition progress, wind continuity and original pelvis interpolation,
not visual quality or GPU performance. Its PNG plot is labelled as a diagnostic
alongside the screenshots in the gallery.

Gallery 129 preserves the clear-day regression. Iteration 130 records
clear → overcast → rain → night in the living room; 131 starts with rain and
records snow → fog → clear from the balcony. Iteration 132 interrupts an active
rain transition with night, then returns through clear to snow. Each has 654
encoded frames and 653 checked pelvis observations; maximum pelvis deviation
from original packet interpolation is below 0.0000001 m. This does not certify
clothing fit or floor contact. Rust validation reports 270 passed and 2 ignored;
the Python suite retains one existing ownership-test failure and the quality
gate remains non-clean. Full logs are linked from these entries.

Iterations 143–146 address the hard fog horizon. The shared sky function now
uses the camera's linear fog colour/range with an approximate 120 m haze layer;
water reflections inherit the same atmospheric horizon. The sky direction is
relative to the camera. In the fixed fog comparison, the former horizon's
adjacent-row RGB step falls from 9.523 to 0.00074 on a 0–255 scale in the recorded
unoccluded region. Balcony snow/fog/clear and living-room rain/night captures
preserve the transition review. This is an artistic haze approximation, not
volumetric scattering. Exact shader sources, image regions and logs are archived.

Known visual limits: busy distant illuminated windows, unreviewed lighting away
from the current conversation position and remaining garment creases/hem seams. Rain on glass,
snow accumulation, astronomical sun movement, foliage wind, actual weather data
and the ten-minute GPU performance run remain outstanding.

Shader validation for this pass uses actual runtime compilation and captures.
Ripwire does not index these WGSL files, so its scoped checks provide no verdict.
The Python suite still reports six passes and the existing ownership-test failure;
the shader-only changes do not resolve that failure or the broader quality gate.

## Coffee-table stone

The coffee table uses ambientCG's CC0 `Travertine009` colour, roughness and
OpenGL normal maps. `download_assets.py` retrieves the 2K PNG archive with a
pinned SHA-256; source metadata, dimensions and per-file checksums live in
`scenes/toronto_condo/models/Travertine009/provenance.json`. `build.py` applies
the finish automatically. To update the existing normal and lightmapped
Blender sources and exports without rebuilding the room:

```sh
blender --background --python scripts/condo/stone_material.py
```

The finish uses a 1.2 m UV0 repeat, softened normal XY amplitude of 0.10 and
honed roughness `0.38 + 0.65 * source`. Only the coffee table receives it.
Gallery 68 reviews the living-room view and 69 the rounded edge close up.
Geometry, normals, transforms, UV1 and existing embedded image bytes were
verified against the pre-change assets. Unrelated UV0 export rounding stayed
below 0.000001; full GLB byte-for-byte repeatability is not claimed. The existing
macro sky-irradiance maps remain a diagnostic approximation; the scene has
not received a new full light bake or final visual acceptance.

## Tabletop details

`tabletop_props.py` replaces the cup and book proxies with an original hollow
ceramic mug, loop handle, coffee surface and cloth-bound `FIELD NOTES` book.
Their bases use the table's actual upper surface. These are decorative meshes;
no new avatar interaction or motion is implied. Update both saved packages and
refresh only the coffee-table irradiance map with:

```sh
blender --background --python-exit-code 1 --python scripts/condo/tabletop_props.py
blender --background --python-exit-code 1 --python scripts/condo/bake_floor.py -- --coffee
python scripts/condo/encode_lightmap.py --only coffee-sky-linear.png \
  --settings scenes/toronto_condo/sky-bake-coffee-settings.json \
  --oidn-lib /path/to/libOpenImageDenoise.so
```

The partial bake reuses the existing UV1 atlas and keeps other maps and gains.
Its settings and denoising report are saved under `floor-bake.json`'s
`surface_updates`. Gallery 70 preserves the first, edge-on handle; 71 shows
its opening clearly and 72 checks the main composition.

## Woven upholstery

The sofa and reading-chair cushions use Poly Haven's CC0
[Terlenka](https://polyhaven.com/a/terlenka), a beige woven polyester.
`download_assets.py` keeps source checksums and attribution. `upholstery.py`
uses the unmodified colour and roughness maps, with normal XY amplitude 0.2
baked for Bevy. UV0 is unwrapped only on cushion faces and scaled to the
source's approximately 0.266 m repeat. Wood UVs, geometry and baked UV1 stay
unchanged. No new irradiance bake is needed for this surface-only change.

```sh
blender --background --python-exit-code 1 --python scripts/condo/upholstery.py
```

`build.py` applies the finish during a full rebuild as well.
Gallery 73 reviews the close view; 74 checks the warmer beige in the full
living room. The fine weave remains subtle at the wide camera distance.

The main composition is still a prototype. Its current recorded native clip
was prompted to turn and chat, but all 400 source packets have root yaw between
approximately -2.0 and 42.6 degrees, and the avatar remains back-facing in the
wide review. Iteration 74 includes `avatar-review.json` with source hashes and
measurements. Her lifted/tilted shoes also require a source-versus-CC5 fidelity
investigation; no corrective pose or root placement was applied. The canonical
avatar contains body, hair, accessories and heels, but no clothing mesh.

GPU trials 112 and 113 preserve two separately declared seed-731 Core40 attempts
and their complete 20-second recordings. The compound greeting prompt does not
turn (root yaw approximately -2.8 to 22.0 degrees). The single-action prompt,
"A person turns around", reaches approximately 174 degrees at two seconds but
continues turning afterward. Neither is accepted as a relaxed conversational
performance. A future semantic goal handoff must continue service-owned history
after observed completion; it must not rotate, hold or splice a pose to obtain
the desired composition. The fresh greeting's read-only shoe audit still measures
roughly 4.2–5.1 cm of floor penetration. These short GPU captures establish
neither contact acceptance nor sustained performance.

`generate_turn_conversation.py` is an offline semantic-handoff experiment. It
observes a generated turn within a 30-degree opposite-facing cone for 200 ms,
then requests conversation through `MotionService`. Those values are observation
criteria only; the model receives semantic text, not angle or pose constraints.
The service selects the retained native-history boundary. Every raw response,
including superseded lookahead, is retained alongside the selected replay packets.
The turn has an eight-second attempt budget; failure does not trigger pose repair.

Run it with the same managed SDK Python, cached Core40 model and encoder
environment used by `generate_motion.py`:

```sh
python scripts/condo/generate_turn_conversation.py \
  --output artifacts/toronto-condo-captures/my-conversation --seed 731 --seconds 20
```

Replay can now cross recorded revisions when the adjacent `responses.ndjson`
proves that every packet is unchanged and each boundary matches service-owned
history. It rejects mixed sessions, backward timestamps/revisions, altered packets,
missing source responses and invalid boundaries. The handoff waits for the old
prefix to reach the renderer queue before sending `NativeSceneHandoff`, then
waits for `acknowledged_native_handoff` before releasing new-revision packets.
That read-only observation is set after the timeline applies the command; a
buffered prefix alone does not prove that the separate command queue was drained.
Single-revision recordings remain supported. This diagnostic path does not change
the motion service, policy, pose writer or retargeting. The renderer adds only
handoff acknowledgement metadata; the diagnostic replay uses it for transport.

Gallery 114 preserves the initial race: a new-revision packet could clear the
old queue before its handoff command was applied. Gallery 115 replays the same
436 source packets with acknowledgement. Across its captured states, maximum
observed pelvis error against interpolation of the original source roots falls
from 18.1 mm to about 0.0001 mm, with no stale packets. The conversation stays
approximately front-facing after the turn. All reported underruns occur after
the final 21.8-second source timestamp. This establishes this replay's transport
fidelity; shoe penetration, clothing and general avatar acceptance remain open.

## Texture filtering at first render

`texture_filtering::install` schedules texture preparation in `Last`, after
Bevy's `AssetEvents` set and before render extraction. Processing `Added`
events in `Update` was one frame late: a material could retain the original
unfiltered GPU image view even after the CPU image acquired mipmaps. An
unchanged-scene repeat in gallery 86 reproduced the facade sharpness variation.

The regression test loads a real PNG through the scene asset source and checks
the first main frame exposing that image. It failed with one mip level before
the scheduling change and passes with four levels and anisotropic filtering:

```sh
cargo test --offline --lib texture_filtering
```

Gallery 87–88 are independent startup captures with identical configuration
and asset hashes. Two opaque tower regions and a chair-back region matched
pixel-for-pixel in all three sampled frames. The comparison excludes animated
sky, water and traffic; its rectangles and results are archived with the captures.
Gallery 89 checks the full room and avatar; 90 checks the night city material.

This controls preparation of newly added images. It does not claim that hot
reloading an existing image rebuilds every dependent material binding.

## Balcony lounge furniture

`balcony_furniture.py` replaces the two solid chair proxies and cylindrical
table with original slatted timber frames, woven cushions and a three-legged
stone side table. The chairs face the lake, with the central two-metre route
left clear. Gallery 82 preserves the proxies and 83 records the replacement.
The timber samples interior board patches from the existing CC0 Poly Haven
`wood_floor` maps at their 1.7 m source scale, with grain running along each
beam. Its normal map reuses the already softened floor normal; no source map
is overwritten. Geometry and upholstery are preserved.

Three conservative static collision boxes are derived from the furniture
bounds. These are walking obstacles, not seating/support declarations. The
balcony overhang keeps its physical ceiling collider but is no longer an XZ
navigation obstacle; that incorrect flag previously blocked the whole balcony.
The read-only checker verifies the current 0.35 m planning clearance around
approach points, a continuous central-route clearance bound and ceiling height:

```sh
python scripts/condo/check_balcony_clearance.py \
  scenes/toronto_condo/scene-lightmap.json /tmp/balcony-clearance.json
```

Current geometry gives 2.92 m headroom, 0.40 m chair-approach clearance and
about 0.371 m between the table approach and the guard. These static checks
do not establish avatar traversal, seating, or physical contact acceptance.

The asset edit is included in `build.py`; to update existing packages:

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/balcony_furniture.py
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bake_floor.py -- --balcony
python scripts/condo/encode_lightmap.py --only balcony-sky-linear.png \
  --settings scenes/toronto_condo/sky-bake-balcony-settings.json \
  --oidn-lib /path/to/libOpenImageDenoise.so
```

`--balcony` adds or refreshes the slab's sky-irradiance atlas at 512 × 512 and
128 Cycles samples, preserving all existing room lightmaps and their mappings.
`--room` now includes this slab when rebuilding the complete bake. This static
pass includes furniture occlusion; it does not bake the avatar or direct sun.
The normal package remains unbaked. Furniture/material update scripts share
`scene_export.update_saved_environments` for exporting and saving both packages.

## Dining furniture

`dining_furniture.py` replaces the dining proxies with four original curved-back
chairs, tapered oak legs and a rounded travertine table on a fluted pedestal.
Each chair exports as one mesh with two material primitives. The script reuses
the existing CC0 wood, woven fabric and stone maps and is included in `build.py`.
Update both saved packages and refresh the floor beneath the new furniture with:

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/dining_furniture.py
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bake_floor.py -- --floor
python scripts/condo/encode_lightmap.py --only floor-sky-linear.png \
  --settings scenes/toronto_condo/sky-bake-floor-settings.json \
  --oidn-lib /path/to/libOpenImageDenoise.so
```

`--floor` refreshes the existing floor atlas while preserving its UV1 and all
other lightmap bindings. It uses 128 Cycles samples at 512 × 512. Dining furniture
currently has no registered collision or seating affordances; visual captures
do not establish avatar traversal or seating acceptance. Gallery iteration 94
records the previous furniture before this replacement.

## Kitchen geometry

`kitchen_furniture.py` replaces the five kitchen proxies with original oak
cabinet fronts, metal pulls, an actual countertop opening and recessed sink,
an arched faucet, induction markings, oven, hood and refrigerator. The island
keeps its centre and width, shortens from 2.65 m to 2.30 m to open the north
approach, and gains stone waterfall ends. Existing CC0 light-oak
and travertine textures are reused. `build.py` includes this authoring step.

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/kitchen_furniture.py
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bake_floor.py -- --refresh-room
python scripts/condo/encode_lightmap.py \
  --settings scenes/toronto_condo/sky-bake-room-settings.json \
  --oidn-lib /path/to/libOpenImageDenoise.so
```

The script replaces the two original kitchen collision boxes with seven
geometry-derived groups. Base units, refrigerator and island block XZ
navigation; the upper cabinets, backsplash, hood and faucet have separate
physical boxes. These conservative bounds do not model the sink cavity.
Cooking and avatar-operated fixture use remain unimplemented. Running water
is available through the scene controls described below. Iteration 97 is the visual baseline.
`--refresh-room` rebakes all six existing atlases from the lightmapped Blender
file, preserving already-applied geometry and UV layers. `--room` remains the
initial bake from the normal scene and can produce different bevel UVs.

The shortened island leaves a 0.78 m north passage. Check the static corridor
from the open living area to the kitchen aisle with:

```sh
python scripts/condo/check_kitchen_clearance.py \
  scenes/toronto_condo/scene-lightmap.json /tmp/kitchen-clearance.json
```

This verifies the current 0.35 m planning reserve against the package's XZ
obstacles, with a conservative bound between probes. It does not submit a
route to the avatar or prove traversal and reachability by generated motion.

## Bedroom geometry

`bedroom_furniture.py` replaces the bed blocks with oak rails and recessed feet,
an upholstered headboard, puffed linen pillows and a sage duvet with shallow
folds and draped edges. The bedside unit has drawer fronts, a stone top and a
ceramic lamp with a hollow linen shade. Its point light and transmitting shade are authored in the scene package.
These are static furnishings; bedding does not simulate contact with the avatar.

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bedroom_furniture.py
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bake_floor.py -- --refresh-room
python scripts/condo/encode_lightmap.py \
  --settings scenes/toronto_condo/sky-bake-room-settings.json \
  --oidn-lib /path/to/libOpenImageDenoise.so
python scripts/condo/check_bedroom_clearance.py \
  scenes/toronto_condo/scene-lightmap.json /tmp/bedroom-clearance.json
```

Room bakes include the bedroom rug, mattress and duvet when the detailed bed is
present. `--bedroom` updates only those three atlases and writes
`sky-bake-bedroom-settings.json`. Always rebake after replacing bedroom geometry:
the new mattress and duvet initially have no UV1 or matching lightmap bindings.
The existing room atlases retain their UVs during `--refresh-room`.
The duvet uses continuous top and underside UV1 rectangles with a separate
padded rim strip. This avoids the patch boundaries seen with its initial smart
projection. `--duvet` rebakes only that surface; encode it with
`--only bedroom-duvet-sky-linear.png --settings scenes/toronto_condo/sky-bake-duvet-settings.json`.

Four geometry-derived collision groups replace the original bed box. Static
west and east corridors pass the 0.35 m planning reserve, with conservative
clearance bounds of approximately 0.413 m and 0.363 m. These checks do not prove
avatar traversal, sitting, lying, cloth interaction or physical support.
Iteration 102 records the previous bedroom as the visual baseline.

## Authored practical lights

The optional `point_lights` list adds static indoor lights to a scene package.
Each entry specifies an ID, metre-based position/range/radius, sRGB colour,
lumens and whether it casts shadows. An optional diffuser names one mesh in
the environment GLB and gives its linear emissive strength. Empty or omitted
lists preserve earlier packages. Duplicate IDs/diffusers, non-finite values,
invalid colours and radii at or beyond the range are rejected.

The renderer clones the diffuser material for that mesh alone, applies camera
exposure to its emission, and disables its shadow casting to approximate linen
transmission. Walls and other furniture continue to cast shadows. Diffuser
matching requires both the `scene` asset source and this package's geometry
file; avatar meshes and vehicle prefabs cannot be selected by a matching name.

The bedroom light is derived from the authored shade bounds. To update only
lamp placement/configuration in both manifests, leaving meshes and bakes intact:

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/export_practical_lights.py
```

The exporter reads each Blender source separately and resolves the actual mesh
names for bedroom, workspace and bathroom diffusers. This preserves bindings
after incremental rebuilds add Blender mesh suffixes. All three room-update
scripts refresh this metadata, and `build.py` includes it in fresh builds. These lamps
are currently on in every weather preset; time-based switching is not implemented.
An emissive shade and a point light approximate a real fabric luminaire, rather
than simulating light transport through the shade material.

The current bedside fixture uses 600 lumens, a 3.5 m range and 0.08 m radius.
Gallery 108/109 records night/day GPU renders. Iteration 110 tests PCSS;
its negligible visible improvement did not justify retaining the experimental
feature. The final implementation uses ordinary shadow mapping. Iteration 111
checks the main living-room view, but does not establish avatar or performance
acceptance.

## Foot calibration audit

Gallery 75 captures the untouched bind-pose shoes; 76 uses the same camera with
unchanged native playback. Gallery 77 contains the read-only geometry audit:

```sh
MPLCONFIGDIR=/tmp/condo-matplotlib python scripts/condo/audit_avatar_feet.py \
  path/to/original/packets.ndjson /tmp/new-foot-audit \
  --capture path/to/native/capture.json
```

The script reuses the adjacent SDK's `heel_contact_foundation.Rig` CPU mirror
of calibrated FK and normalized four-influence skinning. It hashes the source
avatar/map, packets, audit code and calibration implementation; computes shoe
geometry for every original packet; and optionally compares final observed
runtime ankles at an exact source timestamp. The neutral probe exists only in
the offline audit and is never sent to playback.

For the current canonical avatar, the original ankle-to-toe vectors point
approximately 50.8 degrees downward. Neutral calibration aligns them with the
canonical horizontal +Z axis, raising the forefoot relative to the heel by
approximately 10.5 cm. At the recorded 2500 ms packet, the left outsole heel
centroid is -12.5 mm and the forefoot centroid +92.1 mm relative to y=0. The
native pelvis is also 45.4 mm below the asset's bind pelvis. These are separate
reference-frame and body-proportion findings, not permission to move the root.
The CPU/runtime ankle-coordinate difference is below 0.0002 mm at that checked
frame; this is not a full GPU vertex readback or a physical contact test.

The existing calibrated tests pass while this shoe defect remains visible:
they establish orientation transfer and agreement with the current reference,
not anatomically appropriate shoe calibration. A fix must keep the renderer,
SDK geometry calibration and bound profile/fixtures consistent, preserve all
source motion deltas, and verify the original avatar/map hashes. No production
retargeting or motion was changed by this audit. Some skin/shoe intersection is
already visible in bind view 75 and needs its own asset-fit review.

Gallery 78 applies the fixed-reference correction in both `pose.rs` and the
SDK's CPU mirror: neutral ankles retain their authored calf-relative bind
rotation, and neutral toes inherit that reference. All animated ankle/toe
rotations still pass through the existing full-world-delta transfer. No root
translation, floor snapping or source packet changes were introduced. Gallery
79 repeats the complete geometry audit with the corrected reference.

The content-bound SDK profile was regenerated with only neutral matrices
7, 8, 10 and 11 changed; all other geometry arrays and seat patches remain
identical. Its new identity is
`074ca43d30bb5cdfc3052d8fcd949997c6816ab52bb5c984f18620ae6a11f991`.
The canonical GLB/map remain unchanged. Rebuild the renderer and reload any
motion service holding the previous profile before using the new calibration;
the condo captures use unchanged recorded packets, not a newly loaded model.

`refresh_avatar_calibration.py PREVIOUS_PROFILE NEW_DIRECTORY` stages this
one-time profile migration and the cross-language fixture without overwriting
its input. It refuses a profile whose changed-bone set differs. The matching
verification uses every original packet and produces the existing Rust
geometry test's input:

```sh
python scripts/condo/check_avatar_calibration.py \
  path/to/profile.json path/to/original/packets.ndjson /tmp/new-parity
CC_GEOMETRY_FIXTURE=/tmp/new-parity/renderer-parity-fixture.json \
  cargo test --offline --lib exported_native_geometry_matches_python -- --ignored
```

This removes the unnatural upward shoe tilt, but **does not solve floor
contact**. The unchanged sample now puts both outsole regions roughly 3–4 cm
below y=0. The source/target proportion mismatch remains an explicit failure,
and the avatar view is not accepted as polished. The new regression first
failed on the old calibration; full Rust tests and profile/geometry parity now
pass without changing source root coordinates or weakening ownership gates.

## Workspace and entry

`workspace_furniture.py` replaces the desk/monitor/chair and entry-storage proxies
in both editable Blender scenes and exported packages. The desk now follows the
bedroom partition, leaving the entry approach open. It has a beveled oak top,
bronze frame, drawer, cable tray, inactive monitor, keyboard, mouse, notebook and
task lamp, with floating oak shelves, cloth-bound books and a ceramic vase above.
The chair reuses the original curved-back dining-chair construction.
The entry adds an oak cabinet with separate doors/handles, cushioned shoe bench,
original geometric artwork and a static closed front door with upper infill.

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/workspace_furniture.py
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bake_floor.py -- --refresh-room
python scripts/condo/encode_lightmap.py --oidn-lib /path/to/libOpenImageDenoise.so \
  --settings scenes/toronto_condo/sky-bake-room-settings.json
python scripts/condo/check_workspace_clearance.py \
  scenes/toronto_condo/scene-lightmap.json /tmp/workspace-clearance.json
```

Use `CC_CONDO_VIEW=workspace` for the saved review camera. The source builder also
includes the furnishings, light and camera for fresh scene builds. Static collider
bounds come from the authored furniture groups. Entry/cabinet/desk-side free-space
checks use a 0.35 m radius and begin inside the closed door. They do not drive the
avatar or certify traversal, sitting, desk use or opening the door. Furniture and
monitor remain static, with no interactive desktop. Gallery 155 preserves the
placeholder baseline; its compressed asset backups are in
`artifacts/toronto-condo-captures/workspace155-backup/`.

## Bathroom

`bathroom_furniture.py` replaces the vanity, shower and toilet proxies in both
Blender sources. It authors a floating oak vanity, recessed ceramic basin,
faucet, framed mirror, two sconces, ceiling light, stone shower cladding,
fixed glass screen with an open east-side entrance, rain head, closed toilet,
towel and soap dispenser. The room bake includes a separate bathroom-floor atlas.

```sh
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bathroom_furniture.py
blender --background --threads 8 --python-exit-code 1 \
  --python scripts/condo/bake_floor.py -- --refresh-room
python scripts/condo/encode_lightmap.py --oidn-lib /path/to/libOpenImageDenoise.so \
  --settings scenes/toronto_condo/sky-bake-room-settings.json
python scripts/condo/check_bathroom_clearance.py \
  scenes/toronto_condo/scene-lightmap.json /tmp/bathroom-clearance.json
```

Use `CC_CONDO_VIEW=bathroom` for the saved review camera. Gallery 160 preserves
the placeholder baseline and 161 the first authored review, including the
initial diffuser-binding failure. The static route checks include all fixtures
intersecting the checked height envelope, with a 0.35 m radius; the current
minimum bound is 0.385 m. An injected shower obstruction must be rejected.
These checks do not drive or certify avatar traversal or fixture use.

Reviews 162–163 preserve the night-lighting repair: actual mesh-name bindings,
then full-length opal sconce emission. Bathroom diffuser strengths are 1800 for
the sconces and 12000 for the ceiling; point-light powers remain 500/500/2200 lm.
The ceiling fixture lies outside the current review camera, so its visible
appearance has not been assessed by these captures.

The mirror currently reflects the external environment map, not the bathroom
or avatar. Practical lights still have hard shadow edges. Running water is
controlled independently as described below, with read-only avatar surface collisions.

## Functional shower and faucets

All condo packages include three independently controllable outlets:
`kitchen_faucet`, `bathroom_faucet`, and `shower`. They start off.
On `CcRenderer`, `set_water_flow("shower", 1.0)?` opens the shower fully;
`set_water_flow("shower", 0.0)?` closes it. Values between zero and one
adjust flow. Invalid values and unknown fixture IDs return errors.
`water_state()` reports each fixture's applied revision, valve opening,
litres/minute, cumulative estimated litres and visible/runoff particle counts.
It also reports body/drain impacts, dropped emissions, unresolved collisions,
collision geometry counts/revision, and any observation error.
CPU collision-update and simulation timings are included in milliseconds.
The returned revision is per fixture; observe it before treating a request as applied.
Rapid requests retain the latest setting for each outlet independently.

Water parcels integrate gravity and drag with substeps no longer than 1/120 s.
Swept point/triangle queries test the avatar's previous and current skinned
render meshes, including morph weights and optional tissue deformation.
The collision solver reuses the tissue continuous-collision implementation;
it accounts for both falling droplets and moving body surfaces. Impacts absorb
normal momentum and retain tangential motion in a gravity-driven surface film.
Runoff follows triangle material coordinates, crosses connected faces, and detaches
into falling drops. The adapter only reads avatar geometry; it never edits
bones, motion packets, or mesh vertices.
The GPU renders the droplets; independent CPU collision queries use Bevy's task
pool. The surface tree is refitted between periodic rebuilds, and idle water
does not continuously reskin the avatar. Morph/topology caches warm during scene
loading. Zero-area triangles are counted and excluded from collision surfaces.
Water queries use a 10-micrometre contact tolerance and bounded interval subdivision
for grazing contacts; the tissue solver retains its existing tolerance.

Authored receiving planes represent the basin bottoms and shower floor. Drops
make a small impact splash there and are absorbed on subsequent contact.
Closing a valve stops emission and allows remaining drops to drain or expire.
A fixed pool of 2,048 shower parcels and 512 per tap (3,072 total) bounds water
entities and materials. Parcel lifetime is three seconds; surface attachment
lasts at most 1.5 seconds. The simulation uses the scene clock and also runs
without avatar playback. Collision-observation errors pause water advancement
and are exposed in state; individual unresolved sweeps retire that parcel and
increment an explicit diagnostic counter.

This is a bounded parcel approximation, with estimated flow rates of 6 L/min
per tap and 9 L/min for the shower at full opening. Parcels represent bundles
of liquid, not volume-conserving fluid cells. There is no pressure/viscosity
solve, two-way force on the avatar, basin filling, overflow, heat, steam,
sound or wet clothing. Hair cards are triangle surfaces without alpha-cutout
collision filtering. Receiving planes do not model basin walls or puddles.
The existing
lever meshes remain static. Direct scene controls do not generate an avatar
reaching for a tap; ARDY retains ownership of any future body interaction.
The concrete renderer API is not yet exposed through the SDK/node protocol.

`scripts/condo/plumbing.py` defines the outlet positions and receiving heights
in package coordinates and refreshes existing manifests without rebuilding the
large GLBs. `build.py` includes the same definitions in new exports. To record
an off/on/reduced-flow/off cycle through the public renderer API:

```sh
cargo test --offline --lib water
cargo build --offline --example condo_capture
AVATAR_BEVY_GPU_INDEX=1 CC_CONDO_VIEW=bathroom CC_CONDO_SECONDS=8 \
  CC_CONDO_WATER_SEQUENCE=scripts/condo/water-cycle.json \
  target/debug/examples/condo_capture \
  ../remotemedia-sdk-office-main/avatars/processed/beautiful_assistant.smpl_aligned.glb \
  ../remotemedia-sdk-office-main/avatars/beautiful_assistant.arkit_map.resolved.json \
  scenes/toronto_condo/scene-lightmap.json /tmp/condo-water-NEW
```

The capture preserves the sequence, `water-requests.ndjson`,
`water-states.ndjson`, and final water state in `capture.json`. State observations
are sampled by the capture consumer, not synchronized GPU frame metadata.
Use a fresh output directory and publish each render to the progress gallery.

For stationary avatar interaction, `generate_water_motion.py OUTPUT` records a
fixed-seed, semantic-only standing session. Run it in the SDK's configured ARDY
Python environment. It saves its plan
before sampling, requests, complete responses, unchanged packets and native journals.
Use `CC_CONDO_NATIVE_PACKETS=OUTPUT/standing/packets.ndjson` with
`AVATAR_START=-5.5,-4.3,90`. Pass `water-body-cycle.json` as the water sequence.
The renderer must receive the same `AVATAR_START` recorded in service provenance.
This uses the existing owner-approved session-start implementation from commit
`2c1db23`: initial scene-root translation only, with world poses subsequently
consumed unchanged. It does not disable or relax navigation admission.

**Shower entry uses normal navigation.** Connect `condo_capture` to the existing
scene-enabled ARDY daemon with `ARDY_SOCKET` and its startup signing-key path in
`CC_CONDO_SCENE_KEY`, then set `CC_CONDO_NAVIGATE=shower_screen`. Do not set
`CC_CONDO_NATIVE_PACKETS` in this mode. The capture submits a normal v3 goal:

```json
{"operation":"navigate","target_id":"shower_screen","pace":"normal"}
```

The shower screen's authored approach point lies under the water outlet. It stays
a navigation obstacle; the planner must route around the glass. Older diagnostic
packages without the detailed bathroom do not acquire this target.
`navigation_recording.rs` uses the application's `CcRenderNode`, normal native
bootstrap, registered capabilities, signed renderer-owned scene snapshots,
navigation lifecycle and revision handoffs. ARDY supplies walking through the
installed planned-navigation backend. There is no shower-specific step prompt or
pose path. The capture preserves goals, signed requests, complete responses and
node events (including arrival status). Its frame tap preserves raw frame metadata.

For the entry water check, use `AVATAR_START=-4.4,-4.3,-90` on both the daemon and
renderer, `CC_CONDO_VIEW=bathroom`, `CC_CONDO_SECONDS=30` and
`CC_CONDO_WATER_SEQUENCE=scripts/condo/water-navigation-cycle.json` (on at zero,
off at 25 seconds). Each fresh output directory creates a fresh motion session.

The [normal-navigation recording](../../artifacts/toronto-condo/20261019T042823Z-normal-navigation-into-running-shower-completed/view.mp4)
reports navigation completion at native 5,262 ms, then remains under the outlet
through the 30-second capture without a motion rejection. It reaches 543 runoff
parcels, has zero unresolved queries/dropped emissions, and finishes with no
visible/runoff parcels after shutoff. The 900-frame 1080p capture takes 31.26
seconds; this is not a GPU timing or general navigation-reliability benchmark.
The earlier normal-command attempt explicitly rejected `shower_floor` because
non-obstacle floor solids are not exported as office targets; that attempt is
preserved. The final command uses the existing screen object without changing
navigation admission or lifecycle code. Navigation/capture tests pass (22 + 6);
the repository's existing Python text-slice failure remains. The scoped static
quality gate still reports trait-method reachability findings and is not clean.
Plans, source hashes, test logs and daemon journals are retained under
`artifacts/toronto-condo-captures/water-navigation-v1/`.

For a timed washing prompt after arrival, add these settings to the same live
navigation capture:

```sh
CC_CONDO_AFTER_NAV_PROMPT='A person stands in place under a running shower, washing their body with their hands and rinsing off.'
CC_CONDO_PROMPT_SECONDS=30
CC_CONDO_SECONDS=50
CC_CONDO_WATER_SEQUENCE=scripts/condo/water-washing-cycle.json
```

The capture waits for the matching Navigate completion before submitting the
semantic prompt. Its timer starts on the first displayed frame of that revision;
bootstrap, navigation, generation ahead and playback stalls do not consume the
requested duration. After 30 seconds of displayed native time it submits a relaxed
standing prompt through the ordinary replacement handoff. Its native boundary is
normally 0.2–0.6 seconds ahead, with additional observation latency.
`prompt-duration.json` records start, stop request and
first displayed replacement timestamps, or an explicit failure. Motion remains
model-generated; a timed prompt does not guarantee washing quality or collision
acceptance. This sequence keeps water on until capture second 46 and allows four
seconds for drainage. Use the report to verify completion before accepting a run.

The [timed washing recording](../../artifacts/toronto-condo/20261019T044851Z-normal-navigation-then-thirty-second-shower-washing-prompt/view.mp4)
completed normal navigation at native 5,290 ms. Washing started at 5,972 ms;
the standing replacement was requested at 35,979 ms and first observed at
36,704 ms: **30.732 seconds of native washing motion**. The video shows repeated
bending and hands moving along the legs and hips. All 16 washing responses have
zero future constraints and no policy exceptions; no motion rejection occurred.
Water stayed on through every washing frame, reaching 583 runoff parcels. One
unresolved water query occurred later, at encoded frame 1,298, after washing had
ended; no emissions were dropped and the system drained completely. The capture
produced 50 seconds of video in 76.56 seconds of wall time, so this is not a
real-time performance result. The washing segment spans about 28.77 encoded
seconds because the capture uses fixed 30 fps while native playback uses its own
clock. Eight capture tests pass; the existing Python text-slice failure remains.

## Shower caddy and dispensing bottles

`scene-shower-caddy.json` extends the lightmapped condo with a bronze wall caddy
and three labeled, independent rigid bodies: `shampoo`, `conditioner`, and
`body_wash`. Regenerate its assets with:

```sh
blender --background --python-exit-code 1 --python scripts/condo/shower_caddy.py -- /tmp/new-caddy-preview
```

Package `props` carry verified geometry, compound collision shapes and optional
dynamic mass. Loose bottles settle under physics. The caddy experiment now uses
the existing declared `interact` attachment and placement behavior, explicitly
requested after the failed `grasp` experiment. Held bottles follow the attached
hand and set-down uses the declared support zone; this is not proof of a
contact-admitted rigid grip or purely dynamic placement. Each open bottle
has 250 ml available and dispenses from its transformed mouth when tilted past
horizontal. It stops upright or empty. Liquid uses the existing gravity parcels
and moving-skin collision/runoff; shampoo viscosity, squeeze pressure, liquid
free surfaces, and decreasing bottle mass are not simulated.

For the live experiment, start the ordinary scene daemon with
the `beautiful_assistant` calibration and a matching
scene key. Use the existing navigation capture with these settings and select
`scenes/toronto_condo/scene-shower-caddy.json` as the scene argument:

```sh
unset AVATAR_START
unset ARDY_SCENE_GUIDANCE
CC_CONDO_VIEW=bathroom
CC_CONDO_NAVIGATE=shower_screen
CC_CONDO_CADDY_TASK=shampoo
CC_CONDO_SECONDS=90
CC_CONDO_WATER_SEQUENCE=scripts/condo/water-caddy-cycle.json
```

The caddy task and `CC_CONDO_AFTER_NAV_PROMPT` are mutually exclusive. Normal
navigation completes first, followed by `interact(target, pick_up)`.
The pickup prompt explicitly describes standing upright and reaching up to a high
shelf near head height. Pickup, pouring and return use generic bottle wording,
without shampoo, conditioner or body-wash names in the model prompt. This is semantic posture intent; the measured
contact zone remains the actual bottle on the existing shelf.
The initial attach reach uses calibrated avatar wrist measurements and at most
two native measured-miss retries, as subsequently approved by the owner. Observed
attachment ends pickup targeting.
Pickup ends at observed attachment; the additional two-second hold phase has
been removed per the owner's playback correction.
After matching pickup completion, the task waits for the actual bottle attachment
before submitting `interact(target, pour_into_hand)`. It does not wait for shelf
departure before starting the next motion: the completed pickup can stop while
the held bottle is still near its original position. Observed departure remains
necessary for successful pouring.
The owner explicitly superseded the motion-ownership restriction for this work.
Pouring now guides the bottle outlet over the upward receiving palm using the
actual acquired grip transform and captured hand geometry. Both hand positions
and orientations, and the native encoder's required root channels, are recorded
in the audit. Up to two same-seed measured-miss retries are retained per block. The experiment observes the declared right-hand coupling rather
than the separate physical-grip registry. It requests `interact(target, set_down)`
after observing a lift, at least 0.1 ml dispensed and three actual skin
impacts within 4 cm of the upward left palm. Counters start at the first displayed
pour revision, excluding earlier incidental drips. After 10 seconds without those
outcomes, it still attempts set-down but retains an explicit pour failure.
The palm distance is a diagnostic approximation, not exact hand-mesh identity.
The bottle must remain in that shelf region during a one-second observation
window after observed release. Only then does `interact(shower_caddy, face_outwards)`
use the ordinary approach-navigation planner to face into the shower, away from
the caddy wall. Its declared stand phase requires observed position and facing;
the washing prompt starts only after that turn completes. The requested order is
**grab → pour into hand → put back → face outwards → shower animation**.
The final shower prompt runs for 30 seconds of displayed native time, then waits
for its relaxed continuation to be displayed before completing the task.
The return phase first faces the caddy through normal approach navigation and
then plays an explicit right-hand placement prompt. The owner approved bounded
calibrated hand guidance for this put-back phase as well: targeting ends after
observed release. Washing remains unconstrained semantic animation, with an
upright, forward-looking prompt to avoid excessive head turns.

`caddy-task.json` identifies the object handling
as `declared_interact_coupling` and records the evidence or failure; a
pour timeout never counts as successful manipulation.
The recording finalizes early on a terminal caddy-task result.

The [complete sequence recording](../../artifacts/toronto-condo/20261019T072033Z-complete-shower-sequence-grab-pour-put-back-face-outwards-wa/view.mp4)
passes the full sequence at fixed seed 731. It observes 7.79 ml dispensed with
49 left-palm impacts, shelf release and settling, an outward turn, and 30.599
seconds of displayed washing animation. The bottle remains upright and released
on the shelf throughout washing. All 16 washing blocks have zero future
conditioning; the observed-release placement block also has zero future
conditioning. The turn uses navigation channels without hand targets. This is
one successful seed, not a reliability matrix. The 80 targeted Python tests and
nine capture-example Rust tests pass. Source snapshots, all reach attempts,
audits and validation are retained in
`artifacts/toronto-condo-captures/shower-caddy-outwards-v2/`; the failed unguided
return is retained separately under `shower-caddy-outwards-v1/`.

The subsequent generic-bottle wording run also passes at seed 731, with 571
total left-palm impacts and 30.6 displayed seconds of washing. Its
[clip](../../artifacts/toronto-condo-captures/shower-generic-bottle-v2/pour-and-return.mp4)
and evidence are in `artifacts/toronto-condo-captures/shower-generic-bottle-v2/`.
The first generic run exposed the pickup handoff stall described above and is
retained as `shower-generic-bottle-v1/`. Hand guidance was still needed; this
combined prompt and handoff change does not isolate the effect of the product
name. Ten capture tests pass, including attached-but-still-near-shelf handoff.

The guided-pour implementation now passes with the upright pickup. The [latest
recording](../../artifacts/toronto-condo/20261019T125839Z-upright-pickup-guided-palm-pour-shelf-return-and-30-second-s/view.mp4)
(seed 731) visibly tips the bottle over the open left palm, observes 448 total
left-palm impacts, returns the bottle upright and released, turns outward, and
completes 30.6 displayed seconds of washing. All 16 washing blocks have zero
future conditioning. The task first observes successful pouring at 22,006 ms;
already committed pouring finishes displaying at 24,438 ms. This queued tail
is retained unchanged. There is also a [12.5-second pour/return excerpt](../../artifacts/toronto-condo-captures/shower-guided-pour-v4/pour-and-return.mp4).

Evidence, source snapshots, channel audits and checks are under
`artifacts/toronto-condo-captures/shower-guided-pour-v4/`. The three preceding
attempts remain in `shower-guided-pour-v{1,2,3}/`: attachment handoff, missing
wrist-audit metadata, and a late head-twist failure during washing, respectively.
The latter motivated the explicit upright, forward-looking washing prompt.
91 targeted SDK tests and 16 renderer context/interaction/capture tests pass.
The broader renderer Python suite still has its existing ownership source-text
matcher failure; structural checks retain findings in the larger uncommitted
work. This establishes one fixed-seed end-to-end pass, not a reliability matrix.

Before guided dispensing, the upright-reach revision improved pickup posture
but did **not** pass that full sequence. Two seed-731 captures reduce maximum displayed pickup
torso tilt from 56.6 degrees to 17.9 degrees while still attaching the shampoo.
Pickup takes about 5.7 seconds from the goal request, versus 5.2 seconds before;
this is a posture improvement, not a latency fix. The more upright continuation
raises the bottle toward the face during pouring and records no palm impacts.
Explicitly prompting a downward pour over the upward cupped left palm also
misses. Both attempts return the bottle and correctly retain the pour failure,
without starting washing. Evidence, source snapshots and all model attempts are
retained in `artifacts/toronto-condo-captures/shower-caddy-upright-v{1,2}/`.
The v2 directory includes a five-second `pickup.mp4` excerpt and `validation.json`.
Nine capture-example Rust tests pass; the Python suite has 15 passes and the
existing ownership source-text matcher failure. Structural checks still flag
duplication in the larger uncommitted caddy work.

The subsequent [semantic playback trial](../../artifacts/toronto-condo/20261019T054449Z-interact-plays-unconstrained-ardy-after-arrival-pickup-misse/view.mp4)
uses the same seed 731. All eight generated interaction blocks carry the native
semantic audit: zero future channels/values and no exceptions. The pickup misses
its contact zone (closest 35.6 cm, 0/175 observed frames) and times out after 10
seconds; no bottle attaches and pouring/set-down never start. This verifies the
unconstrained handoff, not successful manipulation. All 74 targeted Python tests
pass. Source snapshots, prepared/native audits and validation logs are retained
in `artifacts/toronto-condo-captures/shower-caddy-interact-v3/`.

The first `interact` run passed its pickup reach relation and attached the bottle,
then reported a shelf collision. It exposed an empty-box erosion bug in the
generic segment/cuboid test: negative half extents were being sorted back into
slabs. `ColliderBox::intersects` now treats that eroded box as empty. Regression
coverage includes rotated thin slabs, true contact and nonempty erosion; the
existing penetration tolerances remain unchanged.

The [interact retry](../../artifacts/toronto-condo/20261019T053403Z-interact-bottle-attachment-succeeds-but-sustained-hold-fails/view.mp4)
uses seed 731 with scene guidance disabled. The bottle attaches at native
16,049 ms, but the hold relation passes only 45/55 observed frames, below the
unchanged 90% requirement. Upward movement peaks at 3 mm; the coupled bottle
also moves beyond the west wall during holding. Pickup therefore fails before
the pour or set-down requests. This is not accepted manipulation. All 34
targeted Rust tests pass, including the thin-collider regression and existing
coupling, declared-interaction, support and native-seat checks. The known Python
source-slice failure remains. Both runs and source snapshots are retained under
`artifacts/toronto-condo-captures/shower-caddy-interact-v{1,2}/`.

The caddy variant moves the shared shower/bottle approach to 30 cm in front of
the shelf's measured bounds, still under the spray. The old shower destination
had only 18.6 cm clearance once the caddy was added; the unchanged native root
validator requires 20 cm.

The earlier [grasp attempt](../../artifacts/toronto-condo/20261019T051803Z-shower-caddy-reach-misses-physical-shampoo-grip/view.mp4)
completes normal navigation at native 12,605 ms, then fails the physical grasp at
17,301 ms. The closest sampled right wrist is 11.7 cm from the shampoo bottle,
outside the unchanged 5 cm grip bound. No bottle lift or dispensing occurred;
the pour-and-return prompt did not run. This did not establish an end-to-end
interaction. Both earlier configuration/clearance failures are also
retained in the gallery. Plans, source hashes and diagnostics are under
`artifacts/toronto-condo-captures/shower-caddy-v{1,2,3}/`.

All 36 targeted Rust tests pass. The Python ownership suite remains at 15 passes
and one pre-existing source-slice failure in
`test_root_acceptance_does_not_rewrite_the_candidate`; no policy or validator was
weakened. Source-model output, rendered reach, and successful prop manipulation
are separate evidence levels; only the first two were observed in this attempt.
Static checks report archive-source duplicates, test reachability and a complexity
score of 16 for the new event observer. Full reports and the fixed-seed attempt
plan are retained in `artifacts/toronto-condo-captures/water-washing-v1/`.

The first body-water GPU attempt omitted renderer start placement and was rejected
at its first native root. It is retained as a failed gallery entry, not collision
acceptance; the initial empty-shower image is evidence of that failure.

Earlier body-water recordings use the same compiled source and preserve the complete
eight-second sequence at 1920 × 1080:

- [Standing with moving arms and hair](../../artifacts/toronto-condo/20261019T035528Z-shower-water-moving-skin-runoff-final-standing-validation/view.mp4):
  554 peak runoff parcels, zero unresolved queries/dropped emissions, and zero
  remaining particles after shutoff. All 160 native frames were sent and playback
  reached 8,000 ms without motion rejection. Capture took 8.41 seconds;
  arrival median/p95 were 33.19/42.64 ms.
- [Walking into the running shower](../../artifacts/toronto-condo/20261019T035609Z-shower-entry-water-contact-with-native-late-wall-rejection/view.mp4):
  first observed body contact at encoded frame 97 while the valve was open,
  226 peak runoff parcels, zero unresolved queries/dropped emissions, and complete
  drainage. Capture took 8.34 seconds; arrival median/p95 were 33.01/37.09 ms.
  **The native action fails later:** ARDY's unchanged continuation crosses the
  west-wall clearance reserve at 7,679 ms; playback rejects it at 7,646 ms.
  This is water-contact evidence, not successful end-to-end shower entry.
  This free-form entry test is superseded by the normal Navigate command above.

One bounded semantic short-step retry was also retained. It passed root admission
but moved only about 6 cm and did not enter the shower. Neither candidate was
repaired. Service responses, conditioning audits and source equality reports are
under `artifacts/toronto-condo-captures/water-body-native-v1/` and `water-body-native-v2/`.

In the standing capture, active-water CPU skin/tree updates had a 26.75 ms median;
parcel simulation had a 1.75 ms median. GPU rendering is enabled, but collision
geometry remains CPU work. These short captures do not establish sustained 30 fps.
Earlier failed performance/collision attempts are preserved in the gallery.
Final source hashes, traces and test logs are under
`artifacts/toronto-condo-captures/water-body-validation/`: 13 water, 6 shared
collision, 12 tissue-system and 1 session-start Rust tests pass. The Python suite
still has the unchanged baseline failure described below. Static quality analysis
also flags the new Bevy systems as apparently unused and reports geometry-function
complexity; it is not a clean quality gate.

The earlier visual-particle baseline GPU recordings (1920 × 1080, 240 frames
each, before avatar collision support) are archived in the
[progress gallery](../../artifacts/toronto-condo/index.html):
[bathroom shower and vanity](../../artifacts/toronto-condo/20261019T023502Z-functional-bathroom-water-shower-and-vanity/view.mp4)
and [kitchen tap](../../artifacts/toronto-condo/20261019T023613Z-functional-kitchen-tap-flow-and-shutoff/view.mp4).
All eight commands were observed in each trace, and the final off views match
the initial dry views (no pixels differ by more than two 8-bit levels).
The kitchen camera uses `CC_AVATAR_CAMERA_POS=-4.4,1.5,-0.05`,
`CC_AVATAR_CAMERA_LOOK_AT=-5.55,0.97,-0.35`, and `CC_AVATAR_CAMERA_FOV_DEG=55`.
Validation reports and source hashes are in
`artifacts/toronto-condo-captures/water-validation/`.
The water and scene-package Rust checks pass. The repository Python suite has
one pre-existing failure in `test_root_acceptance_does_not_rewrite_the_candidate`:
its text slice includes unrelated methods; both scanned source and test are
unchanged from HEAD. The ownership gate has not been weakened.

## Baked sky-light diagnostic

The separate `scene-lightmap.json` package tests static sky irradiance on the
floor, ceiling, living rug, sofa and coffee table. Rebuild the normal scene
first, then run:

```sh
blender --background --python scripts/condo/bake_floor.py -- --room
python scripts/condo/encode_lightmap.py --oidn-lib /path/to/libOpenImageDenoise.so
```

Omit `-- --room` for the original floor-only experiment. This creates a separate
`toronto-condo-lightmap.blend`, `interior-city-lightmap.glb`, linear PNG maps,
float bake buffers, settings and a diagnostic scene manifest. Pass
`scenes/toronto_condo/scene-lightmap.json` to the capture tool. The normal
`scene.json` is unchanged. See gallery iterations 41 onward for reviews.

The Cycles bake uses a normalized white world, diffuse direct and indirect
passes, and excludes receiving-surface colour. Runtime sun, lamps and avatar
shadows are not baked. The images are explicitly linear and use a second UV
set; runtime strength scales with sky intensity, including weather transitions.
This follows Blender's [bake pass controls](https://docs.blender.org/manual/en/4.4/render/cycles/baking.html)
and Bevy 0.15.3's installed lightmap example. Only captured presets establish
visual evidence; this is not dynamic global illumination.

Use an OIDN build containing the dedicated `RTLightmap` filter. The official
[OIDN 2.5.1 Linux package](https://github.com/OpenImageDenoise/oidn/releases/download/v2.5.1/oidn-2.5.1.x86_64.linux.tar.gz)
worked here; the library bundled with this Blender build rejected that filter's
input combination. Denoising runs on the float irradiance buffers before PNG
quantization, preserves the raw buffers, and records the library hash. Omitting
`--oidn-lib` reproduces the noisy raw diagnostic from iteration 42. The filter's
purpose and HDR input contract are described in the [OIDN documentation](https://www.openimagedenoise.org/documentation.html#rtlightmap).

Package loading rejects missing/undecodable maps and invalid declarations.
Readiness waits for every declared mesh's map to attach and load. Capture
metadata hashes the map files. Moving furniture or changing architecture
invalidates baked occlusion and requires a new bake. The diagnostic package
must not be used to claim arbitrary movable-furniture lighting acceptance.

## Furniture and bedding interaction preparation

Visual detail work is deferred while mechanisms and deformable bedding are
prepared. See [the manipulation plan](../../docs/design/condo-manipulation.md).
Gallery 168 records the inventory and duvet topology; 169 records the separate
fridge asset's Blender physics fixture, including failed checks and corrected
full-frame screenshots. Entry 170 supplies verified final asset hashes and source
snapshots, correcting stale audit records in 169. These are asset diagnostics;
they do not enable avatar manipulation in the live condo.

Reproduce the isolated fridge checks from the repository root:

```sh
blender --background --threads 8 --python-exit-code 1 --python scripts/condo/fridge_mechanism.py
python scripts/condo/check_fridge_mechanism.py scenes/toronto_condo/interaction-authoring/fridge scenes/toronto_condo/scene-lightmap.json /tmp/fridge-sweep.json
blender --background --threads 8 --python-exit-code 1 --python scripts/condo/check_fridge_physics.py -- /tmp/fridge-physics-review
```

The physics fixture uses labelled lateral gravity loads to exercise passive
hinges. It contains no avatar, grasp controller or runtime success animation.
Preserve every new capture in a fresh gallery entry with `publish_progress.py`.

Checkpoint 172 adds an opt-in runtime package:

```sh
python scripts/condo/prepare_articulated_package.py
cargo test --offline --lib mechanism -- --nocapture
cargo build --offline --example condo_capture
```

Pass `scenes/toronto_condo/scene-articulated.json` to `condo_capture` to load the
separate fridge bodies and hinges. The original `scene.json` and
`scene-lightmap.json` retain the static fridge. Gallery 172 shows a closed-door
runtime preview; the force-driven open/close test is an isolated physics fixture.
Avatar manipulation, generated navigation and dynamic lighting acceptance
remain pending. Checkpoint 173 keeps held hinged parts in the observed obstacle
list and adds `navigation_geometry_sha256` to read-only waypoint previews.
`SceneState::waypoint_geometry_is_current` checks the latest geometry without
executing or replanning motion. Gallery 173 is a living-room regression capture
with the original avatar packets. See the manipulation plan for evidence and
limitations.

## Bed-sheet cloth diagnostic

```sh
blender --background --threads 8 --python-exit-code 1 --python scripts/condo/check_bed_sheet.py -- /tmp/condo-sheet-review
```

Use a new output directory. The script creates a separate, unpinned 0.8 kg
top sheet over the actual mattress, saves its initial Blender fixture and
material grid, then simulates frames 1–241 at 24 fps. It writes three screenshots,
effective coefficients/source provenance, final positions and per-frame metrics.
Opening the saved `.blend` starts at the initial state; play the timeline to
simulate it. The final observed mesh positions are a separate JSON artifact.

Acceptance requires bounded stretch/vertex penetration, low sustained speed,
successful solver steps and at least 40% of material vertices within 8 mm of an
upward mattress surface. This proximity measure is not a contact-force sensor.
An earlier low-speed false pass with the sheet still in mid-air is preserved in
the checkpoint 175 diagnostics; the support gate rejects it.

The reusable fixture is under `scenes/toronto_condo/interaction-authoring/bedding`.
It does not change the main condo, simulate fitted sheets or duvet layers, or
enable runtime hand grips. Coefficients remain provisional; full triangle
collision and pull/lift/release tests precede live avatar handling.

The separate local-constraint fixture exercises an initially attached patch:

```sh
blender --background --threads 6 --python-exit-code 1 --python scripts/condo/check_sheet_patch.py -- /tmp/condo-sheet-clamp video
blender --background --threads 6 --python-exit-code 1 --python scripts/condo/check_sheet_patch.py -- /tmp/condo-sheet-control control
ffmpeg -n -framerate 8 -pattern_type glob -i '/tmp/condo-sheet-clamp/video_frames/frame-*.png' -c:v libx264 -pix_fmt yuv420p /tmp/condo-sheet-clamp/view.mp4
```

The orange marker is a noncolliding synthetic target, not a hand. Three vertices
of one foot-edge material triangle start with compliant pin springs; the rest of
the sheet is free. The fixture lifts that patch and removes all three weights
before the target moves farther away. The control has no attachment at any time.
Both retain the flat material rest shape and start from the previously observed
drape with zero initial velocity. This tests a local constraint and release;
finger contact, acquisition policy and a barycentric contact solver are pending.
Four screenshots, per-frame observations and an initial-state check accompany
each run. The optional 8 fps preview records 121 frames across 15 simulated
seconds. The initial `.blend` contains the fixture animation, not a baked cache.

The Rust contact foundation can be checked with
`cargo test --offline --lib deformable_surface -- --nocapture`. It includes a
probe against the saved settled sheet and emits `CLOTH_CONTACT_REPORT` with
material coordinates and contact witnesses. Checkpoint 177 saves the report and
source/input hashes under `artifacts/toronto-condo-captures/contact177/`.
This is read-only geometry evidence; runtime cloth, hand-contact admission and
avatar manipulation remain pending. Surface revision checks do not certify an
unchanged hand collider: re-query current hand geometry before binding.

Checkpoint 178 adds a CPU solver prototype, exercised with
`cargo test --offline --lib cloth_solver -- --nocapture`. To retain the exact
simulated positions, set `CC_CLOTH_TRACE_DIR` to a new directory. The solver test
checks a 20-second gravity drop against a rounded mattress approximation; no
avatar or runtime scene binding is present yet. Render its recorded positions:

```sh
blender --background --threads 6 --python-exit-code 1 --python scripts/condo/preview_cloth_trace.py -- TRACE_DIR NEW_PREVIEW_DIR
```

This produces four screenshots without Blender cloth simulation or geometry
repair. Failed and corrected solver trials are retained under
`artifacts/toronto-condo-captures/cloth178/` and in gallery 178. See
`docs/design/condo-manipulation.md` for solver limits and outstanding integration.

Checkpoint 179 installs the solver in the opt-in `scene-cloth.json` package.
Build its assets with Blender:

```sh
blender --background --threads 6 --python-exit-code 1 --python scripts/condo/prepare_cloth_package.py
cargo test --offline --lib cloth
cargo build --offline --example condo_capture
```

Use this package with the existing `condo_capture` command and original native
packets; select `CC_CONDO_VIEW=bedroom` for the live sheet. It remains stationary
until native motion is displayed, then advances on that content clock and pauses
when the clock stops. A capture spanning the full original packet stream can be
checked with `python scripts/condo/audit_cloth_capture.py CAPTURE_DIR`; the audit
writes a new `cloth-audit.json` and never overwrites one. Gallery 179 contains the
24-second bedroom recording and a living-room regression with beautiful_assistant.
The package is a prototype: no hand attachment, body/pillow coupling, self-collision
or continuous collision proof; dark edge shading remains under review.

## Exterior wet film

`wet_surfaces.rs` and `wet_surfaces.wgsl` apply a cosmetic water film only to
both authored Toronto street batches and the balcony slab in the selected scene
GLB. The balcony mask keeps the threshold and sheltered strip dry, tapering
wetness toward the outer 0.8 m and side edges. Shared indoor stone materials are
not modified. The baked balcony lighting continues to follow the weather.

The film darkens paving and lowers roughness, with a restrained procedural-sky
sheen. It does not reflect buildings or the avatar, simulate puddle depth, or
change collision/friction. Weather snapshots expose `surface_wetness`; preset
rain starts wet, while transitions use 12-second wetting and 90-second drying
time constants. Changing weather does not reset accumulated wetness.

Gallery 164 preserves the dry-material rain baseline; 165 records the first
film review. Review 166 reduces the overly bright road sheen (sky blend 0.18
on asphalt, 0.35 on balcony stone; wet roughness 0.34/0.22). Its sampled indoor
and sheltered balcony regions are pixel-identical to 164. Review 167 records
654 frames through clear/overcast/rain/night with the original avatar playback.
Observed film rises to 0.376 and retains 0.354 after rain stops; weather and
wetness traces are archived. Complete evaporation is longer than this clip.
Tests cover exterior-only binding and wetting/drying across weather changes.
Long-run performance remains unverified. No motion source or pose processing
was changed by this feature.

Checkpoint 180 adds a **diagnostic contact clamp**, not runtime avatar handling:

```sh
CC_CLOTH_GRIP_TRACE=/tmp/new-cloth-grip-trace cargo test --offline --lib cloth_grip -- --nocapture
blender --background --threads 6 --python-exit-code 1 \
  --python scripts/condo/preview_cloth_trace.py -- \
  /tmp/new-cloth-grip-trace /tmp/new-cloth-grip-preview --frames 0 120 198 780
```

Both output directories must be new. The trace records all observed steps and
fails before claiming success if the fixture cannot lift, release or settle.
The preview's `capture.json` separately measures finite triangle samples against
the authored mattress. Checkpoint 180 passes the clamp tests but fails surface
clearance (up to 10.03 mm penetration between otherwise clear vertices).
No avatar handling or continuous collision correctness is certified.

Checkpoint 181 adds discrete triangle contacts. For an exact-runtime-topology
preview and an authored-mattress audit across every stored trace frame, use:

```sh
blender --background --threads 6 --python-exit-code 1 \
  --python scripts/condo/preview_cloth_trace.py -- TRACE_DIR NEW_PREVIEW_DIR \
  --frames 0 120 198 780 \
  --topology scenes/toronto_condo/interaction-authoring/bedding/runtime-top-sheet.json \
  --audit-all
```

`capture.json` distinguishes explicit runtime triangles from legacy Blender
quads and records the finite-sample `surface_audit`. The same 131-frame audit
finds 10.59 mm penetration in checkpoint 180 and zero sampled penetration in
181. This does not certify swept collision, self-collision or avatar handling.
Rebuild `condo_capture` before checking the changed solver in the live bedroom.

Checkpoint 182 publishes read-only hand/sheet observations in
`SceneState.cloths[*].hand_contacts` after transform propagation. The legacy
production finger-closing writer was removed. Distal capsules are rig geometry
approximations; observations never attach the sheet or certify a pinch. The
current 22-joint body envelope has no generated finger rotations.

```sh
cargo test --offline --lib cloth::contact
python scripts/condo/audit_cloth_contacts.py CAPTURE_DIR --expect-no-contact
```

Use `--expect-no-contact` only for a known negative control, such as the living
room while the sheet is in the bedroom. The audit verifies both hands' geometry,
clock/session agreement, contact revisions and unchanged root interpolation.
It writes a new `hand-contact-audit.json`. Positive geometry/ECS tests are
separate from the live negative control and from future avatar grasp evidence.

Checkpoint 183 profiles the same live scene and the existing 1,980-step condo
clamp fixture. Collision dominates CPU cost; hand observation is small. Both
optimization trials were reverted: typed dispatch did not improve speed, and
gradient-seeded queries failed the unchanged settling check. Logs, source
snapshots, full fixture trajectories and a timing/settling plot are in
`artifacts/toronto-condo-captures/perf183/`. Production behavior remains at
checkpoint 182; the profiling timers are absent from the final source.

Checkpoint 184 retains a development-profile optimization for `parry3d` only.
Rebuild the renderer to apply it. The unchanged 1,980-step condo fixture takes
36.03 seconds versus 43.51 seconds, with byte-identical positions, forces and
report. A renderer-crate optimization trial added no measured benefit and was
not retained. Release already uses optimization level 3; these measurements
cover the local development build. Full traces and build/test evidence are in
`artifacts/toronto-condo-captures/perf184/`.
