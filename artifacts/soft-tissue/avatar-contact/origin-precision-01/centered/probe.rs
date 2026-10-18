mod cc_render {
    #[path = "/home/acidhax/dev/personal/remotemedia/cc-render-soft-tissue/artifacts/soft-tissue/avatar-contact/origin-precision-01/centered/soft_tissue/mod.rs"]
    pub mod soft_tissue;
}
use bevy::math::DVec3;
use cc_render::soft_tissue::*;

fn main() {
    let asset = TissueAsset::from_json(&std::fs::read(
        "assets/tissue/beautiful_assistant.base-quality-stabilized-volume-prototype.json").unwrap()).unwrap();
    for origin_y in [0.0, 1.3, 10.0] {
        for dt in [1.0 / 240.0, 1.0 / 960.0, 1.0 / 15360.0] {
            for compression in [0.000025, 0.0001, 0.001] {
                let region = &asset.regions[1];
                let frame = Frame { translation: DVec3::Y * origin_y, ..Frame::IDENTITY };
                let mut tissue = region.solver(frame).unwrap();
                let weights = [0.7768370156511999, 0.0, 0.0, 0.22316298434880016];
                let initial: DVec3 = region.tetrahedra[587].iter().zip(weights)
                    .map(|(i,w)| tissue.positions()[*i]*w).sum();
                let contact = PlaneContact {
                    point: EmbeddedPoint { tetrahedron: 587, barycentric: weights, influence: 1.0, offset: DVec3::ZERO },
                    surface_point: initial - DVec3::Z * compression,
                    normal: DVec3::NEG_Z,
                    stiffness_n_per_m: 614433.103241009,
                };
                let result = tissue.step_with_contacts(dt, frame, DVec3::NEG_Y*9.81, &[contact]);
                println!("{}",serde_json::json!({"origin_y":origin_y,"dt":dt,"compression":compression,
                    "error":result.as_ref().err().map(ToString::to_string),"metrics":result.ok(),
                    "positions_local":tissue.positions().iter().map(|x|(*x-frame.translation).to_array()).collect::<Vec<_>>()}));
            }
        }
    }
}
