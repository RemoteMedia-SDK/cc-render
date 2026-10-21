//! An equirectangular HDR (Radiance `.hdr`, e.g. CC5's `Image/Default_IBL.hdr`)
//! as Bevy environment-map cubemaps, made in memory: the diffuse map holds the
//! cosine-convolved radiance (irradiance / π, from 9 spherical harmonics), the
//! specular map one GGX-prefiltered mip per perceptual roughness step
//! (Bevy samples mip `roughness * (levels - 1)`). RGB9E5 texels.
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use rayon::prelude::*;
use std::f32::consts::PI;
use std::path::Path;

/// An equirectangular radiance map and its box-filtered mips.
struct Equirect {
    levels: Vec<(usize, usize, Vec<Vec3>)>,
}

/// Radiance `.hdr` (RGBE, run-length encoded scanlines or flat).
fn read_hdr(path: &Path) -> anyhow::Result<(usize, usize, Vec<Vec3>)> {
    let data = std::fs::read(path)?;
    let mut pos = 0;
    let mut line = || {
        let start = pos;
        while pos < data.len() && data[pos] != b'\n' {
            pos += 1;
        }
        pos += 1;
        String::from_utf8_lossy(&data[start..pos - 1]).into_owned()
    };
    loop {
        if line().trim().is_empty() {
            break;
        }
    }
    let dims = line();
    let parts: Vec<&str> = dims.split_whitespace().collect();
    anyhow::ensure!(parts.len() == 4 && parts[0] == "-Y" && parts[2] == "+X", "unsupported HDR layout {dims}");
    let (h, w): (usize, usize) = (parts[1].parse()?, parts[3].parse()?);
    let mut rgbe = vec![[0u8; 4]; w * h];
    for y in 0..h {
        let row = &mut rgbe[y * w..(y + 1) * w];
        if w >= 8 && w < 32768 && data[pos] == 2 && data[pos + 1] == 2 && data[pos + 2] & 0x80 == 0 {
            pos += 4;
            for c in 0..4 {
                let mut x = 0;
                while x < w {
                    let n = data[pos] as usize;
                    pos += 1;
                    if n > 128 {
                        let v = data[pos];
                        pos += 1;
                        for p in &mut row[x..x + n - 128] {
                            p[c] = v;
                        }
                        x += n - 128;
                    } else {
                        for (i, p) in row[x..x + n].iter_mut().enumerate() {
                            p[c] = data[pos + i];
                        }
                        pos += n;
                        x += n;
                    }
                }
            }
        } else {
            for p in row.iter_mut() {
                p.copy_from_slice(&data[pos..pos + 4]);
                pos += 4;
            }
        }
    }
    let px = rgbe
        .iter()
        .map(|p| {
            if p[3] == 0 {
                Vec3::ZERO
            } else {
                let f = 2f32.powi(p[3] as i32 - 136);
                Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) * f
            }
        })
        .collect();
    Ok((w, h, px))
}

impl Equirect {
    fn new(w: usize, h: usize, px: Vec<Vec3>) -> Self {
        let mut levels = vec![(w, h, px)];
        while levels.last().unwrap().1 > 1 {
            let (w, h, p) = levels.last().unwrap();
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            let mut n = vec![Vec3::ZERO; nw * nh];
            for y in 0..nh {
                for x in 0..nw {
                    let mut s = Vec3::ZERO;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        s += p[(2 * y + dy).min(h - 1) * w + (2 * x + dx).min(w - 1)];
                    }
                    n[y * nw + x] = s * 0.25;
                }
            }
            levels.push((nw, nh, n));
        }
        Self { levels }
    }

    /// Radiance towards world direction `d` (Y up; the map's centre column faces
    /// `+Z` rotated by `rot` about Y), bilinear at mip `lod` (nearest level).
    fn sample(&self, d: Vec3, rot: f32, lod: f32) -> Vec3 {
        let (w, h, p) = &self.levels[(lod.max(0.0).round() as usize).min(self.levels.len() - 1)];
        let lon = d.x.atan2(d.z) + rot;
        let u = (0.5 + lon / (2.0 * PI)).rem_euclid(1.0) * *w as f32 - 0.5;
        let v = d.y.clamp(-1.0, 1.0).acos() / PI * *h as f32 - 0.5;
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let at = |x: f32, y: f32| p[(y.clamp(0.0, *h as f32 - 1.0) as usize) * w + (x.rem_euclid(*w as f32) as usize)];
        let a = at(x0, y0).lerp(at(x0 + 1.0, y0), fx);
        let b = at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), fx);
        a.lerp(b, fy)
    }
}

