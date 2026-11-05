//! Перевод исходных координат SMF в геометрию Bevy для статического просмотра.
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, SmfModel};
use std::{collections::BTreeMap, error::Error, f32::consts::FRAC_PI_2};
mod visual_pack;
mod visual_surface;
mod wall_caps;

#[allow(dead_code)] // Shared scene module also builds standalone model browsers.
pub(crate) fn open_game_catalog(
    game: &std::path::Path,
    overrides: Option<&std::path::Path>,
) -> shadow_formats::Result<AssetCatalog> {
    let executable = std::env::current_exe()?;
    let bundled = executable.parent().map(|p| p.join("environment-pack"));
    let bundled = bundled.as_deref().filter(|p| p.is_dir());
    AssetCatalog::open_layered(game, bundled, overrides)
}

#[derive(Clone)]
// The model browser preserves bindings but renders the parked pose.
#[allow(dead_code)]
pub struct WiperBinding {
    pub part: String,
    pub pivot: Vec3,
    pub axis: Vec3,
    pub sweep: f32,
}
#[derive(Clone, Copy)]
pub enum LampRole {
    Head,
    Tail,
}
#[derive(Clone)]
pub struct PreparedPart {
    pub name: String,
    pub mesh: Mesh,
    pub texture: Option<String>,
    pub alpha_mode: AlphaMode,
    pub glass: bool,
    pub wiper: Option<WiperBinding>,
    pub lamp: Option<LampRole>,
}

impl PreparedPart {
    pub fn surface(&self) -> StandardMaterial {
        let (roughness, metallic) = if self.lamp.is_some() {
            (0.25, 0.05)
        } else if self.glass {
            (0.16, 0.0)
        } else {
            match self.texture.as_deref() {
                Some(
                    "textures/opensc-environment/steel.bmp"
                    | "textures/opensc-environment/galvanized.png",
                ) => (0.6, 0.72),
                Some("textures/opensc-environment/camouflage.png") => (0.78, 0.15),
                Some("textures/opensc-environment/hummer-body-v1.png") => (0.78, 0.05),
                Some("textures/opensc-environment/hummer-trim.bmp") => (0.68, 0.4),
                Some("textures/opensc-environment/hummer-wheels.bmp") => (0.9, 0.05),
                Some("textures/opensc-environment/hummer-glass.bmp") => (0.15, 0.1),
                Some(
                    "textures/opensc-environment/hummer-lamp.bmp"
                    | "textures/opensc-environment/hummer-amber.bmp"
                    | "textures/opensc-environment/hummer-red.bmp",
                ) => (0.25, 0.05),
                _ => (0.95, 0.0),
            }
        };
        StandardMaterial {
            alpha_mode: self.alpha_mode,
            cull_mode: None,
            double_sided: self.texture.as_deref() == Some("textures/opensc-environment/leaves.png"),
            perceptual_roughness: roughness,
            metallic,
            // Uniform ambient lighting has no directional environment map.
            // Limit its white specular lobe so tinted glass keeps its albedo.
            reflectance: if self.glass || self.lamp.is_some() {
                0.12
            } else {
                0.5
            },
            ..default()
        }
    }
}

pub struct PreparedModel {
    pub name: String,
    pub version: String,
    pub parts: Vec<PreparedPart>,
    interaction_parts: Option<Vec<PreparedPart>>,
    pub images: BTreeMap<String, crate::asset_mesh::Texture>,
    pub triangles: usize,
    pub vertices: usize,
    pub repaired_normals: usize,
    pub notes: Vec<String>,
    pub focus: Vec3,
}

impl PreparedModel {
    // The separate model browser draws meshes without an interaction system.
    #[allow(dead_code)]
    pub fn interaction_parts(&self) -> &[PreparedPart] {
        self.interaction_parts.as_deref().unwrap_or(&self.parts)
    }
}

/// `normalize = false` сохраняет размеры и начало модели для карты.
pub fn prepare(
    catalog: &AssetCatalog,
    path: &str,
    normalize: bool,
) -> Result<PreparedModel, Box<dyn Error>> {
    let bytes = catalog.read(path)?;
    let proxy = visual_pack::base_bytes(catalog, path, &bytes)?;
    let mut model = prepare_bytes(catalog, path, false, &bytes)?;
    if let Some(base) = proxy {
        for binding in &base.wipers {
            let found = model
                .parts
                .iter_mut()
                .filter(|p| p.name == binding.part)
                .collect::<Vec<_>>();
            let [part] = found.as_slice() else {
                return Err("Wiper binding must match one replacement part".into());
            };
            let name = part.name.clone();
            model
                .parts
                .iter_mut()
                .find(|p| p.name == name)
                .unwrap()
                .wiper = Some(binding.clone());
        }
        let original = prepare_bytes(catalog, path, false, &base.bytes)?;
        let mut used_textures = std::collections::BTreeSet::new();
        for name in &base.render_parts {
            let selected = original
                .parts
                .iter()
                .filter(|p| &p.name == name)
                .collect::<Vec<_>>();
            let [part] = selected.as_slice() else {
                return Err(format!("Retained model part is not unique: {name}").into());
            };
            let mut part = (**part).clone();
            let rule = part
                .texture
                .as_ref()
                .and_then(|name| base.textures.get(name));
            let parts = if let Some(rule) = rule {
                used_textures.insert(part.texture.clone().unwrap());
                if !model.images.contains_key(&rule.texture) {
                    model.images.insert(
                        rule.texture.clone(),
                        crate::asset_mesh::image(catalog, &rule.texture)?,
                    );
                }
                part.texture = Some(rule.texture.clone());
                part.alpha_mode = model.images[&rule.texture].alpha_mode;
                visual_surface::readable_hummer_stencil(&mut part)?;
                visual_surface::split_glass(part, &rule.glass_regions)?
            } else {
                vec![part]
            };
            for part in parts {
                model.vertices += part.mesh.count_vertices();
                model.triangles += part.mesh.indices().map_or(0, |i| i.len() / 3);
                model.parts.push(part);
            }
        }
        if used_textures.len() != base.textures.len() {
            return Err("Texture remapping does not match a retained model part".into());
        }
        if !base.render_parts.is_empty() {
            model.images.extend(original.images);
            model.notes.push(
                "Original body silhouette retained; attached details are replacements".into(),
            );
            if !used_textures.is_empty() {
                model.notes.push(
                    "Retained body uses repainted textures and separate opaque glass surfaces"
                        .into(),
                );
            }
        }
        model.interaction_parts = Some(original.parts);
        model
            .notes
            .push("Visual replacement uses installed collision and interaction surfaces".into());
    }
    if normalize {
        normalize_model(&mut model)?;
    }
    Ok(model)
}

