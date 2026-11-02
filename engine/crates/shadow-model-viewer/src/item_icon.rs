//! Cached transparent thumbnails rendered from the player's own textured SMF models.
use crate::scene;
use bevy::{
    asset::RenderAssetUsages,
    mesh::VertexAttributeValues,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shadow_formats::AssetCatalog;
use std::error::Error;

pub fn render(catalog: &AssetCatalog, path: &str, wide: bool) -> Result<Image, Box<dyn Error>> {
    let model = scene::prepare(catalog, path, false)?;
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for part in &model.parts {
        if let Some(VertexAttributeValues::Float32x3(points)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        {
            for p in points {
                lo = lo.min(Vec3::from_array(*p));
                hi = hi.max(Vec3::from_array(*p));
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return Err("inventory model has no vertices".into());
    }
    let extent = hi - lo;
    let mut axes = [0usize, 1, 2];
    axes.sort_by(|a, b| extent[*b].total_cmp(&extent[*a]));
    let (x, y) = if wide {
        (axes[0], axes[1])
    } else {
        (axes[1], axes[0])
    };
    let z = axes[2];
    // A small oblique projection exposes depth while retaining the model's silhouette.
    let project = |p: Vec3| Vec3::new(p[x] + p[z] * 0.18, -p[y] + p[z] * 0.12, p[z]);
    let mut lower = Vec2::splat(f32::INFINITY);
    let mut upper = Vec2::splat(f32::NEG_INFINITY);
    for part in &model.parts {
        if let Some(VertexAttributeValues::Float32x3(points)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        {
            for p in points {
                let q = project(Vec3::from_array(*p)).truncate();
                lower = lower.min(q);
                upper = upper.max(q);
            }
        }
    }
    const W: usize = 256;
    const H: usize = 128;
    let scale = (Vec2::new(W as f32 - 12.0, H as f32 - 12.0)
        / (upper - lower).max(Vec2::splat(0.001)))
    .min_element();
    let center = (lower + upper) * 0.5;
    let mut rgba = vec![0u8; W * H * 4];
    let mut depth = vec![f32::NEG_INFINITY; W * H];
    let edge = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
    for part in &model.parts {
        let Some(VertexAttributeValues::Float32x3(points)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            continue;
        };
        let Some(VertexAttributeValues::Float32x2(uvs)) = part.mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            continue;
        };
        let Some(indices) = part.mesh.indices() else {
            continue;
        };
        let indices: Vec<_> = indices.iter().collect();
        let texture = part.texture.as_ref().and_then(|n| model.images.get(n));
        for tri in indices.as_chunks::<3>().0 {
            let v: Vec<_> = tri
                .iter()
                .map(|i| project(Vec3::from_array(points[*i])))
                .collect();
            let p: Vec<_> = v
                .iter()
                .map(|q| (q.truncate() - center) * scale + Vec2::new(W as f32, H as f32) * 0.5)
                .collect();
            let area = edge(p[0], p[1], p[2]);
            if area.abs() < 0.001 {
                continue;
            }
            let min = p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO);
            let max = p[0]
                .max(p[1])
                .max(p[2])
                .ceil()
                .min(Vec2::new(W as f32 - 1.0, H as f32 - 1.0));
            let n = (v[1] - v[0]).cross(v[2] - v[0]).normalize_or_zero();
            let light = 0.6 + 0.4 * n.dot(Vec3::new(-0.3, -0.5, 0.8).normalize()).abs();
            for py in min.y as usize..=max.y as usize {
                for px in min.x as usize..=max.x as usize {
                    let q = Vec2::new(px as f32 + 0.5, py as f32 + 0.5);
                    let b = Vec3::new(
                        edge(p[1], p[2], q),
                        edge(p[2], p[0], q),
                        edge(p[0], p[1], q),
                    ) / area;
                    if b.min_element() < -0.0001 {
                        continue;
                    }
                    let dz = b.x * v[0].z + b.y * v[1].z + b.z * v[2].z;
                    let pos = py * W + px;
                    if dz <= depth[pos] {
                        continue;
                    }
                    let uv = Vec2::from_array(uvs[tri[0]]) * b.x
                        + Vec2::from_array(uvs[tri[1]]) * b.y
                        + Vec2::from_array(uvs[tri[2]]) * b.z;
                    let color = texture
                        .and_then(|t| {
                            t.image.data.as_ref().map(|data| {
                                let s = t.image.size();
                                let tx = ((uv.x.rem_euclid(1.0) * s.x as f32) as u32).min(s.x - 1);
                                let ty = ((uv.y.rem_euclid(1.0) * s.y as f32) as u32).min(s.y - 1);
                                let i = ((ty * s.x + tx) * 4) as usize;
                                [data[i], data[i + 1], data[i + 2], data[i + 3]]
                            })
                        })
                        .unwrap_or([180, 180, 170, 255]);
                    if color[3] < 128 {
                        continue;
                    }
                    depth[pos] = dz;
                    for c in 0..3 {
                        rgba[pos * 4 + c] = (color[c] as f32 * light) as u8;
                    }
                    rgba[pos * 4 + 3] = color[3];
                }
            }
        }
    }
    let mut bounds = (W, H, 0usize, 0usize);
    for y in 0..H {
        for x in 0..W {
            if rgba[(y * W + x) * 4 + 3] > 0 {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x);
                bounds.3 = bounds.3.max(y);
            }
        }
    }
    if bounds.0 > bounds.2 || bounds.1 > bounds.3 {
        return Err("inventory model produced no visible pixels".into());
    }
    let width = bounds.2 - bounds.0 + 1;
    let height = bounds.3 - bounds.1 + 1;
    let mut cropped = Vec::with_capacity(width * height * 4);
    for y in bounds.1..=bounds.3 {
        cropped.extend_from_slice(&rgba[(y * W + bounds.0) * 4..(y * W + bounds.0 + width) * 4]);
    }
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        cropped,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    Ok(image)
}