/// World direction (Y up) of texel (x, y) of cube face `f` (wgpu order +X −X +Y −Y +Z −Z;
/// Bevy looks cubemaps up with z negated).
fn face_dir(f: usize, x: usize, y: usize, size: usize) -> Vec3 {
    let s = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let t = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    let c = match f {
        0 => Vec3::new(1.0, -t, -s),
        1 => Vec3::new(-1.0, -t, s),
        2 => Vec3::new(s, 1.0, t),
        3 => Vec3::new(s, -1.0, -t),
        4 => Vec3::new(s, -t, 1.0),
        _ => Vec3::new(-s, -t, -1.0),
    };
    Vec3::new(c.x, c.y, -c.z).normalize()
}

fn rgb9e5(c: Vec3) -> [u8; 4] {
    const MAX: f32 = 65408.0;
    let c = c.clamp(Vec3::ZERO, Vec3::splat(MAX));
    let m = c.max_element();
    let mut e = (m.max(1e-30).log2().floor() as i32).max(-16) + 1 + 15;
    let mut denom = 2f32.powi(e - 15 - 9);
    if (m / denom + 0.5).floor() as i32 == 512 {
        denom *= 2.0;
        e += 1;
    }
    let q = |v: f32| ((v / denom + 0.5).floor() as u32).min(511);
    let v = q(c.x) | (q(c.y) << 9) | (q(c.z) << 18) | ((e.clamp(0, 31) as u32) << 27);
    v.to_le_bytes()
}

fn cube_image(size: usize, mips: usize, layers: Vec<Vec<Vec<Vec3>>>) -> Image {
    // layer-major: every mip of face 0, then face 1, ...
    let mut data = Vec::new();
    for face in &layers {
        for mip in face {
            for c in mip {
                data.extend_from_slice(&rgb9e5(*c));
            }
        }
    }
    // (Image::new checks the data against mip 0 only)
    let mut img = Image { data, asset_usage: RenderAssetUsages::RENDER_WORLD, ..default() };
    img.texture_descriptor.size = Extent3d { width: size as u32, height: size as u32, depth_or_array_layers: 6 };
    img.texture_descriptor.dimension = TextureDimension::D2;
    img.texture_descriptor.format = TextureFormat::Rgb9e5Ufloat;
    img.texture_descriptor.mip_level_count = mips as u32;
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    img.sampler = ImageSampler::linear();
    img
}

fn hammersley(i: u32, n: u32) -> Vec2 {
    Vec2::new(i as f32 / n as f32, i.reverse_bits() as f32 * 2.328_306_4e-10)
}