fn normalize_model(model: &mut PreparedModel) -> Result<(), Box<dyn Error>> {
    use bevy::mesh::VertexAttributeValues;
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for part in &model.parts {
        if let Some(VertexAttributeValues::Float32x3(points)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        {
            for p in points {
                let p = Vec3::from_array(*p);
                low = low.min(p);
                high = high.max(p);
            }
        }
    }
    let extent = (high - low).max_element();
    if !extent.is_finite() || extent <= f32::EPSILON {
        return Err("в модели нет геометрии ненулевого размера".into());
    }
    let scale = 3. / extent;
    let mid = low + (high - low) * 0.5;
    let center = Vec3::new(mid.x, low.y, mid.z);
    for part in model
        .parts
        .iter_mut()
        .chain(model.interaction_parts.iter_mut().flatten())
    {
        if let Some(wiper) = &mut part.wiper {
            wiper.pivot = (wiper.pivot - center) * scale;
        }
        if let Some(VertexAttributeValues::Float32x3(points)) =
            part.mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        {
            for p in points {
                *p = ((Vec3::from_array(*p) - center) * scale).to_array();
            }
        }
    }
    model.focus = Vec3::new(0., (high.y - low.y) * scale * 0.5, 0.);
    Ok(())
}

fn prepare_bytes(
    catalog: &AssetCatalog,
    path: &str,
    normalize: bool,
    bytes: &[u8],
) -> Result<PreparedModel, Box<dyn Error>> {
    let mut model = SmfModel::from_bytes(bytes)?;
    let closed_wall_end = wall_caps::repair(path, bytes, &mut model);
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
            interaction_parts: None,
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
        interaction_parts: None,
        images: BTreeMap::new(),
        triangles: 0,
        vertices: 0,
        repaired_normals: 0,
        notes: Vec::new(),
        focus: Vec3::new(0.0, (upper.y - lower.y) * scale * 0.5, 0.0),
    };
    if closed_wall_end {
        result.notes.push(
            "Closed the original concrete wall's open end with four generated triangles".into(),
        );
    }
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
                    Ok(mut image) => {
                        // Leaf cards represent solid leaves with empty space
                        // between them. Write depth for the covered pixels so
                        // overlapping cards sort and cast cutout shadows.
                        if ((path.starts_with("models/alsctr-south")
                            && name == "textures/shared/southtreebranch1.bmp")
                            || name == "textures/opensc-environment/leaves.png")
                            && image.alpha_mode == AlphaMode::Blend
                        {
                            image.alpha_mode = AlphaMode::Mask(0.5);
                            result
                                .notes
                                .push("Leaf cards use depth-writing alpha cutouts".into());
                        }
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
            glass: false,
            wiper: None,
            lamp: match source.name.as_str() {
                "OpenSC Hummer headlamps" => Some(LampRole::Head),
                "OpenSC Hummer tail lamps" => Some(LampRole::Tail),
                _ => None,
            },
        });
    }
    if result.parts.is_empty() {
        return Err("в модели нет треугольников".into());
    }
    result.notes.sort();
    result.notes.dedup();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    fn part(name: &str, x: f32) -> PreparedPart {
        let mut mesh = Mesh::from(Cuboid::new(2., 2., 2.));
        if let Some(VertexAttributeValues::Float32x3(points)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        {
            for p in points {
                p[0] += x;
            }
        }
        PreparedPart {
            name: name.into(),
            mesh,
            texture: None,
            alpha_mode: AlphaMode::Opaque,
            glass: false,
            wiper: None,
            lamp: None,
        }
    }
    #[test]
    fn retained_body_and_attached_details_share_one_viewer_normalization() {
        let body = part("body", 0.);
        let mut model = PreparedModel {
            name: "overlay".into(),
            version: "test".into(),
            interaction_parts: Some(vec![body.clone()]),
            parts: vec![body, part("detail", 5.)],
            images: BTreeMap::new(),
            triangles: 24,
            vertices: 48,
            repaired_normals: 0,
            notes: Vec::new(),
            focus: Vec3::ZERO,
        };
        normalize_model(&mut model).unwrap();
        let positions = |p: &PreparedPart| match p.mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        {
            VertexAttributeValues::Float32x3(v) => v.clone(),
            _ => unreachable!(),
        };
        assert_eq!(
            positions(&model.parts[0]),
            positions(&model.interaction_parts()[0])
        );
        let all = model
            .parts
            .iter()
            .flat_map(positions)
            .map(Vec3::from_array)
            .collect::<Vec<_>>();
        let low = all
            .iter()
            .copied()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min);
        let high = all
            .iter()
            .copied()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
        assert!(((high - low).max_element() - 3.).abs() < 1e-5);
    }
}
