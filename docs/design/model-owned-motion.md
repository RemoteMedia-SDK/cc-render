# Model-owned motion: architecture and removal contract

Status: partially implemented; see `model-owned-motion-implementation.md` for limits.

## Owner clarification: spatial grounding is allowed

Scene-referenced contact regions and support surfaces are desired outcomes, not authored body motion. The owner has explicitly requested these inputs for ground touch and sitting. Ardy must still generate the approach, root trajectory, joint configuration and contact timing. The current semantic-only v2/zero-mask implementation is a baseline capability limit, not a permanent prohibition on environmental goal conditioning.

Read the adjacent SDK's `scripts/avatars/docs/spatial-conditioning-audit.md` and pending `openspec/changes/add-model-owned-spatial-goals/` before implementing spatial support. This clarification supersedes any interpretation below that would ban scene geometry itself; it does not authorize raw timed joint/root targets, hidden wrapper channels, reference-pose placement or renderer repair. The installed native world-hand encoder currently requires root XZ constraints, so simply enabling its end-effector wrapper is not an approved solution.

## Shower-caddy override (owner instruction, 2026-10-19)

For the current shampoo sequence, the owner explicitly instructed: "ignore the model-owned-motion". This supersedes the restriction below for guiding the bottle over the receiving palm and tilting it during dispensing. The implementation records its native hand position/orientation and required root channels and measures actual liquid contact before returning the bottle. Pickup, placement, normal navigation and the timed washing lifecycle remain separately observable. See the adjacent SDK `openspec/changes/add-guided-palm-dispensing/` for implementation and validation.

## Decision

**Task code describes desired outcomes. Ardy generates body motion. Physics determines object response. No other subsystem authors or repairs body motion.**

This applies to all actions, not just chair pushing: navigation, reaching, grabbing, carrying, sitting, standing, kneeling, jumping, gestures, recovery, and idle. A model limitation is an unsupported capability or a failed attempt, never permission to synthesize a successful animation.

The owner's prohibition on authored motion is the default. “Necessary” cannot be decided by an implementing agent, an environment variable, a prompt, or a request field. Exceptions require explicit owner approval under the process below.

This design supersedes conflicting behavior documented in the existing interactive-office design, including authored exit trajectories, contact correction, and bounded IK. Existing implementation and acceptance passes are not precedents for permitted behavior.

## Why the previous boundary failed

1. Task/controller code can construct root tracks, timed joint targets, fixed heights, headings, and full-body destinations.
2. The compiler treats those as trusted instructions and expands sparse-looking requests into dense constraints.
3. Upstream end-effector constraints also emit root X/Z, root height, and heading. Removing rotation indices does not remove those channels.
4. A pose sampled from a model is labeled model-generated even after application code relocates, height-adjusts, schedules, or indefinitely holds it. Model origin does not make subsequent prescription permissible.
5. Worker and renderer can mutate generated motion after sampling: hand orientation, history translation, IK, filtering, root correction, and finger posing.
6. Success tests mostly observe destination/contact/state-machine completion. They do not establish that displayed motion remained model-owned.
7. Environment flags and diagnostic paths retain alternative implementations that agents can reactivate.

The chair logs illustrate the failure: a stopped avatar is given about 0.20 m of dense root travel in about 0.25 seconds; its pelvis advances before its feet and the torso leans backward approximately 32 degrees. This is not evidence that Ardy needs a corrective torso pose.

## Ownership and data flow

```text
Task API -> Goal scheduler -> Motion service -> Candidate artifact
                  ^              ^                    |
                  |              |                    v
             Task outcome   World snapshot       Read-only validators
                  ^              ^                    |
                  |              |             accept / reject only
                  |              |                    v
             Observations <- Physics/world <- Committed motion
                                                 |
                                                 v
                                    Fixed retarget + playback
```

There is one body-pose writer at the renderer boundary. The scheduler, scene queries, validators, physics interaction code, and action implementations cannot obtain writable skeleton transforms or construct motion packets.

### Goal API: outcomes, not trajectories

A versioned replacement request carries:

- session and revision;
- semantic action and optional semantic style;
- object identity or world-space destination region;
- completion tolerances and an operational timeout;
- immutable world snapshot identity;
- references to the service's committed history, not caller-supplied pose arrays.

Examples: `push(chair, destination_region)`, `sit(chair)`, `walk(destination_region)`, `wave(left)`.

Forbidden fields include root/pelvis tracks, joint coordinates, bone rotations, root height, body heading tracks, pose timestamps, gait phase, hand-authored velocities, constraint masks, correction magnitudes, full-body references, and raw timeline layers. Arbitrary extension maps and passthrough JSON are also forbidden.

