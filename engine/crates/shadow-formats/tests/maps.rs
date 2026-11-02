use shadow_formats::{MtfMap, PcxImage, config_lines};
use shadow_formats::{TerrainConfig, TerrainGrid};

fn mapping() -> Vec<u8> {
    b"SET texture_map_base_name training\nSET map_dx 2\nSET map_dy 2\nSET nominal_edge_size 200\nSET altitude_map_height_base 10\nSET min_alt_grad 1\nSET max_alt_grad 1\nSET water_level 0\n".to_vec()
}
#[test]
fn terrain_inverts_indices_and_samples_the_actual_mesh_triangles() {
    let grid = TerrainGrid::new(
        TerrainConfig::from_bytes(&mapping()).unwrap(),
        PcxImage {
            width: 2,
            height: 2,
            pixels: vec![255, 255, 255, 245],
        },
    )
    .unwrap();
    assert_eq!(grid.heights, [0.0, 0.0, 0.0, 100.0]);
    assert_eq!(grid.height_at(100.0, 100.0), Some(50.0)); // bilinear дал бы 25
    assert_eq!(grid.height_at(150.0, 50.0), Some(25.0));
    assert_eq!(grid.height_at(50.0, 150.0), Some(25.0));
    assert_eq!(grid.height_at(200.0, 200.0), Some(100.0));
    assert_eq!(grid.height_at(-0.1, 0.0), None);
    assert_eq!(grid.height_at(f32::NAN, 0.0), None);
    assert_eq!(grid.height_at(200.1, 0.0), None);
}
#[test]
fn terrain_rejects_bad_sizes_scales_names_and_mismatched_pcx() {
    let text = String::from_utf8(mapping()).unwrap();
    for (old, new) in [
        ("map_dx 2", "map_dx 1"),
        ("map_dy 2", "map_dy 9999999"),
        ("size 200", "size 0"),
        ("base 10", "base NaN"),
        ("name training", "name ../escape"),
        ("max_alt_grad 1", "max_alt_grad 2"),
    ] {
        assert!(
            TerrainConfig::from_bytes(text.replace(old, new).as_bytes()).is_err(),
            "{new}"
        );
    }
    assert!(TerrainConfig::from_bytes(b"SET map_dx 2").is_err());
    let mut b = mapping();
    b.extend(b"SET map_dx 2\n");
    assert!(TerrainConfig::from_bytes(&b).is_err());
    assert!(
        TerrainGrid::new(
            TerrainConfig::from_bytes(&mapping()).unwrap(),
            PcxImage {
                width: 3,
                height: 2,
                pixels: vec![0; 6]
            }
        )
        .is_err()
    );
}
fn object() -> Vec<u8> {
    b"OBJECT Scenery Crate \"Box; one\"\r\nOBJECT_POSITION 1 2 3\nOBJECT_ROTATION 0 0 1.57\nOBJECT_ID 1 -42\n".to_vec()
}
#[test]
fn mtf_reads_quoted_names_globals_and_keeps_unknown_properties() {
    let mut data = b"; comment\nGAME_STATE_TIME_OF_DAY 12 4\n".to_vec();
    data.extend(object());
    data.extend(b"OBJECT_MTF_CONFIG \"note with spaces\"\n");
    let map = MtfMap::from_bytes(&data).unwrap();
    assert_eq!(map.globals[0].words, ["GAME_STATE_TIME_OF_DAY", "12", "4"]);
    assert_eq!(map.objects[0].name, "Box; one");
    assert_eq!(map.objects[0].position, Some([1.0, 2.0, 3.0]));
    assert_eq!(map.objects[0].id, [1, -42]);
    assert_eq!(map.objects[0].properties[0].words[1], "note with spaces");
    let mut inventory = object();
    inventory.splice(..6, b"OBJECT_INVENTORY".iter().copied());
    assert!(MtfMap::from_bytes(&inventory).unwrap().objects[0].inventory);
}
#[test]
fn mtf_preserves_original_indefinite_height_without_inventing_a_position() {
    let data = String::from_utf8(object())
        .unwrap()
        .replace("POSITION 1 2 3", "POSITION 1 2 -1.#IND00");
    let map = MtfMap::from_bytes(data.as_bytes()).unwrap();
    assert_eq!(map.objects[0].position, None);
    assert_eq!(
        map.objects[0].properties[0].words,
        ["OBJECT_POSITION", "1", "2", "-1.#IND00"]
    );
}
#[test]
fn mtf_rejects_incomplete_or_nonfinite_properties() {
    for input in [
        "OBJECT_POSITION 1 2 3",
        "OBJECT X Y Z",
        "OBJECT X Y Z\nOBJECT_POSITION NaN 0 0",
        "OBJECT X Y Z\nOBJECT_POSITION 0 0 0\nOBJECT_POSITION 1 2 3",
        "OBJECT X Y Z\nOBJECT_POSITION 0 0 0\nOBJECT_ROTATION 0 0 0\nOBJECT_ID 1 2147483648",
    ] {
        assert!(MtfMap::from_bytes(input.as_bytes()).is_err(), "{input}");
    }
    let mut data = object();
    data.extend(b"OBJECT X Y Z\n");
    assert!(MtfMap::from_bytes(&data).is_err());
}
#[test]
fn text_handles_windows_encoding_comments_and_bad_quotes() {
    let lines = config_lines(b"X \"caf\xe9; room\" ; ignored\r\nY \"\"\n").unwrap();
    assert_eq!(lines[0].words, ["X", "café; room"]);
    assert_eq!(lines[1].words, ["Y", ""]);
    assert!(config_lines(b"X \"unterminated").is_err());
    assert!(config_lines(b"X\0Y").is_err());
}
fn pcx() -> Vec<u8> {
    let mut b = vec![0; 128];
    b[..4].copy_from_slice(&[10, 5, 1, 8]);
    b[8..10].copy_from_slice(&2u16.to_le_bytes()); // width 3, stride 4
    b[10..12].copy_from_slice(&1u16.to_le_bytes());
    b[65] = 1;
    b[66..68].copy_from_slice(&4u16.to_le_bytes());
    b.extend([0xc3, 200, 0, 1, 2, 3, 0]);
    b
}
#[test]
fn pcx_decodes_rle_and_removes_scanline_padding() {
    let mut data = pcx();
    let im = PcxImage::from_bytes(&data).unwrap();
    assert_eq!((im.width, im.height), (3, 2));
    assert_eq!(im.pixels, [200, 200, 200, 1, 2, 3]);
    data.push(12);
    data.extend([0; 768]);
    assert_eq!(PcxImage::from_bytes(&data).unwrap().pixels, im.pixels);
}
#[test]
fn pcx_rejects_truncation_overflow_and_unsupported_modes() {
    let valid = pcx();
    for n in 0..valid.len() {
        assert!(PcxImage::from_bytes(&valid[..n]).is_err(), "length {n}");
    }
    for (index, value) in [
        (0, 0),
        (2, 0),
        (3, 24),
        (65, 3),
        (66, 2),
        (128, 0xc0),
        (128, 0xc5),
    ] {
        let mut b = valid.clone();
        b[index] = value;
        assert!(
            PcxImage::from_bytes(&b).is_err(),
            "index {index}, value {value}"
        );
    }
    let mut b = valid.clone();
    b[8..10].copy_from_slice(&4096u16.to_le_bytes());
    assert!(PcxImage::from_bytes(&b).is_err());
    let mut b = valid;
    b.push(99);
    assert!(PcxImage::from_bytes(&b).is_err());
}
