# Soft-tissue handoff — 2026-10-18

Checkpointed at the user's request to conserve the remaining weekly quota.
The volumetric replacement remains **opt-in and unfinished**. Broad bounce is
visible in the reviewed video; full avatar contact and smooth live playback
are still open.

Worktree: `/home/acidhax/dev/personal/remotemedia/cc-render-soft-tissue`.
Branch: `feat/breast-soft-tissue`. Runtime baseline: **`c098357`**. This handoff
and the rejected precision experiment are saved in a subsequent documentation
commit. Runtime source and the `tissue_lab` binary were restored to `c098357`;
the final rebuild passed. Other worktrees were not modified.

## View the result

- [Boundary-pressure comparison](../../artifacts/soft-tissue/volume-audit/tracked-off-on-boundary.mp4):
  the exact clip the user reviewed as “Looks much much better.”
- [Latest localized-volume comparison](../../artifacts/soft-tissue/quality-pressure/reviewed-vs-local-volume.mp4):
  broad movement with reduced local volume extremes; this newer clip has not
  received the same explicit user acceptance.
- [Review page](../../artifacts/soft-tissue/volume-audit/index.html).

The lab uses the base `beautiful_assistant.smpl_aligned.glb`, a focused camera
and native ARDY motion, without clothing or condo geometry. Selected asset:
`assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json`.
See [the design record](breast-soft-tissue.md) for input paths, hashes and solver
parameters. ARDY retains sole ownership of body/root/joint motion; preserve
unchanged packets and the [motion ownership contract](model-owned-motion.md).

## Saved implementation and regression evidence

- `26c57bd`: selected local-volume stabilization and broad deformation.
- `58ba162`: one preconditioner refresh after initial Newton updates, reducing
  measured CPU solve time by 6–7% with deformation retained to roundoff.
- `c098357`: bounded contact-penalty retries, retryable line-search failure,
  full diagnostic error chains, native revision handoffs in the lab, and
  preserved contact failures.

The latest no-contact jump completes all **478 frames through 8000 ms**, with
exactly unchanged nodes, attachments, rotations and surface displacement versus
the preceding saved baseline. Peak skin displacement is **32.56 mm**. This
trace-only regression did not produce a new video and has zero active contacts.
Evidence: [jump comparison](../../artifacts/soft-tissue/avatar-contact/chest-continuation-01/jump-comparison.json).

## Contact remains the main blocker

Use the existing **seed-739 native continuation**, which starts clear and reaches
contact. Standing is followed by a semantic chest-rub request using the service's
own history. All 160 packets remain unchanged. Initial arm clearance is 18.184 mm,
with zero tissue-edge/arm-face crossings. Seeds 736–738 remain preserved with
their initial-intersection or no-contact outcomes.

The bounded penalty update fixes the first contact step: residual overlap falls
from about 130 µm to **95.777 µm**, below the unchanged 100 µm limit. Its 153.24 N
reaction is a numerical observation, not anatomical force calibration.

**Update (residual-precision checkpoint):** the later 7666.67 ms
depth-6 failure was a line-search defect, not a precision floor. Measured on
the bit-exact recorded state: gradient noise 4e-12 N, representability floor
7e-9 N, condition 900, all far below the 1.33e-7 N limit; one Newton step
reaches 7e-9 N. The energy Armijo test rejected it because trial energies carry
~6e-15 J rounding against a ~1e-19 J decrease. `descend` now falls back to the
Hager–Zhang approximate Wolfe test through the gradient; convergence limits are
unchanged. The jump regression is identical in every value. The contact replay
now stops at **7671.35 ms (ContactUnresolved)** on skin samples embedded 78–89 %
in Fixed attachment nodes. See the
[residual-precision checkpoint](../../artifacts/soft-tissue/avatar-contact/residual-precision-01/README.md).
Coordinate-centering remains a rejected experiment
([precision experiment](../../artifacts/soft-tissue/avatar-contact/origin-precision-01/README.md);
[native contact checkpoint](../../artifacts/soft-tissue/avatar-contact/chest-continuation-01/README.md)).

## Verification and remaining limits

- **89 focused tissue tests pass** with the line-search change (87 earlier + 2
  regressions that fail without it); the native recording example's five pass.
- Python: **15 pass, one existing failure** in
  `test_root_acceptance_does_not_rewrite_the_candidate`. Its source slice includes
  an unrelated `for` beyond `resolve_root`. Protected assertions were unchanged.
- The final baseline `cargo build --offline --example tissue_lab` passes with
  38 existing warnings. The discarded precision experiment's 60 core tests
  passed, but its actual-avatar replay failed.
