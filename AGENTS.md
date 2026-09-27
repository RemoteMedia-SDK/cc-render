# Motion ownership (owner-mandated)

Owner-approved seat overlap policy: target-seat penetration and swept overlap are diagnostic WARN by default, with no 3mm fatal cutoff. `CC_RENDER_STRICT_SEAT_COLLISIONS=1` opts into strict checks. See `docs/design/development-seat-warnings.md`. Do not repair poses or treat these warnings as proof of physical support.

Read `docs/design/model-owned-motion.md` before changing motion, actions, physics interaction, retargeting, or acceptance tests.

- Task code describes semantic outcomes; it must not write pelvis/root paths, body heights/headings, joint targets, stance/gait timing, reference poses, or corrective transforms.
- A model-generated pose becomes a prohibited prescription if relocated, height-adjusted, scheduled or held by task code.
- Validators accept or reject unchanged motion. Never add IK, floor/seat snapping, smoothing, bone clamps or object actuators to pass a task.
- Unsupported actions fail explicitly. Do not restore retired behavior through flags, defaults, fallback clips or renamed helpers.
- Fixed action-independent bind/coordinate conversion and adjacent-frame interpolation are distinct from behavioral repair; changes need fidelity evidence.
- Owner approval is required for exceptions and for weakening protocol/policy/ownership tests. An implementing agent cannot approve its own exception.
- Run `python -m unittest discover -s tests -v` and relevant Rust tests. Preserve failed GPU attempts and report source-model proof separately from CC5/physics proof.

Migration note: the legacy office controller remains to be replaced. Its `motion_request` is rejected by the new worker. Do not add a compatibility translator that revives its timelines. Historical repair code compiled only under tests is not an implementation template.
