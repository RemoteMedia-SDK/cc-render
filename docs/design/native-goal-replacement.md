# Pending native goal replacement

## Ordering bug and fix

A scene goal can be accepted but wait for an existing native source-token handoff. During that wait the generation driver still owns the older in-flight request. Previously:

- semantic goals were validated only against the driver, allowing a stale semantic revision to discard a newer pending scene goal;
- scene snapshots reported only the driver goal, hiding the newest accepted revision from UI/LLM producers.

All incoming motion goals now respect the pending revision floor. Revisions at or below a pending scene goal are rejected before state changes, regardless of whether the incoming goal is semantic or spatial. A strictly newer semantic goal may supersede the pending scene goal.

`office.motion` reports the newest accepted request, including a pending scene goal. `office.scene_lifecycle` and `avatar.native_stream` remain separate evidence of lifecycle/displayed ownership. A pending request never implies successful handoff or task completion. Lock acquisition retains delivery→task→driver order.

Replacing a goal does not interrupt or relabel generation already in flight. Only that request's matching session/revision/PTS ACK releases its slot. Old accepted frames cannot set the replacement's generation horizon. No generated poses, history, timestamps, collision policies or geometry are modified.

## Verification

Two new renderer regressions initially failed, then passed:

- `pending_scene_is_reported_as_requested_owner_not_displayed_completion` checks requested versus displayed ownership.
- `semantic_replacement_respects_pending_revision_and_in_flight_ack` checks rejected revisions2/3 against pending3, accepted replacement4, wrong-ACK rejection, old-output horizon isolation, and correct-ACK release.

Full renderer suite:210 passed,2 ignored; ownership tests:3 passed. SDK UI tests:41 passed plus TypeScript. UI regression confirms both semantic/spatial producers choose revision6 after observed pending5, even when their previous local request is revision1; no displayed completion is inferred.

Logs: `/tmp/replacement-{red-owner,red-order,green,ownership,edit-check,quality,ui-tests,ui-types,sdk-quality}.log`.

Renderer quality-delta remains exit2: short-horizon churn on the two modified production methods and increased size of the existing test module; no clean-quality claim. SDK quality-delta and renderer edit-check pass. No live GPU run of this replacement fix yet; previous full-workflow60s acceptance applies to the preceding reliability commit, not this change.

Remaining interruption work: real in-flight replacement and Stop/reconnect acceptance, process cleanup under interrupted generation, cancellation/status attribution. No new cancel_action compatibility path or pose hold is introduced.
