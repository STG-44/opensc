//! Перевод исходных координат SMF в геометрию Bevy для статического просмотра.
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, SmfModel};
use std::{collections::BTreeMap, error::Error, f32::consts::FRAC_PI_2};

pub struct PreparedPart {
    pub name: String,
    pub mesh: Mesh,
    pub texture: Option<String>,
    pub alpha_mode: AlphaMode,
}

pub struct PreparedModel {
    pub name: String,
    pub version: String,
    pub parts: Vec<PreparedPart>,
    pub images: BTreeMap<String, crate::asset_mesh::Texture>,
    pub triangles: usize,
    pub vertices: usize,
    pub repaired_normals: usize,
    pub notes: Vec<String>,
    pub focus: Vec3,
}

/// `normalize = false` сохраняет размеры и начало модели для карты.
pub fn prepare(
    catalog: &AssetCatalog,
    path: &str,
    normalize: bool,
) -> Result<PreparedModel, Box<dyn Error>> {
    let model = SmfModel::from_bytes(&catalog.read(path)?)?;
    // На карте есть исходные невидимые предметы без треугольников.
    if !normalize
        && !model
            .nodes
            .iter()
            .any(|n| n.meshes.iter().any(|m| !m.faces.is_empty()))
    {
        return Ok(PreparedModel {
            name: model.name,
            version: model.version.label().into(),
            vertices: model
                .nodes
                .iter()
                .flat_map(|n| &n.meshes)
                .map(|m| m.vertices.len())
                .sum(),
            triangles: 0,
            repaired_normals: 0,
            parts: Vec::new(),
            images: BTreeMap::new(),
            notes: vec!["В исходном SMF нет треугольников".into()],
            focus: Vec3::ZERO,
        });
    }
    // SMF: Z вверх. Bevy: Y вверх. Перестановка (x, z, -y) сохраняет ориентацию.
    let basis = Mat4::from_quat(Quat::from_rotation_x(-FRAC_PI_2));
    let mut transforms = vec![Mat4::IDENTITY; model.nodes.len()];
    for &index in &model.node_order {
        let node = &model.nodes[index];
        let local = crate::smf_transform::rest(node, model.parents[index].is_none()).to_matrix();
        transforms[index] = model.parents[index]
            .map(|parent| transforms[parent])
            .unwrap_or(basis)
            * local;
    }
    let mut lower = Vec3::splat(f32::INFINITY);
    let mut upper = Vec3::splat(f32::NEG_INFINITY);
    let mut positions = Vec::new();
    for (index, node) in model.nodes.iter().enumerate() {
        for mesh in &node.meshes {
            let points: Vec<_> = mesh
                .vertices
                .iter()
                .map(|v| transforms[index].transform_point3(Vec3::from_array(v.position)))
                .collect();
            for &point in &points {
                if !point.is_finite() {
                    return Err("SMF: преобразование координат вышло за пределы f32".into());
                }
                lower = lower.min(point);
                upper = upper.max(point);
            }
            positions.push((index, mesh, points));
        }
    }
    let extent = (upper - lower).max_element();
    if !extent.is_finite() || extent <= f32::EPSILON {
        return Err("в модели нет геометрии ненулевого размера".into());
    }
    // Масштабирование только для витрины: самая большая сторона = 3 единицы.
    let scale = if normalize { 3.0 / extent } else { 1.0 };
    let midpoint = lower + (upper - lower) * 0.5;
    let center = if normalize {
        Vec3::new(midpoint.x, lower.y, midpoint.z)
    } else {
        Vec3::ZERO
    };
    let mut result = PreparedModel {
        name: model.name,
        version: model.version.label().into(),
        parts: Vec::new(),
        images: BTreeMap::new(),
        triangles: 0,
        vertices: 0,
        repaired_normals: 0,
        notes: Vec::new(),
        focus: Vec3::new(0.0, (upper.y - lower.y) * scale * 0.5, 0.0),
    };
    for (index, source, _) in positions {
        if source.faces.is_empty() {
            continue;
        }
        let normal_matrix = transforms[index].inverse().transpose();
        result.repaired_normals += source
            .vertices
            .iter()
            .filter(|v| {
                v.normal
                    .map(|n| normal_matrix.transform_vector3(Vec3::from_array(n)))
                    .is_none_or(|n| !n.is_finite() || n.length_squared() <= f32::EPSILON)
            })
            .count();
        let mesh = crate::asset_mesh::mesh(
            source,
            Mat4::from_scale(Vec3::splat(scale))
                * Mat4::from_translation(-center)
                * transforms[index],
        );
        let resolved = catalog.resolve_texture(path, &source.texture)?;
        let mut texture = None;
        if let Some(name) = resolved {
            if !result.images.contains_key(&name) {
                let decoded = crate::asset_mesh::image(catalog, &name);
                match decoded {
                    Ok(image) => {
                        result.images.insert(name.clone(), image);
                    }
                    Err(error) => result.notes.push(format!("Текстура {name}: {error}")),
                }
            }
            if result.images.contains_key(&name) {
                texture = Some(name);
            }
        } else if !source.texture.is_empty() {
            result
                .notes
                .push(format!("Текстура не найдена: {}", source.texture));
        }
        result.triangles += source.faces.len();
        result.vertices += source.vertices.len();
        result.parts.push(PreparedPart {
            name: source.name.clone(),
            mesh,
            alpha_mode: texture
                .as_ref()
                .map(|name| result.images[name].alpha_mode)
                .unwrap_or(AlphaMode::Opaque),
            texture,
        });
    }
    if result.parts.is_empty() {
        return Err("в модели нет треугольников".into());
    }
    result.notes.sort();
    result.notes.dedup();
    Ok(result)
}
