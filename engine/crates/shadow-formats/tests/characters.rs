use shadow_formats::CharacterProfile;

fn profile() -> String {
    "CHARACTER_BEGIN\nCHARACTER Hero\nFIRSTNAME Test\nLASTNAME Actor\nBODY_INITIAL uniform\nBODY_DEFINITION\nBODY_TYPE UNIFORM\nHEAD_MODEL Head_Test\nHEAD_MAP Head_Test.BMP\nBODY_MODEL Body_Test\nBODY_MAP Body_Test.BMP\nPACK_MODEL Pack_Test\nPACK_MAP Pack_Test.BMP\nBODY_DEFINITION_END\nGIVE_OBJECT Knife EQUIP\nSKILL Medic 10\nCHARACTER_END".into()
}
#[test]
fn profile_reads_appearance_and_keeps_equipment_and_other_game_data() {
    let p = CharacterProfile::from_bytes(profile().as_bytes()).unwrap();
    assert_eq!(p.id, "Hero");
    assert_eq!(p.display_name(), "Test Actor");
    assert_eq!(p.initial_body().body_model, "body_test");
    assert_eq!(
        p.initial_body().pack.as_ref().unwrap(),
        &("pack_test".into(), "pack_test.bmp".into())
    );
    assert_eq!(p.equipment[0].words, ["GIVE_OBJECT", "Knife", "EQUIP"]);
    assert_eq!(p.properties[0].words, ["SKILL", "Medic", "10"]);
}
#[test]
fn default_body_is_first_and_explicit_initial_body_selects_by_case_insensitive_type() {
    let base = profile()
        .replace("BODY_INITIAL uniform\n", "")
        .replace("CHARACTER_END", "");
    let p = CharacterProfile::from_bytes((base.clone() + "CHARACTER_END").as_bytes()).unwrap();
    assert_eq!(p.initial_body_index, 0);
    let source = base
        + "BODY_DEFINITION\nBODY_TYPE SECOND\nHEAD_MODEL Head_Test\nHEAD_MAP head.bmp\nBODY_MODEL Other\nBODY_MAP other.bmp\nBODY_DEFINITION_END\nBODY_INITIAL second\nCHARACTER_END";
    let p = CharacterProfile::from_bytes(source.as_bytes()).unwrap();
    assert_eq!(p.initial_body_index, 1);
    assert_eq!(p.initial_body().body_model, "other");
    assert!(p.initial_body().pack.is_none());
}
#[test]
fn rejects_partial_duplicate_and_escaping_appearance_fields() {
    for (old, new) in [
        ("CHARACTER_END", ""),
        ("BODY_DEFINITION_END", ""),
        ("BODY_INITIAL uniform", "BODY_INITIAL missing"),
        ("PACK_MAP Pack_Test.BMP\n", ""),
        ("BODY_MODEL Body_Test", "BODY_MODEL ../outside"),
        ("HEAD_MAP Head_Test.BMP", "HEAD_MAP /outside.bmp"),
        (
            "HEAD_MODEL Head_Test",
            "HEAD_MODEL Head_Test\nHEAD_MODEL duplicate",
        ),
        ("BODY_TYPE UNIFORM", "BODY_TYPE UNIFORM\nBODY_DEFINITION"),
        ("CHARACTER Hero", "CHARACTER Hero\nCHARACTER duplicate"),
    ] {
        assert!(
            CharacterProfile::from_bytes(profile().replace(old, new).as_bytes()).is_err(),
            "{new}"
        );
    }
    for source in [
        "",
        "BODY_DEFINITION_END",
        "CHARACTER_BEGIN\nCHARACTER Hero\nCHARACTER_END",
    ] {
        assert!(CharacterProfile::from_bytes(source.as_bytes()).is_err());
    }
}
#[test]
fn retains_additional_appearance_parts_instead_of_silently_discarding_them() {
    let source = profile().replace(
        "BODY_TYPE UNIFORM",
        "BODY_TYPE UNIFORM\nFACE_MODEL Mask\nFACE_MAP Mask.bmp",
    );
    let p = CharacterProfile::from_bytes(source.as_bytes()).unwrap();
    assert_eq!(p.initial_body().extra[0].words, ["FACE_MODEL", "Mask"]);
}