A task timeout limits effort; it does not specify the exact instant that a joint must reach a coordinate. World-space destination geometry describes success, not a sampled body path. A pathfinder may establish route feasibility or a spatial corridor; it cannot schedule pelvis positions along it.

### World facts are not body targets

World snapshots expose colliders, material parameters, object transforms/velocities, and observations with timestamps and provenance. Geometry queries may return actual surfaces and free-space regions. They may not manufacture a fixed pelvis clearance, seat height for the avatar, wrist spacing, or foot arrangement.

Separate scene collision/surface facts from old avatar-specific anchors such as `seat_pelvis` and `seat_left_foot`. Remove the latter from the motion-facing scene schema. Legacy rendering metadata must not silently become motion constraints.

### Motion service: the only generation authority

Only this service imports the model or constructs its native conditioning. Task handlers do not import Ardy constraint classes.

The initial compliant baseline is semantic generation from committed history, with coordinate calibration and observations. It does not promise successful precise interaction merely by replacing constraints with text.

A model/adapter capability matrix states what goal conditioning is genuinely supported. If Ardy cannot express an object-relative or spatial goal without a forbidden prescription, the service returns `unsupported_conditioning`. A learned goal-conditioned planner or explicit approved extension is a future capability, not a hidden scripted substitute.

The service may generate and evaluate bounded candidate continuations, then select an unchanged valid candidate. It may not repair, warp, splice an arbitrary posture into, or numerically optimize a body pose after generation. Rejected candidates never enter committed history. Retry budgets are fixed and failures observable; seeds cannot be cherry-picked to claim general success.

### Generated references are not a loophole

Delete the production `model_pose`/`generated_posture` reference-placement route. A separately sampled standing or sitting pose cannot be relocated to an authored root, height-adjusted, and imposed at a timestamp while retaining a claim of model-owned motion.

Observed history is valid context. Turning an observed foot position into an indefinite lock is a new prescription, not merely history. Learned continuation from that history is allowed; manually turning it into future joint constraints is not allowed by default.

### Constraint boundary and audit

For every request, the service records the actual expanded native conditioning channels, frame coverage, and provenance **before normalization and sampling**. Policy validation operates on emitted channels, not wrapper names or user-facing intent labels.

Baseline future body/root constraint channels are empty. History conditioning is separately identified. A fixed session coordinate transform is calibration, not future movement.

Native library version and adapter version are pinned. Adapter tests detect hidden channel changes, including end-effector root height/heading emission. Unexpected channels fail closed. Disabling a top-level height flag must not be reported as freeing height while another constraint still emits it.

### Retargeting and playback

Allowed transformations are a fixed, tested skeleton/coordinate conversion, bind calibration, and ordinary interpolation between adjacent committed samples. Calibration cannot depend on action, contact, furniture, success predicates, or per-frame target residuals.

Audit representation reconciliation such as position-derived bone alignment separately: retain only what is demonstrably necessary for faithful format conversion, with action-independent invariance and error tests. Do not delete essential bind transforms simply because they are called corrections.

Disallowed transformations include contact IK, seat-root offsets, body lean clamps, hand reorientation, finger grasp presets, action-specific smoothing, floor snapping, trajectory warping, and motion-history translation to hide tracking errors.

Source, retargeted, and displayed streams remain distinguishable in evidence. Root/world coordinate differences must be explained by the fixed calibration, not corrected back into history.

Facial expression/audio animation remains a separately scoped channel; it cannot write body or root channels.

### Physics and contact

Remove goal-directed furniture actuators and object position animation. Object response must arise from actual collision/contact or a general, explicitly modeled interaction mechanism—not `if push { force = 100 N }`, progress gates, or distance-to-goal force laws.

A kinematic generated avatar can transfer impulses to dynamic objects, but that alone is not a dynamically balanced human simulation. Document this limitation. Do not claim force-aware whole-body generation or mutual physical response unless the architecture actually provides it.

If animation-driven collisions destabilize the world, reject/pause/replan or report unsupported interaction. Do not introduce a task-specific force controller to make the acceptance result pass. Grasp attachments also require a separately defined generic physical-contact policy; proximity alone cannot create a scripted object follower.

### Failure and observation

Validators detect invalid geometry, collisions, motion discontinuities, foot sliding, unsupported posture, and goal progress. They return evidence and accept/reject decisions, never modified poses.

No default frozen standing pose, recovery clip, or corrective pelvis path. On generation failure or underrun, pause the simulation/motion clock at the last committed state and expose the reason. A paused state is not an animated idle and does not count as task success. Resumption must validate continuity and reconcile any externally changed world state.