- Structural review findings remain recorded; the quality gate is not clean.
- Latest live evidence is roughly 24–25 delivered frames/s with a 73 ms p95
  delivery gap. Smooth 30 FPS and browser presentation latency are unproven.
- Anatomical calibration, reduced local distortion, complete contact playback,
  friction, edge/edge, mutual-breast and environmental contact remain open.
  Do not default-enable the replacement on the present evidence.

## Contact status after rim and pressure-cap work

- `0a8ee06`: compliant skin-side rim variants (`soften_tissue_rim.py`) and an
  attachment-aware pressure partition; the reviewed bounce is unchanged at 1e-5.
- Bounded contact pressure (default 10 kPa, `CC_AVATAR_TISSUE_CONTACT_PRESSURE_PA`):
  **the seed-739 chest rub now completes through 8000 ms** at 2/10/50 kPa on both
  rims. Hands visibly pass ~2–3 mm into the breasts (reported, not repaired).
  See the [pressure-cap checkpoint](../../artifacts/soft-tissue/avatar-contact/pressure-cap-02/README.md).

- Owner-approved exception (`docs/design/hand-ribcage-clamp-exception.md`,
  opt-in `CC_AVATAR_HAND_RIBCAGE_CLAMP=1`): wrist/palm ribcage guard plus
  finger conformance. On seed 739 only the right hand's fingers entered the
  ribcage (up to 23 mm); they now bend up to 14° with zero residual. See the
  [hand clamp checkpoint](../../artifacts/soft-tissue/avatar-contact/hand-clamp-01/README.md).

- Display-only hand rest on the solved breast skin (second exception
  amendment): works while corrections stay within 50 mm; seed 739's last
  ~150 ms need 58 mm to 30 cm (hand wedged between both breasts), so the
  source motion is the limit. See the
  [hand rest checkpoint](../../artifacts/soft-tissue/avatar-contact/hand-rest-01/README.md).

- Preregistered regeneration (12 semantic attempts): 3/12 pass the clamp and
  rest without unsupported frames; best `gentle-740` and `rub-740` show hands
  resting on the upper chest. Hands still pass through each other where they
  cross (outside the exception). See the
  [matrix checkpoint](../../artifacts/soft-tissue/avatar-contact/chest-matrix-01/README.md).

## Resume

1. Owner review of the finger-conformance and pressure-cap videos; choose the
   default cap (10 kPa now) and whether fingers should also rest on breast skin.
2. Live 120 Hz performance with cap and clamp; neither is measured live yet.
3. Default enablement still needs anatomical calibration and broader clips.

Run from this worktree:

```sh
cargo build --offline --example tissue_lab
cargo test --offline --lib tissue
python -m unittest discover -s tests -v
python artifacts/soft-tissue/quality-pressure/run_bevy.py \
  --fixed-only --trace-only --label next-contact-check \
  --output artifacts/soft-tissue/avatar-contact/next-contact-check \
  --packets artifacts/soft-tissue/lab-chest-continuation-739-01/packets.ndjson \
  --snapshot-pts-ms 7666 \
  --log-filter info,cc_render_plugin::cc_render::bevy_app::tissue_collision=debug
```

GPU access is required for Bevy. Use a fresh label and output path for each
attempt; the driver refuses to overwrite evidence. Omit `--trace-only` when
rendered frames are needed. External avatar inputs remain in the sibling
`remotemedia-sdk-office-main/avatars/` directory.

## Preservation and stopping reason

Necessary earlier temporary sources are preserved under
`artifacts/soft-tissue/preserved-tmp/` with a manifest. Current sources, scripts,
reports, source archives, selected videos and failed-run logs are in the
worktree. Ignored NDJSON inputs/traces are committed inside the contact
checkpoint's `source-and-capture-traces.tar.gz` and the precision experiment's
`traces.tar.gz`; member SHA-256 manifests accompany both. Extract from the
worktree root when reproducing from a fresh checkout. Raw PNG captures remain
locally under `artifacts/soft-tissue/`, ignored by Git. Rebuildable executables
are not required archival inputs. No necessary new work resides only in `/tmp`.

The earlier unexpected stop at **2026-10-18 21:20:45 UTC / 5:20:45 p.m. Toronto**
was a service interruption: `Selected model is at capacity. Please try a
different model.` (`server_overloaded`). It was not a physics-completion signal.
The goal tracker remains blocked from that interruption. This later checkpoint
is the requested quota wrap-up, not a claim that contact or performance is done.
