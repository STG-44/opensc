use crate::scene::PreparedModel;
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};
use shadow_formats::{AssetCatalog, MtfMap, PcxImage, TerrainConfig, TerrainGrid, config_lines};
use std::{collections::BTreeMap, error::Error, f32::consts::FRAC_PI_2};

pub struct Instance {
    pub id: [i32; 2],
    pub inventory: bool,
    pub path: String,
    pub name: String,
    pub transform: Transform,
}
pub struct MapScene {
    pub models: BTreeMap<String, PreparedModel>,
    pub instances: Vec<Instance>,
    pub terrain: Mesh,
    pub texture: Image,
    pub grid: TerrainGrid,
    pub daylight: crate::daylight::Daylight,
    pub actors: Vec<shadow_formats::MtfObject>,
    pub focus: Vec3,
    pub span: f32,
    pub report: serde_json::Value,
}
pub const WORLD_SCALE: f32 = 0.01;
pub fn prepare(
    catalog: &AssetCatalog,
    path: &str,
    terrain_name: &str,
) -> Result<MapScene, Box<dyn Error>> {
    let map = MtfMap::from_bytes(&catalog.read(path)?)?;
    let daylight = crate::daylight::Daylight::prepare(catalog, path, &map.globals)?;
    let mapping = format!("textures/terrain/{terrain_name}/terrain_mapping.txt");
    let config = TerrainConfig::from_bytes(&catalog.read(&mapping)?)?;
    let height_path = format!("maps/{}.pcx", config.name);
    let texture_path = format!("trnhigh/{}.jpg", config.name);
    let grid = TerrainGrid::new(config, PcxImage::from_bytes(&catalog.read(&height_path)?)?)?;
    let mut texture = crate::asset_mesh::image(catalog, &texture_path)?.image;
    texture.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        ..bevy::image::ImageSamplerDescriptor::linear()
    });
    let terrain = terrain_mesh(&grid);
    // Индекс строится по basename, чтобы читать модели по ссылкам MTF и профилей LOD.
    let mut model_index: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in catalog.paths().filter(|p| p.ends_with(".smf")) {
        let name = p.rsplit('/').next().unwrap().trim_end_matches(".smf");
        model_index
            .entry(name.to_owned())
            .or_default()
            .push(p.to_owned());
    }
    let mut lod = BTreeMap::new();
    for p in catalog
        .paths()
        .filter(|p| p.starts_with("config/lod_model_profiles/") && p.ends_with(".txt"))
    {
        let lines = config_lines(&catalog.read(p)?)?;
        let name = lines
            .iter()
            .find(|l| l.words[0] == "LOD_MODEL_NAME")
            .and_then(|l| l.words.get(1));
        let base = lines
            .iter()
            .find(|l| l.words[0] == "SUB_MODEL_MODEL")
            .and_then(|l| l.words.get(1));
        if let (Some(name), Some(base)) = (name, base) {
            lod.insert(name.to_lowercase(), base.to_lowercase());
        }
    }
    let mut models = BTreeMap::new();
    let mut instances = Vec::new();
    let mut unresolved = Vec::new();
    let mut soldiers = 0;
    let mut sample_matches = 0;
    let mut sample_count = 0;
    let basis = Quat::from_rotation_x(-FRAC_PI_2);
    for o in &map.objects {
        let (Some(position), Some(angles)) = (o.position, o.rotation) else {
            unresolved.push(format!(
                "{}: исходное положение или поворот неопределён",
                o.name
            ));
            continue;
        };
        if let Some(h) = grid.height_at(position[0], position[1]) {
            sample_count += 1;
            if (h - position[2]).abs() < 0.02 {
                sample_matches += 1;
            }
        }
        if o.class.eq_ignore_ascii_case("Bipedal") {
            soldiers += 1;
            continue;
        }
        let name = o.model.to_lowercase();
        let name = lod.get(&name).unwrap_or(&name);
        let Some(candidates) = model_index.get(name) else {
            unresolved.push(format!("{}: {}", o.name, o.model));
            continue;
        };
        let [p] = candidates.as_slice() else {
            unresolved.push(format!("{}: неоднозначная модель {}", o.name, o.model));
            continue;
        };
        if !models.contains_key(p) {
            let prepared =
                crate::scene::prepare(catalog, p, false).map_err(|e| format!("{p}: {e}"))?;
            if prepared.parts.is_empty() {
                unresolved.push(format!("{}: в исходном {} нет треугольников", o.name, p));
                continue;
            }
            models.insert(p.clone(), prepared);
        }
        let [x, y, z] = angles;
        let rotation = basis * Quat::from_euler(EulerRot::ZYX, z, y, x) * basis.inverse();
        instances.push(Instance {
            id: o.id,
            inventory: o.inventory,
            path: p.clone(),
            name: o.name.clone(),
            transform: Transform::from_translation(basis * Vec3::from_array(position))
                .with_rotation(rotation),
        });
    }
    let c = &grid.config;
    let min_height = grid.heights.iter().copied().fold(f32::INFINITY, f32::min);
    let max_height = grid
        .heights
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let span = (c.width.max(c.height) as f32) * c.edge * WORLD_SCALE;
    let focus = Vec3::new(
        c.width as f32 * c.edge * 0.5,
        (min_height + max_height) * 0.5,
        -(c.height as f32) * c.edge * 0.5,
    ) * WORLD_SCALE;
    let mesh_info = models.iter().map(|(p,m)|serde_json::json!({"path":p,"name":m.name,"version":m.version,"vertices":m.vertices,"triangles":m.triangles,"parts":m.parts.len(),"transparent_parts":m.parts.iter().filter(|p| p.alpha_mode != AlphaMode::Opaque).count(),"repaired_normals":m.repaired_normals,"focus":m.focus.to_array(),"notes":m.notes,"textures":m.images.iter().map(|(name,t)| {
        let pixels = t.image.data.as_deref().unwrap_or(&[]);
        serde_json::json!({"path":name,"alpha_mode":format!("{:?}", t.alpha_mode),
            "size":[t.image.texture_descriptor.size.width,t.image.texture_descriptor.size.height],
            "transparent_pixels":pixels.as_chunks::<4>().0.iter().filter(|p| p[3] == 0).count(),
            "fractional_pixels":pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 0 && p[3] < 255).count(),
            "opaque_pixels":pixels.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count()})
    }).collect::<Vec<_>>() })).collect::<Vec<_>>();
    let report = serde_json::json!({"daylight":daylight.source.clone(),"map":path,"objects_total":map.objects.len(),"static_instances":instances.len(),"soldiers_deferred":soldiers,"unresolved_objects":unresolved,"models":mesh_info,
        "instances":instances.iter().map(|o|serde_json::json!({"name":o.name,"path":o.path,"translation":o.transform.translation.to_array(),"rotation_xyzw":o.transform.rotation.to_array()})).collect::<Vec<_>>(),
        "terrain":{"mapping":mapping,"heightmap":height_path,"texture":texture_path,"samples":[c.width,c.height],"vertices":(c.width+3)*(c.height+3),"triangles":(c.width+2)*(c.height+2)*2,"edge_source":c.edge,"extent_source":grid.extent(),"visual_border_cells":1,"boundary_reconstruction":"clamped outer height samples and render-only border; original EXE boundary sampling unverified","height_step":c.height_step,"height_range_source":[min_height,max_height],"water_index":c.water_index,"object_positions_at_surface":sample_matches,"object_positions_sampled":sample_count},
        "world_scale":WORLD_SCALE,"mission_logic":false,"water_rendered":false,"native_lod_switching":false});
    Ok(MapScene {
        models,
        instances,
        terrain,
        texture,
        grid,
        daylight,
        actors: map
            .objects
            .into_iter()
            .filter(|o| o.class.eq_ignore_ascii_case("Bipedal"))
            .collect(),
        focus,
        span,
        report,
    })
}
fn terrain_mesh(grid: &TerrainGrid) -> Mesh {
    let c = &grid.config;
    // Edge pillars protrude beyond the nominal atlas extent. A render-only
    // cell supports these details without enlarging the playable map.
    let stride = c.width + 3;
    let mut positions = Vec::with_capacity(stride * (c.height + 3));
    let mut uv = Vec::with_capacity(positions.capacity());
    for y in -1..=c.height as i32 + 1 {
        for x in -1..=c.width as i32 + 1 {
            positions.push([
                x as f32 * c.edge,
                grid.sample(x.max(0) as usize, y.max(0) as usize),
                -(y as f32) * c.edge,
            ]);
            // Clamp-to-edge sampling extends the atlas across the visual border.
            uv.push([x as f32 / c.width as f32, y as f32 / c.height as f32]);
        }
    }
    let mut indices = Vec::new();
    for y in 0..c.height + 2 {
        for x in 0..c.width + 2 {
            let a = (y * stride + x) as u32;
            let b = a + 1;
            let d = a + stride as u32 + 1;
            let e = a + stride as u32;
            indices.extend([a, b, d, a, d, e]);
        }
    }
    let normals = crate::normals::reconstructed_normals(&positions, &indices);
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    #[test]
    fn terrain_mesh_uses_source_units_atlas_uvs_and_upward_triangles() {
        let grid = TerrainGrid {
            config: TerrainConfig {
                name: "test".into(),
                width: 2,
                height: 2,
                edge: 200.0,
                height_step: 10.0,
                water_index: 0,
            },
            heights: vec![0.0, 0.0, 0.0, 100.0],
        };
        let mesh = terrain_mesh(&grid);
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("indices");
        };
        assert_eq!(indices.len(), 96);
        assert_eq!(&indices[..6], &[0, 1, 6, 0, 6, 5]);
        assert_eq!(&indices[90..], &[18, 19, 24, 18, 24, 23]);
        let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        assert_eq!(p[12], [200.0, 100.0, -200.0]);
        assert_eq!(p[18], [400.0, 100.0, -400.0]);
        assert_eq!(p[24], [600.0, 100.0, -600.0]);
        assert!(grid.height_at(600.0, 600.0).is_none()); // visual border only
        let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("uv");
        };
        assert_eq!(uv[12], [0.5, 0.5]);
        assert_eq!(uv[18], [1.0, 1.0]);
        assert_eq!(uv[24], [1.5, 1.5]);
        let Some(VertexAttributeValues::Float32x3(n)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("normals");
        };
        assert!(n.iter().all(|v| v[1] > 0.0));
    }
}
