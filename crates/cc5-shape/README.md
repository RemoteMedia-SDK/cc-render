# cc5-shape

Apply Character Creator 5 shaping sliders (body type, height, face features — the
Morphs panel) to a character GLB without CC5. One Rust implementation, usable from
Rust and from CPython.

Inputs, produced once per character by
`remotemedia-sdk/scripts/avatars/cc5_editor/build_cc5_editor_pack.ps1`:

- `character.glb` — the character's reference export; every vertex carries a
  `_CCIDX` attribute (its CC5 vertex index);
- `pack/` — `pack.json` + `pack.bin`, what each slider does to that character.

See `../../docs/design/avatar-editor.md` for how the pack is captured and how
accurate it is.

## Rust

```toml
# from another repository
cc5-shape = { git = "https://github.com/RemoteMedia-SDK/cc-render", package = "cc5-shape" }
```

```rust
let pack = cc5_shape::ShapePack::load("pack".as_ref())?;
let weights = pack.weights_from([("Character Height", 0.5), ("Body Fat A", 0.3)])?;
let delta = pack.evaluate(&weights); // per-vertex / per-joint deltas, glTF frame (m)
let shaped = cc5_shape::shape_glb(&std::fs::read("character.glb")?, &pack, &delta)?;
std::fs::write("shaped.glb", shaped)?;
```

Sliders are found by id or by display name (an error if a name is ambiguous —
use the id then); values are range-checked.

Facial expressions (CC5's ~400 morph targets; Bevy loads at most 64 per mesh)
come as an expression pack (`expressions.json` + `expressions.bin`, built by
`build_expression_pack.py` from the reference FBX) in the same vertex space, for
CPU morphing on top of the shape:

```rust
let expr = cc5_shape::ExpressionPack::load("expressions".as_ref())?;
let mut w = expr.weights_from([("Eye_Blink_L", 1.0), ("Jaw_Open", 0.3)])?;
let arkit = cc5_shape::ArkitMap::load("character.arkit_map.json".as_ref(), &expr)?;
w.extend(arkit.weights([("mouthSmileLeft", 0.6)])?);
expr.add_to(&w, &mut delta.verts); // layer on a ShapeDelta (or expr.evaluate(&w))
```

HD characters: CC5 renders HD characters subdivided (level 1 = 4x faces, level 2 =
16x) with sculpted detail. `build_hd_map.py` (or `build_cc5_editor_pack.ps1 -HdLevel 1`)
writes a map from the pack's level-0 space to CC5's HD FBX export, so the same pack
shapes the HD mesh:

```rust
let hd = cc5_shape::SubdivMap::load("hd/map".as_ref())?;
hd.check(&pack)?;
let hd_delta = hd.expand(&pack.evaluate(&weights));
let shaped = cc5_shape::shape_glb_hd(&std::fs::read("hd/character_hd.glb")?, &pack, &hd, &hd_delta)?;
```

Exact for sliders that only reshape the base mesh (p99 within 0.02 mm of CC5's own
HD export). HD presets and large body sliders also change the HD level directly,
which a level-0 pack cannot see: p99 up to ~0.8 mm, max ~3 mm.

## Python

```sh
cd crates/cc5-shape
maturin build --release          # wheel: target/wheels/cc5_shape-*-cp310-abi3-*.whl
pip install ../../target/wheels/cc5_shape-*.whl
```

```python
import cc5_shape

pack = cc5_shape.Pack("pack")
w = pack.weights({"Character Height": 0.5, "Body Fat A": 0.3})
shaped = pack.shape_glb(open("character.glb", "rb").read(), w)

verts, joints = pack.evaluate(w)   # bytes: f32 xyz per vertex; {joint: (x, y, z)}
# numpy.frombuffer(verts, "<f4").reshape(-1, 3)
for category, indices in pack.categories():
    ...                            # pack.sliders[i]: id, name, category, min, max, base
```

```python
ex = cc5_shape.Expressions("expressions")
d = ex.evaluate({"Eye_Blink_L": 1.0})            # bytes: f32 xyz per vertex
arkit = cc5_shape.ArkitMap("character.arkit_map.json", ex)
d = ex.evaluate(arkit.weights({"jawOpen": 0.4}))
```

```python
hd = cc5_shape.SubdivMap("hd/map")
shaped = pack.shape_glb(open("hd/character_hd.glb", "rb").read(), w, hd=hd)
```

The wheel is abi3 (any CPython ≥ 3.10) and its output is byte-identical to the
Rust API's. Errors raise `ValueError`.
