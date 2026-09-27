# Seat overlap warnings (default)

Owner-approved policy: this is an animation environment, not a strict physical
simulation. Native target-seat endpoint penetration and swept vertex overlap are
WARN by default. There is no 3mm hard cutoff. Motion is never repaired or snapped.

No environment setting is needed. `CC_RENDER_DEV_SEAT_WARN` is retired and ignored.
To opt into strict diagnostic rejection, start the renderer host with:

```sh
CC_RENDER_STRICT_SEAT_COLLISIONS=1 bash scripts/avatars/run_live_office.sh --port 8182
```

Policy is captured when native seat geometry is installed; rebuild the linked host
and restart to pick up code/configuration changes. Only exact value `1` opts in.
Strict mode rejects endpoint penetration or swept overlap beyond the 1mm tolerance.

Observations expose `collision_warning` plus actual penetration, session/revision
and displayed PTS. This replaces the development-only warning field. Lifecycle
contact checks do not re-escalate admitted overlaps into errors. Bilateral contact
and stability still determine seated completion; overlap alone is not support.

Geometry/identity/history errors, invalid motion, unrelated navigation obstacles,
deformation and grip-loss checks are unchanged. This policy specifically covers
the authenticated target seat, not a general navigation-obstacle bypass.

Tests cover opt-in strict rejection, warning-mode endpoint and swept overlap above
3mm, unchanged pose, invalid history, scope isolation, other obstacles and lifecycle
retention despite a warning. Rust suite:207 passed,2 ignored; ownership suite passes.
Evidence: /tmp/seat-default-warn-tests.log and /tmp/seat-default-warn-ownership.log.
Quality delta remains exit2 in the current worktree; no clean-quality claim.
No new live run performed for this policy change.
