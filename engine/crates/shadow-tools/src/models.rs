use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shadow_formats::{AssetCatalog, SmfMesh, SmfModel};
use std::error::Error;

pub fn describe(catalog: &AssetCatalog, path: &str) -> Result<Value, Box<dyn Error>> {
    let model = SmfModel::from_bytes(&catalog.read(path)?)?;
    let mut nodes = Vec::new();
    for node in &model.nodes {
        let mut meshes = Vec::new();
        for mesh in &node.meshes {
            meshes.push(json!({
                "name": mesh.name, "texture": mesh.texture,
                "resolved_texture": catalog.resolve_texture(path, &mesh.texture)?,
                "vertices": mesh.vertices.len(), "triangles": mesh.faces.len(),
                "normals_to_rebuild": mesh.vertices.iter().filter(|v| v.normal.is_none()).count(),
                "geometry_sha256": geometry_hash(mesh),
            }));
        }
        nodes.push(json!({"name": node.name, "parent": node.parent, "id": node.id,
            "translation": node.translation, "rotation": node.rotation, "flags": node.flags,
            "bounds": node.bounds.iter().map(|b| json!({"coordinates":b.coordinates,"tag":b.tag})).collect::<Vec<_>>(),
            "meshes": meshes}));
    }
    Ok(
        json!({"path":shadow_formats::normalize_path(path)?, "name":model.name,
        "version":model.version.label(),"radius":model.radius,"header_vector":model.header_vector,
        "flags":model.flags,"nodes":nodes}),
    )
}

// Отпечаток разобранной геометрии для независимой сверки, не хеш исходного SMF.
fn geometry_hash(mesh: &SmfMesh) -> String {
    let mut hash = Sha256::new();
    for vertex in &mesh.vertices {
        hash.update(vertex.id.to_le_bytes());
        for value in vertex.position {
            hash.update(value.to_le_bytes());
        }
        hash.update(vertex.color.to_le_bytes());
        hash.update(vertex.specular.to_le_bytes());
        for value in vertex.uv {
            hash.update(value.to_le_bytes());
        }
        hash.update([u8::from(vertex.normal.is_some())]);
        if let Some(normal) = vertex.normal {
            for value in normal {
                hash.update(value.to_le_bytes());
            }
        }
    }
    for face in &mesh.faces {
        hash.update(face.id.to_le_bytes());
        for value in face.indices {
            hash.update(value.to_le_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}