Contact/approach labels may be inferred for observation and completion. Do not use a hand-authored phase machine to impose timed poses or fixed stance transitions. Completion requires sustained observed world/body evidence, not elapsed animation time.

## Enforceable module boundary

Proposed modules, introduced during migration:

| Location | Responsibility | Must not access |
|---|---|---|
| `cc-render/src/cc_render/office/motion_intent.rs` | Closed goal and observation schemas | Raw body channels, timeline passthrough |
| `cc-render/src/cc_render/office/controller.rs` | Scheduling, cancellation, outcomes | Constraint construction, writable skeleton |
| `cc-render/src/cc_render/office/furniture_push.rs` | Push feasibility and observed success | Body staging offsets, hand/root paths, object actuator |
| `remotemedia-sdk-office-main/scripts/avatars/motion_policy.py` | Effective conditioning allowlist/audit | Action-specific exceptions |
| `remotemedia-sdk-office-main/scripts/avatars/motion_service.py` | Generation and artifact ownership | Renderer correction hooks |
| `cc-render/src/cc_render/office/motion_commit.rs` | Validated artifact commit/playback handoff | Action-specific repair |
| `cc-render/src/cc_render/office/motion_validation.rs` | Read-only acceptance evidence | Mutable pose output |

Use private constructors and restricted module visibility for committed packets. Python raw model output stays internal to the service; public transport accepts goals, not arbitrary packet provenance assertions. The service issues artifact IDs and content digests tied to model invocation, history, conditioning audit, and calibration versions. A caller-supplied `source: ardy_generated_reference` string is not authorization or provenance.

These controls prevent routine bypasses; a developer who can edit the enforcement code can still weaken them. Repository protections below are therefore part of the architecture.

## Removal ledger

This is the confirmed inventory from inspection, not a claim that every repository write path has already been audited.

| Existing location | Required disposition |
|---|---|
| `office/controller.rs` | Remove authored action root paths, fixed heights/headings, joint targets, timeline layering, pose scheduling, idle/reference holds and action-dependent repairs. Retain scheduler and outcome observation. |
| `office/locomotion.rs` | Retain spatial route feasibility; remove timed 0.8 m/s pelvis paths and authored reaching curves. |
| `office/furniture_push.rs` | Retain scene feasibility; remove 30 cm body staging prescription and scheduled hand target generation. |
| `office/motion_timeline.rs` | Retire production body/root editing API and dense interpolation compiler input; reject old protocol rather than silently translating it. |
| `office/contact.rs` | Remove contact IK, seated body filtering and finger pose authoring from production body playback. |
| `office/support.rs`, `office/timeline.rs` | Audit/remove seat/support root overrides and world-motion corrections; retain observation and clock/commit responsibilities. |
| `office/systems.rs` | Remove push actuator and contact-correction registration; retain independent validators and observations. |
| `bevy_app/pose.rs` | Remove production reachability of diagnostic inclination/body overrides; retain verified action-independent retargeting and interpolation. Audit all skeleton transform writers. |
| `ardy_timeline.py` | Retire arbitrary future pose/root compilation; no endpoint-to-dense-track conversion. |
| `ardy_worker.py` | Remove manual future body constraints, hand rewriting and history translation; preserve raw generated committed history and generation transport. |
| `ardy_kneel.py`, `ardy_pose_presets.py` | Remove reference placement/hold and posture-template routes; no task-specific model-generated preset substitution. |
| `ardy_support.py` | Remove generated hand orientation rewriting and target-driven body modifications. |
| `ardy_adapter.py` | Audit faithful skeleton conversion; no action-dependent stabilization. |
| Scene JSON, acceptance drivers, runtime flags | Remove avatar-pose anchors and timeline injection from compliant mode; reject removed flags instead of leaving reactivation switches. |

Audit must also cover model bootstrap/history initialization, baked clip fallback, physics grasp followers, SDK request producers, transport compatibility paths, and every body-transform writer. Every finding gets a disposition; unknown behavior blocks compliance certification.

## Exceptions: owner authority, not developer discretion

Default exception registry is empty. There is no general `allow_override`, `unsafe`, or `necessary` field.

An exception proposal must identify the exact channels/transforms, scope and bounds, why model-native approaches cannot satisfy the need, reproduction evidence, expiry, and removal criteria. It requires explicit recorded owner approval. A developer's comment or self-authored approval file is not approval.

Approved implementation uses a narrowly scoped registered capability checked at service startup and invocation. Each activation is visible in artifacts and reports; the run is labeled exception-assisted, not baseline-compliant. An expired, unknown, or broader-than-approved request fails closed.

