# Pressure discretization investigation, 2026-10-18

Historical investigation: the [boundary-pressure checkpoint](breast-soft-tissue-boundary-checkpoint.md)
supersedes the candidate selection and outstanding-work list below.

Goal remains active. This work resumes checkpoint `33bcb3d` in the isolated
`cc-render-soft-tissue` worktree. The previous goal turn made preservation progress;
this turn adds a measured diagnosis, an opt-in formulation and controlled renders.
**The new candidates are not accepted production physics.**

## Diagnosis and implementation

`artifacts/soft-tissue/locking-audit/linear_modes.py` assembles a small-strain
reference stiffness on the actual skin-volume mesh. At the same shear modulus,
bulk modulus, attachments and density, standard element pressure gives first
frequencies of 11.29/10.52 Hz; nodal pressure gives 6.65/6.95 Hz. Element divergence
has rank 331/333 and 315/315 free degrees of freedom, leaving almost no
volume-preserving deformation modes. This supports volumetric locking as a
contributor to the stiff response. The diagnostic omits loaded-state prestress
and finite-strain hardening; it is not a prediction of the full jump.

The new `average_nodal` material option averages tetrahedron volume changes in
nodal control volumes, following the single-material energy construction in
[Joldes, Wittek and Miller](https://isml.ecm.uwa.edu.au/ISML/Publication/pdfs/2009juljoldesmillerCNMEnon.pdf).
`pressure.rs` supplies the energy and its derivatives. The implicit matrix now
includes the wider coupling between nodes sharing a pressure patch. Bulk
viscosity uses the logarithm of the same averaged volume and the existing BDF2
history. Legacy edge damping retains its original geometric edges.

The current nodal formulation also uses isochoric corotated shear,
`Fbar = det(F)^(-1/3) F`, implemented in `isochoric.rs`. Its gradient is exact;
the tangent holds polar rotation fixed and uses a positive-semidefinite
approximation. Bulk pressure uses K, since the shear term is now isochoric.
The initial pressure-only candidate used unsplit corotated shear; its archived
sources and renders are retained as a rejected diagnostic.

The default remains element pressure. Existing assets and application defaults
are unchanged. No body/root/joint authoring or pose repair was added.

## Rendered evidence

[Latest material-sensitivity comparison](../../artifacts/soft-tissue/locking-audit/tracked-off-on-soft-isochoric.mp4)
shows physics off versus volumetric deformation, with identical torso crops,
source timing and no displacement multiplier. A
[local review page](../../artifacts/soft-tissue/locking-audit/index.html) links the
other two candidates. These clips are diagnostic, not approved realism samples.

All successful runs use the base avatar, unchanged 160-packet ARDY jump, the same
front camera, 60 output frames/s and 240 physics steps/s. Each completed 478
frames through 8000 ms. Encoded timestamps were checked against every source
timestamp. The rebuilt renderer has at most 1.49e-8 m attachment-position and
6.13e-8 quaternion-chord differences from the old binary; these are recorded,
not described as bit-identical. The undeformed surface at the snapshot matches
exactly. Source packets match byte-for-byte.

| Candidate | Peak skin displacement | Mean displacement at 7633 ms | Extreme element J over clip |
| --- | ---: | ---: | ---: |
| Previous skin-volume checkpoint | 15.04 mm | 2.12 mm | 0.866–1.382 |
| Nodal pressure, unsplit shear | 19.96 mm | 7.01 mm | 0.069–2.726 |
| Nodal pressure, isochoric shear | 19.47 mm | 7.07 mm | 0.285–4.048 |
| Isochoric, softer material sensitivity | 43.12 mm | 15.48 mm | 0.116–8.988 |

Means are area-weighted over the same skin-volume patch, not nipple-only
measurements. At 7633 ms, 56.6% of the patch moves over 10 mm in the softer
candidate, versus 0% previously. The softer material uses shear modulus 500 Pa
and shear viscosity 5 Pa s; other parameters are unchanged. It preserves the
shear relaxation ratio, but is **not avatar-specific material calibration**.
All runs registered zero contacts. No snapshot triangle rotated over 90 degrees
relative to its undeformed counterpart; this is not an intersection certificate.

The large element volume changes prevent acceptance. A static loaded-reference
audit shows these extremes are localized: for the softer candidate, less than
0.46% of reference volume lies in elements with more than 20% volume change, and
total volume changes by less than 0.027%. This static result does not establish
the dynamic distribution. Next work should measure volume-weighted dynamic
errors and inspect the offending tetrahedra before choosing mesh refinement or
stabilization; increasing visible amplitude alone is insufficient.

## Verification and retained attempts

- 71 focused Rust tissue tests passed. New checks cover affine pressure energy,
  rigid-motion invariance, nonlinear force/energy consistency with BDF2 viscosity,
  sparse coupling derivatives, fixed-node elimination, and distorted compression.
- Python suite: 15 passed, one previously documented ownership-test failure.
  Protected ownership tests were not changed. An edge-damping unit fixture now
  selects `damping_edges`, the renamed storage it directly exercises; its
  assertions are unchanged.
- Source whitespace checks pass. Structural review reports five gating findings
  in this prototype; no findings were blanket-acknowledged.
- First capture used software rendering because the sandbox lacked GPU access;
  it was explicitly terminated after verifying its process identity. The host
  GPU retry completed. Build/link failures during low disk space are retained.
  Only failed/superseded task-local link outputs were removed to make space.
- Source archives, build/test logs, binary/input hashes, launch records, geometry
  audits and video validation are under `artifacts/soft-tissue/locking-audit/`.
  Raw captures remain in adjacent `lab-*-front-*` directories, outside Git.

Interactive performance, active compression/contact, material/attachment
calibration, dynamic volume accuracy and visual acceptance remain incomplete.
