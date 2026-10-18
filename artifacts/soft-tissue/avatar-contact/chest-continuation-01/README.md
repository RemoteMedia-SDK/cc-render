# Native approach contact checkpoint

This checkpoint is **not a completed contact validation**. The selected base-avatar
material and accepted jump remain the baseline. The replacement remains opt-in.

## Reproduction and source ownership

`../generate_chest_touch.py --continuation` generated one bounded seed-739
attempt: one native block of standing, then a semantic chest-rub request from
service-owned history. Revision 1 ends at 2000 ms; revision 2 starts at 2050 ms
with the service history anchor at 2000 ms. All 160 packets are unchanged service
output; all future constraint channels and exception lists are empty.

The lab now accepts the shared native loader's already-validated revision
handoffs and preserves their request/response provenance. No body poses, root
paths, timing, retargeting, material or attachment masks were edited.

Independent initial mesh checks find an 18.184 mm arm gap, 2.088 mm body gap and
zero tissue-edge/arm-face crossings. Earlier seed 736 was rejected for initial
intersection; 737 and 738 completed with zero active contact. Every attempt is
retained rather than presented as a contact success.

## Findings and changes

The original replay fails at 7666 ms. Contact is detected on right-breast skin
vertex 4955, embedded in tet 587 with 22.3163% free-node weight. Repeating the
same finite-penalty solve leaves approximately 130 um overlap above the 100 um
residual limit. The existing 500 um clearance is also unchanged.

Contact now increases its numerical penalty only when residual overlap exceeds
the limit. Each attempt re-solves the same original state; the maximum is eight
contact rounds and a 4096 pressure multiplier. Sample area remains normalized
across neighboring triangle witnesses. This does not change anatomical material
stiffness and does not project skin or repair body motion.

This bounded continuation follows the general contact-stiffness updating approach
in [Ansys contact documentation, section 3.9.4.5](https://ansyshelp.ansys.com/public/Views/Secured/corp/v252/en/ans_ctec/Hlp_ctec_realkey.html).
The precise bounds are this implementation's choices, not values from that source.

The first contact step now passes at **95.777 um overlap** with multiplier
2.60395 and 153.24 N reaction. These are numerical measurements, not anatomical
force calibration. Deeper contact still fails; the experiment does not establish
that this force response is realistic.

An exhausted nonlinear line search was classified as malformed input. It now
has a distinct solver error, so the existing bounded timestep subdivision can
retry it. Terminal errors retain the complete underlying error chain.

## Remaining failure

`bevy-retry-01` still stops in 7666.666667..7666.731771 ms at subdivision depth 6.
Its final nonlinear force residual is 1.11205674e-6 N and remaining correction is
9.76712257e-14 m. The small-step force budget is approximately 1.332e-7 N (1e-8 N floor),
using the last accepted 240 Hz step in the BDF2 derivative. Numerical
precision/scaling needs investigation; no tolerance was relaxed to accept it.
The outer error's generic region-0 context remains a diagnostic limitation;
its underlying contact-solve context correctly identifies region 1.

`bevy-penalty-01` and `bevy-retry-01` retain partial PNG captures. They end before
a completed contact frame and are not a successful contact video. `bevy-debug-01`
retains the original detailed sample/plane diagnostics. `result.json` records
the current limits; launch files archive inputs, source and binary hashes.

## Checks

- The partly fixed skin regression fails before the change and passes after.
- An immovable skin embedding still rejects overlap; original states and fixed
  attachments remain unchanged.
- All 87 tissue tests pass, including the new line-search failure classification.
- The shared native recording example's five tests pass.
- Python: 15 pass, one pre-existing ownership source-slice failure remains.
  Protected assertions are unchanged.
- Structural review removed the newly added resolve complexity and reused the
  existing tetrahedron fixture. Remaining reported debt is recorded in
  `quality-02.xml`: historical churn, test-module length, minor method length,
  and unrecognized Rust test registrations. The gate is not reported clean.
- The no-contact baseline replay completes all 478 frames through 8000 ms.
  Node positions, anchors and surface displacement are exactly unchanged; peak
  skin displacement remains 32.5599 mm. See `jump-comparison.json`. A successful
  jump does not establish contact correctness.
- `source-and-capture-traces.tar.gz` preserves all ignored NDJSON inputs and
  traces from these attempts; every member was checked against its SHA-256 in
  `trace-archive-manifest.json`. Necessary work is in the worktree, not `/tmp`.
