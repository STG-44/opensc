//! Жёсткие части SMF присоединены к узлам: для этого персонажа skin weights не нужны.
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};
use shadow_formats::{AssetCatalog, SmfModel};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    f32::consts::FRAC_PI_2,
};

pub const WEAPON: &str = "models/alitgn-ak47j/alitgn-ak47j.smf";
pub const RIGHT_HAND: u32 = 54;
pub struct Appearance<'a> {
    pub body: &'a str,
    pub head: &'a str,
    pub body_texture: &'a str,
    pub head_texture: Option<&'a str>,
    pub pack: Option<(&'a str, &'a str)>,
}
pub struct BoneDefinition {
    pub id: u32,
    pub name: String,
    pub parent: Option<usize>,
    pub bind: Transform,
}
pub struct Part {
    pub bone: usize,
    pub name: String,
    pub mesh: Mesh,
    pub texture: String,
}
pub struct Rig {
    pub bones: Vec<BoneDefinition>,
    pub parts: Vec<Part>,
    pub images: BTreeMap<String, crate::asset_mesh::Texture>,
    pub root: Transform,
    pub focus: Vec3,
    pub vertices: usize,
    pub triangles: usize,
    pub armed: bool,
    pub weapon_first_part: Option<usize>,
    pub pack_bone_id: Option<u32>,
}
pub fn prepare_appearance(
    catalog: &AssetCatalog,
    appearance: &Appearance,
    armed: bool,
) -> Result<Rig, Box<dyn Error>> {
    let body = SmfModel::from_bytes(&catalog.read(appearance.body)?)?;
    let head = SmfModel::from_bytes(&catalog.read(appearance.head)?)?;
    let mut indices = BTreeMap::new();
    for (index, node) in body.nodes.iter().enumerate() {
        if indices.insert(node.id, index).is_some() {
            return Err("повтор ID в скелете тела".into());
        }
    }
    let bones = body
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| BoneDefinition {
            id: n.id,
            name: n.name.clone(),
            parent: body.parents[i],
            bind: crate::smf_transform::rest(n, body.parents[i].is_none()),
        })
        .collect::<Vec<_>>();
    let basis = Mat4::from_rotation_x(-FRAC_PI_2);
    let mut globals = vec![Mat4::IDENTITY; bones.len()];
    for &i in &body.node_order {
        globals[i] =
            bones[i].parent.map(|p| globals[p]).unwrap_or(basis) * bones[i].bind.to_matrix();
    }
    let mut lower = Vec3::splat(f32::INFINITY);
    let mut upper = Vec3::splat(f32::NEG_INFINITY);
    let mut rig = Rig {
        bones,
        parts: Vec::new(),
        images: BTreeMap::new(),
        root: Transform::default(),
        focus: Vec3::ZERO,
        vertices: 0,
        triangles: 0,
        armed,
        weapon_first_part: None,
        pack_bone_id: None,
    };
    for (path, model, texture_override) in [
        (appearance.body, &body, Some(appearance.body_texture)),
        (appearance.head, &head, appearance.head_texture),
    ] {
        if model.header_vector != body.header_vector {
            return Err("поля заголовков тела и головы отличаются".into());
        }
        // Пустые вспомогательные узлы головы не обязаны совпадать с телом.
        // Узлы с геометрией и вся цепочка их родителей проверяются строго.
        let required = geometry_chain(
            &model.parents,
            model
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node.meshes.iter().any(|mesh| !mesh.faces.is_empty()))
                .map(|(i, _)| i),
        );
        for (index, node) in model.nodes.iter().enumerate() {
            if !required.contains(&index) {
                continue;
            }
            let &bone = indices
                .get(&node.id)
                .ok_or("узел головы отсутствует в теле")?;
            let expected = &body.nodes[bone];
            if node.name != expected.name
                || model.parents[index].map(|p| model.nodes[p].id)
                    != body.parents[bone].map(|p| body.nodes[p].id)
                || node.translation != expected.translation
                || node.rotation != expected.rotation
            {
                return Err(format!("скелеты головы и тела отличаются: {}", node.name).into());
            }
            for source in &node.meshes {
                if source.faces.is_empty() {
                    continue;
                }
                let positions = source
                    .vertices
                    .iter()
                    .map(|v| v.position)
                    .collect::<Vec<_>>();
                for p in &positions {
                    let point = globals[bone].transform_point3(Vec3::from_array(*p));
                    lower = lower.min(point);
                    upper = upper.max(point);
                }
                let faces = source
                    .faces
                    .iter()
                    .flat_map(|f| [f.indices[0], f.indices[2], f.indices[1]])
                    .collect::<Vec<_>>();
                let fallback = crate::normals::reconstructed_normals(&positions, &faces);
                let normals = source
                    .vertices
                    .iter()
                    .zip(fallback)
                    .map(|(v, f)| {
                        v.normal
                            .and_then(|n| Vec3::from_array(n).try_normalize())
                            .map(|n| n.to_array())
                            .unwrap_or(f)
                    })
                    .collect::<Vec<_>>();
                let mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                )
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
                .with_inserted_attribute(
                    Mesh::ATTRIBUTE_UV_0,
                    source.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
                )
                .with_inserted_indices(Indices::U32(faces));
                let texture = catalog
                    .resolve_texture(path, texture_override.unwrap_or(&source.texture))?
                    .ok_or("текстура персонажа не найдена")?;
                if !rig.images.contains_key(&texture) {
                    let image = crate::asset_mesh::image(catalog, &texture)?;
                    rig.images.insert(texture.clone(), image);
                }
                rig.vertices += source.vertices.len();
                rig.triangles += source.faces.len();
                rig.parts.push(Part {
                    bone,
                    name: source.name.clone(),
                    mesh,
                    texture,
                });
            }
        }
    }
    let extent = (upper - lower).max_element();
    if !extent.is_finite() || extent <= f32::EPSILON {
        return Err("размер персонажа недопустим".into());
    }
    let scale = 3.0 / extent;
    let center = Vec3::new(
        (lower.x + upper.x) * 0.5,
        lower.y,
        (lower.z + upper.z) * 0.5,
    );
    rig.root = Transform::from_translation(-center * scale)
        .with_rotation(Quat::from_rotation_x(-FRAC_PI_2))
        .with_scale(Vec3::splat(scale));
    rig.focus = Vec3::new(0.0, (upper.y - lower.y) * scale * 0.5, 0.0);
    if let Some((path, texture)) = appearance.pack {
        let mut matches = rig
            .bones
            .iter()
            .filter(|b| b.name.eq_ignore_ascii_case("Backpack"));
        let bone_id = matches.next().ok_or("нет узла Backpack для рюкзака")?.id;
        if matches.next().is_some() {
            return Err("неоднозначный узел Backpack".into());
        }
        rig.pack_bone_id = Some(bone_id);
        attach_accessory(catalog, &mut rig, bone_id, path, Some(texture), "Backpack")?;
    }
    if armed {
        attach_weapon(catalog, &mut rig)?;
    }
    Ok(rig)
}