/// (diffuse, specular) environment maps from `path`, rotated `rot` radians about Y.
pub fn environment_maps(path: &Path, rot: f32, images: &mut Assets<Image>) -> anyhow::Result<(Handle<Image>, Handle<Image>)> {
    let t0 = std::time::Instant::now();
    let (w, h, px) = read_hdr(path)?;
    let eq = Equirect::new(w, h, px);
    // diffuse: 9 SH coefficients from a 256-wide level
    let lvl = eq.levels.iter().position(|l| l.0 <= 256).unwrap_or(eq.levels.len() - 1);
    let (lw, lh, lp) = &eq.levels[lvl];
    let mut sh = [Vec3::ZERO; 9];
    for y in 0..*lh {
        let theta = (y as f32 + 0.5) / *lh as f32 * PI;
        let dw = (2.0 * PI / *lw as f32) * (PI / *lh as f32) * theta.sin();
        for x in 0..*lw {
            let lon = (x as f32 + 0.5) / *lw as f32 * 2.0 * PI - PI - rot;
            let d = Vec3::new(theta.sin() * lon.sin(), theta.cos(), theta.sin() * lon.cos());
            let c = lp[y * lw + x] * dw;
            for (k, b) in sh_basis(d).iter().enumerate() {
                sh[k] += c * *b;
            }
        }
    }
    // cosine lobe convolution (Ramamoorthi & Hanrahan), then / π: radiance of a white Lambertian surface
    let band = [PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0];
    let dsize = 32;
    let diffuse: Vec<Vec<Vec<Vec3>>> = (0..6)
        .map(|f| {
            vec![(0..dsize * dsize)
                .map(|i| {
                    let d = face_dir(f, i % dsize, i / dsize, dsize);
                    let b = sh_basis(d);
                    (0..9).fold(Vec3::ZERO, |a, k| a + sh[k] * band[k] * b[k]).max(Vec3::ZERO) / PI
                })
                .collect()]
        })
        .collect();
    // specular: GGX prefiltered mips, roughness = mip / (mips - 1)
    let ssize = 256usize;
    let mips = 9usize;
    let texel0 = 4.0 * PI / (6.0 * (w as f32 / 4.0).powi(2));
    let specular: Vec<Vec<Vec<Vec3>>> = (0..6)
        .map(|f| {
            (0..mips)
                .map(|m| {
                    let size = (ssize >> m).max(1);
                    let rough = m as f32 / (mips - 1) as f32;
                    let a = (rough * rough).max(1e-4);
                    let n_samples: u32 = if m == 0 { 1 } else { 96 };
                    (0..size * size)
                        .into_par_iter()
                        .map(|i| {
                            let n = face_dir(f, i % size, i / size, size);
                            if m == 0 {
                                return eq.sample(n, rot, 0.0);
                            }
                            let up = if n.y.abs() < 0.999 { Vec3::Y } else { Vec3::X };
                            let tx = up.cross(n).normalize();
                            let ty = n.cross(tx);
                            let (mut sum, mut wsum) = (Vec3::ZERO, 0.0);
                            for s in 0..n_samples {
                                let xi = hammersley(s, n_samples);
                                let phi = 2.0 * PI * xi.x;
                                let ct = ((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y)).sqrt();
                                let st = (1.0 - ct * ct).sqrt();
                                let hv = (tx * (st * phi.cos()) + ty * (st * phi.sin()) + n * ct).normalize();
                                let l = 2.0 * n.dot(hv) * hv - n;
                                let ndl = n.dot(l);
                                if ndl <= 0.0 {
                                    continue;
                                }
                                // filtered importance sampling: source mip from the sample's solid angle
                                let d = (ct * ct * (a * a - 1.0) + 1.0).powi(2);
                                let pdf = (a * a / (PI * d)) / 4.0;
                                let omega_s = 1.0 / (n_samples as f32 * pdf + 1e-6);
                                let lod = 0.5 * (omega_s / texel0).log2() + 1.0;
                                sum += eq.sample(l, rot, lod) * ndl;
                                wsum += ndl;
                            }
                            sum / wsum.max(1e-6)
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let d = images.add(cube_image(dsize, 1, diffuse));
    let s = images.add(cube_image(ssize, mips, specular));
    info!("ibl: {} ({w}x{h}) -> environment maps in {:.1} s", path.display(), t0.elapsed().as_secs_f32());
    Ok((d, s))
}

fn sh_basis(d: Vec3) -> [f32; 9] {
    let (x, y, z) = (d.x, d.y, d.z);
    [
        0.282095,
        0.488603 * y,
        0.488603 * z,
        0.488603 * x,
        1.092548 * x * y,
        1.092548 * y * z,
        0.315392 * (3.0 * z * z - 1.0),
        1.092548 * x * z,
        0.546274 * (x * x - y * y),
    ]
}
