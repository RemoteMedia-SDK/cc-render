# Model-owned motion: first implementation and proof

## Status

The generation boundary is implemented and exercised on real Core8 inference. This is **not** completion of the entire office/CC5/physics migration.

The worker now rejects the old protocol rather than silently accepting authored timelines. Existing office-action clients therefore fail closed until migrated. No compatibility translator has been added.

## Implemented

- Closed `motion_goal` v2 schema in Python and Rust; unknown fields, timelines, joint/root targets and legacy requests are rejected.
- `motion_policy.py`, `motion_service.py`, `motion_backend.py`: service-owned no-mask generation, model-generated history only, fixed bind retargeting, transactional suffix commit, immutable candidate validation, conditioning audit and artifact digest lineage.
- Replaced the 500-line legacy worker implementation with goal-only dispatch. The production generation path does not import reference-pose planners, timeline compilers or support/hand correction modules.
- SDK stream accepts v2 and surfaces an explicit error for v1; carries artifact audit messages. Daemon reload imports point to the new service.
- Removed production scheduling of contact IK, seated smoothing, finger posing and the scripted chair actuator. Removed the production pelvis-clamp call. Root collision validation accepts unchanged motion or rejects it; no seat adjustment or boundary clipping.
- Added repository instructions and CI boundary tests. Owner-hosted review/bypass protections remain an external setup requirement.

Historical renderer repair implementations are currently compiled only in unit tests. Other legacy controller/experiment modules remain to be removed after producer migration. They are not sanctioned templates or runtime fallback paths.

## Experiments

Fixed seeds: **42 and 73129**. Each semantic subgoal gets an eight-second operational budget; no pose deadline. Goal changes occur only after observed completion at a committed block boundary. The final run requires a continuous **400 ms timestamp span** of satisfied predicates and resets on missing observations.

| Final task | Seed 42 | Seed 73129 |
|---|---|---|
| Walk → deep squat → rise | Pass, 9.6 s | Failed squat within budget |
| Turn left → kneel → rise | Pass, 10.8 s | Pass, 9.2 s |
| Turn left → kneel → floor reach → rise | Failed floor reach | Failed floor reach |

“Pass” means the recorded geometric sequence predicates passed. It is not a certification of naturalness, foot friction, dynamic balance, CC5 mesh fidelity or furniture interaction. The exact predicates are in `prove_model_owned.py` and are tested separately. The successful kneel/rise isolation trial does **not** reclassify the harder floor-reaching task as successful.

Earlier exploration is preserved separately:

- Original wording: walk/squat/rise 1/2; floor-reaching sequence 0/2.
- Revised semantic wording: walk/squat/rise 2/2; floor-reaching sequence 0/2.
- Separate turn/kneel/rise isolation: 2/2.

An evidence bug was found in the exploratory harness: eight consecutive 20 Hz samples span 350 ms, although the original manifests labeled the dwell 400 ms. Those records were not overwritten. The final six attempts use the corrected timestamp gate, and the viewer labels preliminary evidence explicitly. This timing change also changes when semantic goals are submitted, so earlier success is not substituted for final success.

## Evidence

The session proof workspace contains:

- `index.html`: interactive raw Core27 skeleton playback with scrub/rotation controls, all 16 attempts and original failures.
- Per-run manifests, results and JSONL blocks with source joints/rotations, SMPL22 output, requests and audits.
- `audit-verification.json`: 455 accepted blocks across 16 attempts, all with zero future conditioning and no exceptions. Source-joint, source-rotation, root and quaternion digests were independently recomputed from recorded JSON; artifact and parent links were checked.
- `final/source/`: the six exact source files captured for the final experiment, matching its preregistered SHA-256 manifest. A subsequent harness-only helper extraction preserves the same semantics; archived sources remain the precise reproduction reference.

Raw latent/explicit model suffix tensors are hashed but not persisted by this first harness. The digests are integrity evidence, not signed model attestation. Full artifact storage/authentication and renderer-side enforcement remain future work.

The interactive viewer was checked in headless Chromium: cards loaded, animation controls/scrubbing worked, and JavaScript errors were fixed. This is source skeleton visualization, not a CC5 dynamic-render acceptance run.

## Verification

- Python avatar suite: **82 passed** (existing Ardy environment, CPU test execution).
- Rust library suite: **166 passed**.
- Renderer source ownership gates: **3 passed**.
- `cargo check --lib --offline`: passed; existing/dead migration-code warnings remain.
- OpenSpec `enforce-model-owned-motion --strict`: valid.
- `git diff --check`: clean in both repositories.
- Ripwire contract checks: no static signature incompatibility detected for `resolve_root`/`ArdySession`; this does not negate the deliberate wire-protocol break.
- Ripwire quality delta is **not clean** against HEAD. Both repositories started with dirty work; reports include existing controller complexity, migration dead code and new-symbol/reflection findings. This is not a merge-quality certification and unrelated dirty work was not rewritten to clear the score.

## What this establishes

The model can generate coordinated semantic transitions without prescribed pelvis tracks or corrective body poses. A successful task does not require those mechanisms. Precise kneeling floor reach remains unreliable with semantic conditioning alone; the implementation reports failure rather than restoring hand/root constraints.

## Remaining work

1. Replace the legacy office scheduler and all producers with the new goal boundary; reintroduce only genuinely supported model capabilities.
2. Remove retired authoring helpers and remaining diagnostic pathways entirely, rather than relying indefinitely on unreachable/test-only code.
3. Enforce service-issued committed artifacts through renderer transport and audit every skeleton writer and conversion path.
4. Persist raw model artifacts and rejected-candidate diagnostics for independent replay/verification.
5. Run CC5 dynamic playback/retarget fidelity gates and quantitative foot-support checks.
6. Establish a generic contact physics model and prove object interactions without goal-directed actuators. Chair pushing is not yet proven.
7. Configure required owner review and restricted bypass permissions in both repositories. Checked-in AGENTS/CI rules alone cannot prevent an authorized developer from weakening the boundary.