fn attach_weapon(catalog: &AssetCatalog, rig: &mut Rig) -> Result<(), Box<dyn Error>> {
    prepare_weapon(catalog, rig, WEAPON, "AK-47")
}

/// Подготовка в загрузчике. Видимость экипировки переключается отдельно в сцене.
pub fn prepare_weapon(
    catalog: &AssetCatalog,
    rig: &mut Rig,
    path: &str,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    rig.weapon_first_part = Some(rig.parts.len());
    attach_accessory(catalog, rig, RIGHT_HAND, path, None, label)
}

fn attach_accessory(
    catalog: &AssetCatalog,
    rig: &mut Rig,
    bone_id: u32,
    path: &str,
    texture_override: Option<&str>,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    let bone = rig
        .bones
        .iter()
        .position(|b| b.id == bone_id)
        .ok_or("нет узла крепления аксессуара")?;
    let model = SmfModel::from_bytes(&catalog.read(path)?)?;
    let mut globals = vec![Mat4::IDENTITY; model.nodes.len()];
    for &i in &model.node_order {
        let n = &model.nodes[i];
        globals[i] = model.parents[i]
            .map(|p| globals[p])
            .unwrap_or(Mat4::IDENTITY)
            * crate::smf_transform::rest(n, model.parents[i].is_none()).to_matrix();
    }
    for (i, node) in model.nodes.iter().enumerate() {
        for source in &node.meshes {
            if source.faces.is_empty() {
                continue;
            }
            let texture = catalog
                .resolve_texture(path, texture_override.unwrap_or(&source.texture))?
                .ok_or("текстура аксессуара не найдена")?;
            if !rig.images.contains_key(&texture) {
                rig.images.insert(
                    texture.clone(),
                    crate::asset_mesh::image(catalog, &texture)?,
                );
            }
            rig.vertices += source.vertices.len();
            rig.triangles += source.faces.len();
            rig.parts.push(Part {
                bone,
                name: format!("{label} / {}", source.name),
                mesh: crate::asset_mesh::mesh(source, globals[i]),
                texture,
            });
        }
    }
    Ok(())
}

/// Числовая проверка крепления без окна; используем ту же Mesh и иерархию, что в Bevy.
#[allow(dead_code)] // Интерфейс отдельной витрины персонажа.
pub fn weapon_world_vertices(
    rig: &Rig,
    pose: &[Transform],
    world_root: Transform,
) -> Vec<[f32; 3]> {
    let Some(first) = rig.weapon_first_part else {
        return Vec::new();
    };
    accessory_world_vertices(rig, pose, world_root, first..rig.parts.len())
}
pub fn accessory_world_vertices(
    rig: &Rig,
    pose: &[Transform],
    world_root: Transform,
    range: std::ops::Range<usize>,
) -> Vec<[f32; 3]> {
    use bevy::mesh::VertexAttributeValues;
    let mut result = Vec::new();
    for part in &rig.parts[range] {
        let mut transform = pose[part.bone].to_matrix();
        let mut parent = rig.bones[part.bone].parent;
        while let Some(index) = parent {
            transform = pose[index].to_matrix() * transform;
            parent = rig.bones[index].parent;
        }
        transform = world_root.to_matrix() * transform;
        if let Some(VertexAttributeValues::Float32x3(points)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        {
            result.extend(
                points
                    .iter()
                    .map(|p| transform.transform_point3(Vec3::from_array(*p)).to_array()),
            );
        }
    }
    result
}

fn geometry_chain(
    parents: &[Option<usize>],
    nodes: impl Iterator<Item = usize>,
) -> BTreeSet<usize> {
    let mut required = BTreeSet::new();
    for node in nodes {
        let mut current = Some(node);
        while let Some(index) = current {
            if !required.insert(index) {
                break;
            }
            current = parents[index];
        }
    }
    required
}
#[cfg(test)]
mod appearance_tests {
    use super::*;
    #[test]
    fn geometry_chain_keeps_empty_ancestors_but_excludes_unused_accessory_nodes() {
        let parents = [None, Some(0), Some(1), Some(0), Some(3)];
        assert_eq!(
            geometry_chain(&parents, [2].into_iter()),
            BTreeSet::from([0, 1, 2])
        );
        assert_eq!(
            geometry_chain(&parents, [2, 4].into_iter()),
            BTreeSet::from([0, 1, 2, 3, 4])
        );
    }
}