Operational approval evidence and protected policy changes must be controlled outside the implementing agent's writable authority. Exact mechanism depends on repository hosting. Do not pretend an editable local allowlist is a security boundary.

## Gates that prevent recurrence

1. **Schema negatives:** every removed root/joint/timeline field, unknown field, legacy version and unapproved capability is rejected at Rust and Python boundaries.
2. **Dependency boundaries:** task modules cannot import constraint builders, motion packet constructors or pose writers. Python AST and Rust/module checks enforce explicit allowlists; text searches are supplemental only.
3. **Actual-mask tests:** inspect all effective channels, including channels added by upstream end-effectors. The baseline rejects all future body prescriptions.
4. **Single-writer tests:** enumerate/register body transform writers and forbid new writers outside playback/calibration. Capture before/after snapshots around remaining pipeline stages to detect mutation.
5. **Provenance tests:** packet/history artifacts must trace to service-owned generation. Relabeled authored arrays and moved/held model references are rejected.
6. **No-repair tests:** invalid candidates fail without modification or commitment; validators have immutable inputs and no mutation outputs.
7. **Physics tests:** pushing without actual contact cannot move a chair; no action goal can directly supply object force or transform. Document the physical model rather than overstating it.
8. **Behavior regression matrix:** front/rear chair, short/long approaches, low/high surfaces, changed initial stance, cancellation and failure, across a preregistered seed set. Record all attempts, pelvis/feet/torso, contact and object trajectories. Do not bless success from a single selected seed or final position alone.
9. **Artifact gate:** acceptance reports include policy version, empty baseline exception list, constraint audit, source/retarget/display provenance, failures and videos. Missing evidence fails certification.
10. **Repository controls:** protect policy, protocol, model adapter, pose writer, physics authority, tests and CI configuration with required owner review and non-bypassable checks. Establish this in both repositories. CODEOWNERS alone is insufficient without branch protection and restricted bypass permissions; owner setup is an external prerequisite.

Add concise `AGENTS.md` rules in both affected repositories pointing to the ratified contract: never make a motion test pass by imposing body motion; report unsupported capability instead; modifying the gate requires owner review. Instructions support the boundary but do not replace it.

## Migration and acceptance sequence

### 1. Freeze and inventory

Preserve current dirty work and baseline artifacts. Inventory all motion producers, constraint channels, environment switches, physics actuators and transform writers. Do not bulk-delete unknown work. Record approved calibration separately from prohibited prescription.

### 2. Install the rejection boundary first

Introduce the closed goal protocol and baseline policy. Add failing tests for current bypasses, including hidden native root channels. Retire production legacy timeline acceptance. Any temporary comparison harness must be isolated and clearly noncompliant; it must not be a runtime fallback.

### 3. Establish the minimal honest baseline

Run semantic Ardy generation, unchanged history, fixed retargeting, playback and read-only validation. Prove lineage through displayed motion. Unsupported precise goals fail explicitly. This may temporarily reduce capabilities.

### 4. Remove prescriptions end to end

Work through the removal ledger across both repositories. Delete dead flags and helpers, not merely their default activation. Replace tests that assert authored values with ownership and observed-behavior tests. Update the old office design and README claims so they cannot authorize regression.

### 5. Reintroduce interactions only through the permitted boundary

Evaluate actual model capabilities for navigation and each interaction. Add model-native/learned capability support where feasible. Validate generic contact physics separately. Do not restore a removed action by recreating its root path, IK or reference pose under another name.

### 6. Certify and protect

Run policy/unit/adapter tests and the preregistered rendered matrix. Review failures as failures. Complete owner repository protection setup. Merge only when runtime paths, documentation and enforcement agree.

## Definition of done

- No reachable unapproved authored body constraints or post-generation body repair.
- No goal-directed object actuator masquerading as physical interaction.
- All displayed motion has service-owned generation lineage and only approved action-independent conversion/interpolation.
- Legacy inputs, hidden native channels, diagnostic switches and unsupported capabilities fail closed.
- Tests assess ownership and motion quality, not just task completion.
- The owner can see every exception and has exclusive approval authority.
- Lost capabilities are explicitly documented; no successful push, sit or walk is promised until validated under this architecture.

## Open decisions requiring evidence

- Which spatial/object-relative goals the installed Ardy model can support without prescriptions.
- Whether precise dynamic pushing requires a learned interaction controller or another model rather than this Ardy configuration.
- The generic collision/grasp model and its physical limits.
- Which existing representation conversions are essential faithful retargeting versus behavioral correction.
- Hosting-specific branch protections and owner-controlled exception authorization.

These are investigation gates, not invitations for implementing agents to fill gaps with hard-coded motion.
