//! Close the two open end panels in the three original concrete wall sections.
//! Fingerprints keep authored replacements and unrelated open meshes untouched.
use sha2::{Digest, Sha256};
use shadow_formats::{SmfFace, SmfMesh, SmfModel, SmfVertex};

pub(super) fn repair(path: &str, bytes: &[u8], model: &mut SmfModel) -> bool {
    let expected = match path.to_ascii_lowercase().as_str() {
        "models/alstwa-concrete01/alstwa-concrete01.smf" => {
            "9dae28649280c30e30b7e579423e8ea37c8ff741357c95e705cbd73a1ed5b687"
        }
        "models/alstwa-concrete02/alstwa-concrete02.smf" => {
            "03b6b2954f4ef8707a296211e2e929fc98f73ad6427a1702f3b9789c9fe281a5"
        }
        "models/alstwa-concrete03/alstwa-concrete03.smf" => {
            "f0dc4b96b6490c02b201069f761a93930379cec68cc2dbff0f913a7bfea5fc02"
        }
        _ => return false,
    };
    if format!("{:x}", Sha256::digest(bytes)) != expected {
        return false;
    }
    model
        .nodes
        .iter_mut()
        .flat_map(|n| &mut n.meshes)
        .any(close_end)
}

fn close_end(mesh: &mut SmfMesh) -> bool {
    if mesh.name != "Box02" || mesh.vertices.len() < 38 {
        return false;
    }
    // Existing body and coping corners define the end profile. Do not bridge
    // the map's doorway or join neighboring instances: each cap stays on its
    // own section's existing end plane, before the ordinary SMF transform.
    let wall_length = mesh.vertices[9].position[0] - mesh.vertices[8].position[0];
    let uv_per_unit = (mesh.vertices[9].uv[0] - mesh.vertices[8].uv[0]) / wall_length;
    let uv_origin = mesh.vertices[8].uv[0];
    let coping = if mesh.vertices.len() == 64 {
        [58, 57, 62, 61]
    } else {
        [32, 31, 36, 35]
    };
    for corners in [[9, 11, 15, 13], coping] {
        let low_y = mesh.vertices[corners[0]].position[1];
        let vertices = corners.map(|i| {
            let v = &mesh.vertices[i];
            SmfVertex {
                id: 0,
                position: v.position,
                color: v.color,
                specular: v.specular,
                uv: [uv_origin + (v.position[1] - low_y) * uv_per_unit, v.uv[1]],
                normal: Some([1.0, 0.0, 0.0]),
            }
        });
        let base = mesh.vertices.len() as u32;
        for (i, mut v) in vertices.into_iter().enumerate() {
            v.id = base + i as u32;
            mesh.vertices.push(v);
        }
        // SMF's clockwise winding is reversed by asset_mesh::mesh. The
        // resulting end faces and their normals both point outwards (+X).
        for indices in [[base, base + 1, base + 2], [base, base + 2, base + 3]] {
            mesh.faces.push(SmfFace {
                id: mesh.faces.len() as u32,
                indices,
            });
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{mesh::Indices, prelude::*};

    fn wall() -> SmfMesh {
        let mut vertices: Vec<_> = (0..38)
            .map(|i| SmfVertex {
                id: i,
                position: [0.0; 3],
                color: u32::MAX,
                specular: 0,
                uv: [0.0; 2],
                normal: None,
            })
            .collect();
        vertices[8].position[0] = -12.0;
        vertices[8].uv = [0.1, 0.9];
        for (i, y, z, v) in [
            (9, -0.3, 0.0, 0.9),
            (11, -0.3, 2.2, 0.3),
            (15, 0.3, 2.2, 0.3),
            (13, 0.3, 0.0, 0.9),
            (32, -0.45, 2.2, 0.3),
            (31, -0.45, 2.6, 0.2),
            (36, 0.45, 2.6, 0.2),
            (35, 0.45, 2.2, 0.3),
        ] {
            vertices[i].position = [2.0, y, z];
            vertices[i].uv = [0.94, v];
        }
        SmfMesh {
            name: "Box02".into(),
            texture: "con_WallComp.BMP".into(),
            vertices,
            faces: Vec::new(),
        }
    }

    #[test]
    fn body_and_wider_coping_close_without_filling_outside_the_profile() {
        let mut source = wall();
        let original: Vec<_> = source.vertices.iter().map(|v| (v.position, v.uv)).collect();
        assert!(close_end(&mut source));
        assert_eq!(source.vertices.len(), original.len() + 8);
        assert_eq!(source.faces.len(), 4);
        assert!(
            source
                .vertices
                .iter()
                .zip(original)
                .all(|(v, (p, uv))| v.position == p && v.uv == uv)
        );
        let mesh = crate::asset_mesh::mesh(&source, Mat4::IDENTITY);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("indices");
        };
        let crosses: Vec<_> = indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from_array(positions[i as usize]));
                (b - a).cross(c - a)
            })
            .collect();
        assert!(
            crosses
                .iter()
                .all(|n| n.x > 0.0 && n.y == 0.0 && n.z == 0.0)
        );
        // Cap area = body width × height + coping width × height.
        assert!(
            (crosses.iter().map(|n| n.length() * 0.5).sum::<f32>() - (0.6 * 2.2 + 0.9 * 0.4)).abs()
                < 0.00001
        );
        assert!(
            source.vertices[38..]
                .iter()
                .all(|v| v.uv.iter().all(|u| u.is_finite()) && v.normal == Some([1.0, 0.0, 0.0]))
        );
    }

    #[test]
    fn replacement_with_original_filename_is_never_repaired() {
        let mut model = SmfModel {
            version: shadow_formats::SmfVersion::V1_1,
            name: "replacement".into(),
            radius: 1.0,
            header_vector: [1.0; 3],
            flags: 0,
            nodes: Vec::new(),
            parents: Vec::new(),
            node_order: Vec::new(),
        };
        assert!(!repair(
            "models/alstwa-concrete01/alstwa-concrete01.smf",
            b"authored replacement",
            &mut model
        ));
    }
}
