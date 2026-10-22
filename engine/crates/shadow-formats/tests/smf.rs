use shadow_formats::{SmfModel, SmfVersion};

fn text(bytes: &mut Vec<u8>, value: &str, size: usize) {
    assert!(value.len() < size);
    bytes.extend(value.as_bytes());
    bytes.resize(bytes.len() + size - value.len(), 0);
}
fn u32s(bytes: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
}
fn floats(bytes: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
}

// Самостоятельная модель из одного треугольника, без оригинальных данных.
fn triangle(version: SmfVersion, normal: f32) -> Vec<u8> {
    let mut bytes = b"synthetic SMF\r\n".to_vec();
    bytes.extend([0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x14]);
    text(&mut bytes, version.label(), 16);
    text(&mut bytes, "test model", 128);
    floats(&mut bytes, &[2.0, 1.0, 1.0, 1.0]);
    u32s(&mut bytes, &[1, 2]);
    // Ребёнок записан раньше родителя: формат не требует порядка в файле.
    for (name, parent, has_mesh) in [("child", "root", true), ("root", "<root>", false)] {
        text(&mut bytes, name, 128);
        text(&mut bytes, parent, 128);
        u32s(&mut bytes, &[7]);
        floats(&mut bytes, &[0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0]);
        u32s(&mut bytes, &[u32::from(has_mesh), u32::from(has_mesh)]);
        if version == SmfVersion::V1_1 {
            u32s(&mut bytes, &[4]);
        }
        if has_mesh {
            text(&mut bytes, "triangle", 128);
            text(&mut bytes, "Triangle.BMP", 128);
            u32s(&mut bytes, &[3, 1]);
            for (id, xyz, uv) in [
                (0, [0.0, 0.0, 0.0], [0.0, 0.0]),
                (1, [1.0, 0.0, 0.0], [1.0, 0.0]),
                (2, [0.0, 1.0, 0.0], [0.0, 1.0]),
            ] {
                u32s(&mut bytes, &[id]);
                floats(&mut bytes, &xyz);
                u32s(&mut bytes, &[0xffffffff, 0]);
                floats(&mut bytes, &uv);
                floats(&mut bytes, &[0.0, 0.0, normal]);
            }
            u32s(&mut bytes, &[0, 0, 1, 2]);
            floats(&mut bytes, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
            u32s(&mut bytes, &[0]);
        }
    }
    bytes
}

#[test]
fn reads_both_versions_geometry_and_out_of_order_hierarchy() {
    for version in [SmfVersion::V1_0, SmfVersion::V1_1] {
        let model = SmfModel::from_bytes(&triangle(version, -1.0)).unwrap();
        assert_eq!(model.version, version);
        assert_eq!(model.name, "test model");
        assert_eq!(model.parents, vec![Some(1), None]);
        assert_eq!(model.node_order, vec![1, 0]);
        let child = &model.nodes[0];
        assert_eq!(child.translation, [0.0, 0.0, 2.0]);
        assert_eq!(child.flags, (version == SmfVersion::V1_1).then_some(4));
        assert_eq!(child.bounds.len(), 1);
        let mesh = &child.meshes[0];
        assert_eq!(mesh.texture, "Triangle.BMP");
        assert_eq!(mesh.vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[1].uv, [1.0, 0.0]);
        assert_eq!(mesh.vertices[1].color, 0xffffffff);
        assert_eq!(mesh.vertices[1].normal, Some([0.0, 0.0, -1.0]));
        assert_eq!(mesh.faces[0].indices, [0, 1, 2]);
    }
}

#[test]
fn marks_invalid_source_normals_for_reconstruction() {
    for normal in [f32::NAN, f32::INFINITY, 0.0] {
        let model = SmfModel::from_bytes(&triangle(SmfVersion::V1_1, normal)).unwrap();
        assert!(
            model.nodes[0].meshes[0]
                .vertices
                .iter()
                .all(|v| v.normal.is_none())
        );
    }
}

#[test]
fn rejects_truncation_oversized_counts_and_bad_face_indices() {
    let original = triangle(SmfVersion::V1_1, -1.0);
    for size in [0, 30, original.len() - 1] {
        assert!(SmfModel::from_bytes(&original[..size]).is_err());
    }
    let base = b"synthetic SMF\r\n".len() + 8;
    let mut bad_count = original.clone();
    bad_count[base + 164..base + 168].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(SmfModel::from_bytes(&bad_count).is_err());
    let mesh = base + 168 + 300;
    let face_index = mesh + 264 + 3 * 44 + 4;
    let mut bad_face = original;
    bad_face[face_index..face_index + 4].copy_from_slice(&3_u32.to_le_bytes());
    assert!(SmfModel::from_bytes(&bad_face).is_err());
}

#[test]
fn rejects_cyclic_hierarchy_and_unsafe_texture_references() {
    let original = triangle(SmfVersion::V1_1, -1.0);
    let base = b"synthetic SMF\r\n".len() + 8;
    let child = base + 168;
    let mut cycle = original.clone();
    // child -> child вместо child -> root.
    cycle[child + 128..child + 256].fill(0);
    cycle[child + 128..child + 133].copy_from_slice(b"child");
    assert!(SmfModel::from_bytes(&cycle).is_err());
    let mut escape = original;
    let texture = child + 300 + 128;
    escape[texture..texture + 128].fill(0);
    escape[texture..texture + 13].copy_from_slice(b"../secret.bmp");
    assert!(SmfModel::from_bytes(&escape).is_err());
}

#[test]
fn rejects_nonfinite_positions_and_unexplained_trailing_data() {
    let original = triangle(SmfVersion::V1_1, -1.0);
    let mut bad = original.clone();
    let position = b"synthetic SMF\r\n".len() + 8 + 168 + 300 + 264 + 4;
    bad[position..position + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(SmfModel::from_bytes(&bad).is_err());
    let mut tail = original;
    tail.push(0);
    assert!(SmfModel::from_bytes(&tail).is_err());
}
