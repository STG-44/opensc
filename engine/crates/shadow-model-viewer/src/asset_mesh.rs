//! Геометрия без масштаба витрины: подходит для частей персонажа и предметов.
use bevy::{
    asset::RenderAssetUsages,
    image::{
        CompressedImageFormats, ImageAddressMode, ImageSampler, ImageSamplerDescriptor, ImageType,
    },
    mesh::Indices,
    prelude::*,
    render::render_resource::{PrimitiveTopology, TextureFormat},
};
use shadow_formats::{AssetCatalog, SmfMesh};
use std::error::Error;
pub fn mesh(source: &SmfMesh, transform: Mat4) -> Mesh {
    let positions = source
        .vertices
        .iter()
        .map(|v| {
            transform
                .transform_point3(Vec3::from_array(v.position))
                .to_array()
        })
        .collect::<Vec<_>>();
    let mirrored = transform.determinant() < 0.0;
    let indices = source
        .faces
        .iter()
        .flat_map(|f| {
            let [a, b, c] = f.indices;
            if mirrored { [a, b, c] } else { [a, c, b] }
        })
        .collect::<Vec<_>>();
    let fallback = crate::normals::reconstructed_normals(&positions, &indices);
    let normal_matrix = transform.inverse().transpose();
    let normals = source
        .vertices
        .iter()
        .zip(fallback)
        .map(|(v, f)| {
            v.normal
                .and_then(|n| {
                    normal_matrix
                        .transform_vector3(Vec3::from_array(n))
                        .try_normalize()
                })
                .map(|n| n.to_array())
                .unwrap_or(f)
        })
        .collect::<Vec<_>>();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        source.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
    )
    .with_inserted_indices(Indices::U32(indices))
}
pub struct Texture {
    pub image: Image,
    pub alpha_mode: AlphaMode,
}

pub fn image(catalog: &AssetCatalog, path: &str) -> Result<Texture, Box<dyn Error>> {
    let decoded = Image::from_buffer(
        &catalog.read(path)?,
        ImageType::Extension(path.rsplit('.').next().ok_or("нет расширения текстуры")?),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::linear()
        }),
        RenderAssetUsages::default(),
    )?;
    let sampler = decoded.sampler.clone();
    let mut image = if decoded.texture_descriptor.format == TextureFormat::Rgba8UnormSrgb {
        decoded
    } else {
        let mut converted = decoded
            .convert(TextureFormat::Rgba8UnormSrgb)
            .ok_or("текстуру нельзя преобразовать в RGBA8")?;
        converted.sampler = sampler;
        converted
    };
    let (stem, extension) = path.rsplit_once('.').ok_or("нет расширения текстуры")?;
    // Оригинал 0x48ACD0 ищет RAW рядом с 24-bit BMP/JPG. PNG-замены
    // используют собственную альфу, без унаследованной маски другого размера.
    let embedded_alpha = image
        .data
        .as_ref()
        .is_some_and(|data| data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255));
    let raw_path = format!("{stem}.raw");
    let legacy = matches!(extension, "bmp" | "jpg" | "jpeg") && !embedded_alpha;
    let raw = if legacy && catalog.paths().any(|p| p == raw_path) {
        Some(catalog.read(&raw_path)?)
    } else {
        None
    };
    let color_key = legacy && stem.rsplit('/').next().is_some_and(|s| s.contains("_ck"));
    let alpha_mode = apply_alpha(&mut image, raw.as_deref(), color_key)
        .map_err(|error| format!("{path}: {error}"))?;
    Ok(Texture { image, alpha_mode })
}

