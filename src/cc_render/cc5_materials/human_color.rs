//! CC5's "Human Color" (RLHead / RLSkin `_Active Human Color` + `_Active`):
//! a per-channel gain on the sRGB diffuse, set by the `_Red`, `_Yellow`,
//! `_Black` and `_Range` sliders. Measured from CC5's baked exports
//! (`EnableBakeDiffuseFromSkinColor`) of one character with one slider changed
//! at a time; gains interpolate (in log) between the measured slider values and
//! combine multiplicatively.
use bevy::math::Vec3;
use serde_json::Value;

/// The gain at the reference slider values (`REF`) and skin colour range (`REF_SCR`).
const REF_GAIN: [f32; 3] = [1.266, 1.300, 1.313];
/// `_Skin Color Range` of the reference (sRGB 0..255; the head material it was measured on).
const REF_SCR: [f32; 3] = [190.0, 136.0, 112.0];
/// Green and blue scale as (their share of the skin colour range against red's)^SCR_EXP:
/// matches CC5 for other ranges and for the body materials' own range.
const SCR_EXP: f32 = -0.46;
/// _Red, _Yellow, _Black, _Range of the reference.
const REF: [f32; 4] = [24.285, 22.058, 0.0, 0.5];

/// Per slider: (value, gain relative to the reference) samples, sorted by value.
const RED: &[(f32, [f32; 3])] = &[(0.0, [1.0060, 1.0107, 1.0113]), (24.285, [1.0, 1.0, 1.0]), (50.0, [0.9358, 0.8916, 0.8997]), (80.0, [0.8332, 0.7099, 0.7267]), (100.0, [0.8000, 0.6412, 0.6595])];
const YELLOW: &[(f32, [f32; 3])] = &[(0.0, [1.0095, 1.0114, 1.0162]), (22.058, [1.0, 1.0, 1.0]), (50.0, [0.9445, 0.9390, 0.9165]), (80.0, [0.8692, 0.8465, 0.7902]), (100.0, [0.8459, 0.8192, 0.7502])];
const BLACK: &[(f32, [f32; 3])] = &[(0.0, [1.0, 1.0, 1.0]), (25.0, [0.8925, 0.8896, 0.8881]), (50.0, [0.7661, 0.7597, 0.7573]), (100.0, [0.3767, 0.3387, 0.3171])];
// (red at 1: clipped white in every texel; taken as green's)
const RANGE: &[(f32, [f32; 3])] = &[(0.0, [0.7300, 0.7295, 0.7293]), (0.25, [0.8321, 0.8318, 0.8316]), (0.5, [1.0, 1.0, 1.0]), (0.75, [1.3674, 1.3703, 1.3707]), (1.0, [1.8025, 1.8025, 1.6087])];

/// ln(gain) at `x`: piecewise linear between samples, extended linearly past the ends.
fn interp(table: &[(f32, [f32; 3])], x: f32) -> Vec3 {
    let ln = |g: [f32; 3]| Vec3::new(g[0].ln(), g[1].ln(), g[2].ln());
    if table.len() == 1 {
        return ln(table[0].1);
    }
    let i = table.partition_point(|s| s.0 < x).clamp(1, table.len() - 1);
    let (a, b) = (table[i - 1], table[i]);
    let t = (x - a.0) / (b.0 - a.0);
    ln(a.1).lerp(ln(b.1), t)
}

fn var(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32)
}

/// The human colour gain for a material's CC5 variables.
pub fn gain(v: &Value) -> Vec3 {
    let p = [var(v, "_Red", REF[0]), var(v, "_Yellow", REF[1]), var(v, "_Black", REF[2]), var(v, "_Range", REF[3])];
    let ln = Vec3::from(REF_GAIN.map(f32::ln))
        + interp(RED, p[0])
        + interp(YELLOW, p[1])
        + interp(BLACK, p[2])
        + interp(RANGE, p[3]);
    let scr = |i: usize| v.get("_Skin Color Range").and_then(|a| a.get(i)).and_then(Value::as_f64).map_or(REF_SCR[i], |x| (x as f32).max(1.0));
    let (r, g, b) = (scr(0), scr(1), scr(2));
    let ln = ln + Vec3::new(0.0, SCR_EXP * ((g / r) / (REF_SCR[1] / REF_SCR[0])).ln(), SCR_EXP * ((b / r) / (REF_SCR[2] / REF_SCR[0])).ln());
    Vec3::new(ln.x.exp(), ln.y.exp(), ln.z.exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn measured_points() {
        let g = gain(&json!({"_Red": 24.285, "_Yellow": 22.058, "_Black": 0.0, "_Range": 0.5}));
        assert!((g - Vec3::from(REF_GAIN)).abs().max_element() < 1e-4);
        // the body's own skin colour range: CC5 gave (1.266, 1.281, 1.359)
        let g = gain(&json!({"_Skin Color Range": [179.0, 132.0, 98.0]}));
        assert!((g - Vec3::new(1.266, 1.281, 1.359)).abs().max_element() < 0.01, "{g}");
        // all four sliders at once: CC5 gave (0.694, 0.656, 0.647) x the reference
        let g = gain(&json!({"_Red": 50.0, "_Yellow": 40.0, "_Black": 25.0, "_Range": 0.3}));
        assert!((g / Vec3::from(REF_GAIN) - Vec3::new(0.694, 0.656, 0.647)).abs().max_element() < 0.01, "{g}");
        let g = gain(&json!({"_Red": 80.0, "_Yellow": 22.058, "_Black": 0.0, "_Range": 0.5}));
        assert!((g.y - REF_GAIN[1] * 0.7099).abs() < 1e-3, "{g}");
        // between samples: between their gains
        let g = gain(&json!({"_Black": 37.5}));
        assert!(g.x < REF_GAIN[0] * 0.8925 && g.x > REF_GAIN[0] * 0.7661);
    }
}
