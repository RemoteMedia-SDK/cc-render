# Preregistered chest-touch regeneration (semantic only)

Seed 739's chest rub wedged the right hand between both breasts, beyond any
display correction. The owner chose to regenerate the motion. The matrix,
seeds and pass criteria were written to `manifest.json` before any attempt
ran (`../generate_chest_touch_matrix.py`); every attempt is kept. Generation
is semantic only: one standing block, then the description from service-owned
history. All 12 audits are `model-owned-v1` with no future channels or
exceptions; packets equal the service responses.

## Result: 3/12 pass (`evaluation.json`)

Each attempt replayed in the tissue lab with `CC_AVATAR_HAND_RIBCAGE_CLAMP=1`
(10 kPa cap). Pass = completes 8000 ms, has contact, no unsupported ribcage or
breast-rest frames (corrections ≤ 50 mm), finger residual ≤ 1 mm.

| Description | Seeds 740 / 741 / 742 / 743 |
|---|---|
| “A person rests both hands flat on their chest.” | no contact / no contact / **pass** (max rest 11.9 mm) / no contact |
| “A person gently rubs their chest with both hands.” | **pass** (5.0 mm) / fail / fail / fail |
| “A person rubs their chest with both hands.” | **pass** (27.4 mm) / fail / fail / fail |

All 12 complete through 8000 ms and none needs an unsupported ribcage
correction. Failures are seed-739-like: the palm 39–53 mm into the breast.
ARDY has no knowledge of breast volume, so about three in four contact clips
press too deep for the approved 50 mm display rest.

## Review (`review/`)

`gentle-740` and `rub-740`, generated versus ribcage clamp plus breast rest,
real time and third speed. In both, the hands cross on the upper chest above
the breasts (contact 6.25–8 s and 5.78–8 s; peak skin displacement 5.4 and
15.4 mm). Nothing passes into the body or breasts. **The two hands pass
through each other** where they cross; hand-on-hand contact is outside the
approved exception and is not handled.

`generated-inputs-and-traces.tar.gz` holds every attempt's packets, requests,
responses and journal plus the passing evaluation traces (SHA-256 in
`archive-manifest.json`). Raw PNG frames are ignored; regenerate with the
recorded launches.