fn apply_alpha(
    image: &mut Image,
    raw: Option<&[u8]>,
    color_key: bool,
) -> Result<AlphaMode, Box<dyn Error>> {
    let size = image.texture_descriptor.size;
    let pixels = (size.width as usize)
        .checked_mul(size.height as usize)
        .ok_or("переполнение размера текстуры")?;
    let data = image.data.as_mut().ok_or("нет пикселей текстуры")?;
    if data.len() != pixels.checked_mul(4).ok_or("переполнение RGBA")? {
        return Err("неверный размер RGBA8".into());
    }
    if let Some(raw) = raw {
        if raw.len() != pixels {
            return Err(format!(
                "RAW: {} байт, ожидается {pixels} для {}×{}; замени BMP и RAW вместе",
                raw.len(),
                size.width,
                size.height
            )
            .into());
        }
        // RAW идёт сверху вниз; BMP-декодер уже перевернул нижний ряд файла.
        // 0x48747D/0x4874A9 берёт RAW[y * width + x] для строки поверхности y.
        for (pixel, alpha) in data.as_chunks_mut::<4>().0.iter_mut().zip(raw) {
            pixel[3] = *alpha;
        }
    } else if color_key && data.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255) {
        for pixel in data.as_chunks_mut::<4>().0.iter_mut() {
            if pixel[..3] == [0, 0, 0] {
                pixel[3] = 0;
            }
        }
        return Ok(AlphaMode::Mask(0.5));
    }
    Ok(
        if data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use shadow_formats::{SmfFace, SmfVertex};

    fn rgba(pixels: &[[u8; 4]]) -> Image {
        Image::new(
            bevy::render::render_resource::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            pixels.iter().flatten().copied().collect(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        )
    }
    fn alpha(image: &Image) -> Vec<u8> {
        image
            .data
            .as_ref()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[3])
            .collect()
    }

    #[test]
    fn color_key_removes_only_exact_black_and_uses_depth_mask() {
        let mut image = rgba(&[
            [0, 0, 0, 255],
            [1, 1, 1, 255],
            [145, 144, 145, 255],
            [0, 0, 1, 255],
        ]);
        assert_eq!(
            apply_alpha(&mut image, None, true).unwrap(),
            AlphaMode::Mask(0.5)
        );
        assert_eq!(alpha(&image), [0, 255, 255, 255]);
    }
    #[test]
    fn ordinary_black_material_stays_opaque() {
        let mut image = rgba(&[[0, 0, 0, 255]; 4]);
        assert_eq!(
            apply_alpha(&mut image, None, false).unwrap(),
            AlphaMode::Opaque
        );
        assert_eq!(alpha(&image), [255; 4]);
    }
    #[test]
    fn raw_preserves_top_to_bottom_rows_and_fractional_alpha_over_color_key() {
        let mut image = rgba(&[[0, 0, 0, 255]; 4]);
        assert_eq!(
            apply_alpha(&mut image, Some(&[0, 64, 128, 255]), true).unwrap(),
            AlphaMode::Blend
        );
        assert_eq!(alpha(&image), [0, 64, 128, 255]);
    }
    #[test]
    fn embedded_alpha_keeps_opaque_black_and_soft_edges() {
        let mut image = rgba(&[
            [0, 0, 0, 255],
            [12, 34, 56, 64],
            [0, 0, 0, 0],
            [23, 45, 67, 128],
        ]);
        assert_eq!(
            apply_alpha(&mut image, None, true).unwrap(),
            AlphaMode::Blend
        );
        assert_eq!(alpha(&image), [255, 64, 0, 128]);
    }
    #[test]
    fn wrong_raw_size_and_missing_pixels_are_reported() {
        let mut image = rgba(&[[0, 0, 0, 255]; 4]);
        assert!(
            apply_alpha(&mut image, Some(&[0; 3]), false)
                .unwrap_err()
                .to_string()
                .contains("ожидается 4")
        );
        assert_eq!(alpha(&image), [255; 4]);
        image.data = None;
        assert!(apply_alpha(&mut image, None, false).is_err());
    }

    // Синтетический BMP: нижняя строка записана первой, как в оригинальных BMP.
    fn bmp() -> Vec<u8> {
        let mut bytes = b"BM".to_vec();
        bytes.extend(70_u32.to_le_bytes());
        bytes.extend([0; 4]);
        bytes.extend(54_u32.to_le_bytes());
        bytes.extend(40_u32.to_le_bytes());
        bytes.extend(2_i32.to_le_bytes());
        bytes.extend(2_i32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(24_u16.to_le_bytes());
        bytes.extend([0; 24]);
        bytes.extend([0, 0, 255, 255, 255, 255, 0, 0]);
        bytes.extend([0, 0, 0, 0, 255, 0, 0, 0]);
        bytes
    }

    struct Sandbox(std::path::PathBuf);
    impl Sandbox {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "shadow-textures-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("Data/textures/shared")).unwrap();
            // Минимальный GUT с одним не относящимся к текстурам ресурсом.
            let mut gut = b"test\r\n\x1a\xfa\x31\xc1\xde\xed\x42\x13".to_vec();
            gut.extend(1_u32.to_le_bytes());
            gut.extend([0; 32]);
            for n in [2_u32, 1, 22, 1, 0] {
                gut.extend(n.to_le_bytes());
            }
            gut.extend([b'x' ^ 255, 255, 255]);
            std::fs::write(root.join("Data/test.gut"), gut).unwrap();
            Self(root)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn catalog_combines_decoded_bmp_with_override_raw_in_matching_rows() {
        let sandbox = Sandbox::new();
        let name = "textures/shared/test_ck.bmp";
        std::fs::write(sandbox.0.join("Data").join(name), bmp()).unwrap();
        std::fs::write(sandbox.0.join("Data/textures/shared/test_ck.raw"), [255; 4]).unwrap();
        let overrides = sandbox.0.join("overrides");
        std::fs::create_dir_all(overrides.join("textures/shared")).unwrap();
        std::fs::write(
            overrides.join("textures/shared/test_ck.raw"),
            [0, 64, 128, 255],
        )
        .unwrap();
        let catalog = AssetCatalog::open(&sandbox.0, Some(&overrides)).unwrap();
        let loaded = image(&catalog, name).unwrap();
        assert_eq!(loaded.alpha_mode, AlphaMode::Blend);
        assert_eq!(alpha(&loaded.image), [0, 64, 128, 255]);
        assert_eq!(
            loaded.image.data.as_ref().unwrap()[..8],
            [0, 0, 0, 0, 0, 255, 0, 64]
        );
        assert!(
            matches!(loaded.image.sampler, ImageSampler::Descriptor(ref s) if s.address_mode_u == ImageAddressMode::Repeat)
        );
    }

    #[test]
    fn catalog_color_key_is_explicit_and_broken_raw_is_not_silently_ignored() {
        let sandbox = Sandbox::new();
        for name in ["test.bmp", "test_ck.bmp"] {
            std::fs::write(sandbox.0.join("Data/textures/shared").join(name), bmp()).unwrap();
        }
        let catalog = AssetCatalog::open(&sandbox.0, None).unwrap();
        assert_eq!(
            alpha(&image(&catalog, "textures/shared/test.bmp").unwrap().image),
            [255; 4]
        );
        assert_eq!(
            alpha(
                &image(&catalog, "textures/shared/test_ck.bmp")
                    .unwrap()
                    .image
            ),
            [0, 255, 255, 255]
        );
        std::fs::write(sandbox.0.join("Data/textures/shared/test.raw"), [1; 3]).unwrap();
        let catalog = AssetCatalog::open(&sandbox.0, None).unwrap();
        assert!(image(&catalog, "textures/shared/test.bmp").is_err());
    }
    #[test]
    fn png_keeps_its_alpha_and_ignores_an_old_raw_and_color_key_name() {
        let sandbox = Sandbox::new();
        let png: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2,
            8, 6, 0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 26, 73, 68, 65, 84, 120, 156, 99, 96, 96, 96,
            248, 207, 163, 100, 225, 192, 196, 192, 192, 192, 200, 205, 205, 237, 0, 0, 21, 218, 2,
            10, 164, 110, 37, 80, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        std::fs::write(sandbox.0.join("Data/textures/shared/test_ck.png"), png).unwrap();
        std::fs::write(sandbox.0.join("Data/textures/shared/test_ck.raw"), [1; 3]).unwrap();
        let catalog = AssetCatalog::open(&sandbox.0, None).unwrap();
        let loaded = image(&catalog, "textures/shared/test_ck.png").unwrap();
        assert_eq!(loaded.alpha_mode, AlphaMode::Blend);
        assert_eq!(alpha(&loaded.image), [255, 64, 0, 128]);
    }

    #[test]
    fn mesh_preserves_size_and_adjusts_winding_and_normals_for_a_mirror() {
        let source = SmfMesh {
            name: "triangle".into(),
            texture: String::new(),
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                .into_iter()
                .enumerate()
                .map(|(i, p)| SmfVertex {
                    id: i as u32,
                    position: p,
                    color: 0,
                    specular: 0,
                    uv: [0.0, 0.0],
                    normal: Some([0.0, 0.0, 1.0]),
                })
                .collect(),
            faces: vec![SmfFace {
                id: 0,
                indices: [0, 2, 1],
            }],
        };
        let transform = Mat4::from_scale_rotation_translation(
            Vec3::new(-2.0, 3.0, 4.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 20.0, 30.0),
        );
        let mesh = mesh(&source, transform);
        assert_eq!(mesh.indices(), Some(&Indices::U32(vec![0, 2, 1])));
        let Some(VertexAttributeValues::Float32x3(points)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        assert_eq!(
            points,
            &[[10.0, 20.0, 30.0], [8.0, 20.0, 30.0], [10.0, 23.0, 30.0]]
        );
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("normals");
        };
        assert_eq!(normals, &[[0.0, 0.0, 1.0]; 3]);
        let triangle_normal = (Vec3::from_array(points[2]) - Vec3::from_array(points[0]))
            .cross(Vec3::from_array(points[1]) - Vec3::from_array(points[0]))
            .normalize();
        assert_eq!(triangle_normal, Vec3::Z);
    }
}
