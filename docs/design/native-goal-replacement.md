# Pending native goal replacement

## Ordering bug and fix

A scene goal can be accepted but wait for an existing native source-token handoff. During that wait the generation driver still owns the older in-flight request. Previously:

- semantic goals were validated only against the driver, allowing a stale semantic revision to discard a newer pending scene goal;
- scene snapshots reported only the driver goal, hiding the newest accepted revision from UI/LLM producers.

All incoming motion goals now respect the pending revision floor. Revisions at or below a pending scene goal are rejected before state changes, regardless of whether the incoming goal is semantic or spatial. A strictly newer semantic goal may supersede the pending scene goal.

`office.motion` reports the newest accepted request, including a pending scene goal. `office.scene_lifecycle` and `avatar.native_stream` remain separate evidence of lifecycle/displayed ownership. A pending request never implies successful handoff or task completion. Lock acquisition retains delivery→task→driver order.

Replacing a goal does not interrupt or relabel generation already in flight. Only that request's matching session/revision/PTS ACK releases its slot. Old accepted frames cannot set the replacement's generation horizon. No generated poses, history, timestamps, collision policies or geometry are modified.

## Buffered semantic handoff

Subsequent live interruption testing exposed another issue: semantic dispatch cleared queued native frames before generation completed. Semantic replacements now also wait in the pending slot for the old ACK and an existing future source-token boundary. `SceneDelivery::request_anchor` selects a token up to600ms ahead with at least200ms lead (scene goals retain4400/2200ms). The Python service selects the identical token. A reserved semantic handoff must not be overwritten by an unanchored `NativeMotionRevision` command. Initial requests remain unchanged; no pose, history sample or timestamp is repaired.

The live replacement/Stop/reconnect test passed after this change; detailed failures, evidence and limits are in SDK `openspec/changes/add-model-owned-spatial-goals/INTERRUPTION-VALIDATION.md`. The following earlier checkpoint is retained for history.

## Initial revision-order verification

Two new renderer regressions initially failed, then passed:

- `pending_scene_is_reported_as_requested_owner_not_displayed_completion` checks requested versus displayed ownership.
- `semantic_replacement_respects_pending_revision_and_in_flight_ack` checks rejected revisions2/3 against pending3, accepted replacement4, wrong-ACK rejection, old-output horizon isolation, and correct-ACK release.

Full renderer suite:210 passed,2 ignored; ownership tests:3 passed. SDK UI tests:41 passed plus TypeScript. UI regression confirms both semantic/spatial producers choose revision6 after observed pending5, even when their previous local request is revision1; no displayed completion is inferred.

Logs: `/tmp/replacement-{red-owner,red-order,green,ownership,edit-check,quality,ui-tests,ui-types,sdk-quality}.log`.

Renderer quality-delta remains exit2: short-horizon churn on the two modified production methods and increased size of the existing test module; no clean-quality claim. SDK quality-delta and renderer edit-check pass. No live GPU run of this replacement fix yet; previous full-workflow60s acceptance applies to the preceding reliability commit, not this change.

Remaining interruption work: real in-flight replacement and Stop/reconnect acceptance, process cleanup under interrupted generation, cancellation/status attribution. No new cancel_action compatibility path or pose hold is introduced.
