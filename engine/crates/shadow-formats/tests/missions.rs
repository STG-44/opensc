use shadow_formats::{
    CampaignActionType, CampaignEventType, HelpDefinitions, MissionDefinition, config_lines,
};

// Самостоятельная мини-миссия; оригинальные данные не включаются в исходники.
fn mission() -> String {
    "SPECIFY_MTF test.mtf\n\
SPECIFY_VIEW_INITIAL 1 2 3 4\n\
SPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_NONSCRIPTED Team 2\n\
SPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_SCRIPTED Guards 3\n\
MPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_SCRIPTED Guards 3\n\
SET_PLAYER_ALLIANCE Guards\n\
CONFIGURE_PLAYER_AI Guards ai.txt\n\
SET_ZONE exit 10 20 5 Blue\n\
;DEFINE_OBJECTIVE true commented \"Not an objective\" exit\n\
DEFINE_OBJECTIVE true first \"First; task\" EXIT\n\
DEFINE_OBJECTIVE false second \"Second task\"\n\
ADD_OBJECTIVE_ENABLE FIRST second\n\
CER_DEFINITION\n\
CER_FOR_TYPE CET_IMMEDIATE\n\
CER_BODY_DEFINITION\n\
CER_BODY_TYPE CER_BODY_ENABLE_OBJECTIVE\n\
CER_BODY_IDENTIFIER first\n\
CER_DEFINITION\n\
CER_FOR_TYPE CET_ALL_PLAYERS_INFANTRY_KILLED\n\
CER_FOR gUaRdS\n\
CER_BODY_DEFINITION\n\
CER_BODY_TYPE CER_BODY_NOTIFY_OBJECTIVE_COMPLETE\n\
CER_BODY_IDENTIFIER first\n\
CER_BODY_MS_DELAY 100\n\
CER_BODY_DEFINITION\n\
CER_BODY_TYPE CER_BODY_TRIGGER_CONVERSE\n\
CER_BODY_IDENTIFIER greeting\n\
CER_BODY_MS_DELAY 20\n\
CER_BODY_DEFINITION\n\
CER_BODY_TYPE CER_BODY_REMOVE_CER\n\
CONVERSE_SEQUENCE_DEFINITION\n\
CONVERSE_IDENTIFIER greeting\n\
CONVERSE_ELEMENT Friend voice.wav\n"
        .to_owned()
}

#[test]
fn imports_sequence_players_zones_and_case_insensitive_references() {
    let m = MissionDefinition::from_bytes(mission().as_bytes()).unwrap();
    assert_eq!(m.map, "test.mtf");
    assert_eq!(m.initial_view, Some([1.0, 2.0, 3.0, 4.0]));
    assert_eq!(m.objectives.len(), 2);
    assert_eq!(m.objectives[0].description, "First; task");
    assert!(!m.objectives[1].primary);
    assert_eq!(m.players.len(), 3); // SPM и MPM — отдельные пространства ID.
    assert!(m.players[2].multiplayer);
    assert_eq!(m.zones[0].center, [10.0, 20.0]);
    assert_eq!(m.zones[0].radius, 5.0);
    assert_eq!(
        m.responses[1].event,
        CampaignEventType::AllPlayerInfantryKilled
    );
    assert_eq!(m.responses[1].subject.as_deref(), Some("gUaRdS"));
    assert_eq!(m.responses[1].actions[0].delay_ms, Some(100));
    assert_eq!(m.responses[1].actions[1].delay_ms, Some(20));
    assert_eq!(m.responses[1].actions[2].delay_ms, None);
    assert_eq!(
        m.responses[1].actions[2].kind,
        CampaignActionType::RemoveResponse
    );
    assert_eq!(
        m.conversations[0].elements[0].words[1..],
        ["Friend", "voice.wav"]
    );
    assert!(m.diagnostics.is_empty());
}

#[test]
fn empty_original_cer_is_diagnosed_without_creating_an_immediate_event() {
    let source = mission().replace("CER_DEFINITION", "CER_DEFINITION\nCER_DEFINITION");
    let m = MissionDefinition::from_bytes(source.as_bytes()).unwrap();
    assert_eq!(m.responses.len(), 2);
    assert_eq!(m.diagnostics.len(), 2);
    assert_eq!(m.diagnostics[0].line, 13);
}

#[test]
fn preserves_unimplemented_commands_and_reports_unknown_event_actions() {
    let source = mission().replace("CER_BODY_REMOVE_CER", "CER_BODY_CUSTOM\nCER_BODY_EXTRA 17")
        + "\nENVIRONMENT_CUSTOM 42\nCER_DEFINITION\nCER_FOR_TYPE CET_CUSTOM\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_CUSTOM\n";
    let m = MissionDefinition::from_bytes(source.as_bytes()).unwrap();
    assert_eq!(
        m.globals.last().unwrap().words,
        ["ENVIRONMENT_CUSTOM", "42"]
    );
    assert_eq!(m.responses[1].actions[2].kind.as_str(), "CER_BODY_CUSTOM");
    assert_eq!(
        m.responses[1].actions[2].extra[0].words,
        ["CER_BODY_EXTRA", "17"]
    );
    assert_eq!(m.responses[2].event.as_str(), "CET_CUSTOM");
    assert_eq!(m.diagnostics.len(), 4);
}

#[test]
fn rejects_dangling_references_and_objective_cycles() {
    for (old, new) in [
        ("task\" EXIT", "task\" nowhere"),
        ("ENABLE FIRST second", "ENABLE first missing"),
        (
            "ENABLE FIRST second",
            "ENABLE FIRST second\nADD_OBJECTIVE_ENABLE second first",
        ),
        ("IDENTIFIER first", "IDENTIFIER missing"),
        (
            "CER_BODY_IDENTIFIER greeting",
            "CER_BODY_IDENTIFIER missing",
        ),
        ("CER_FOR gUaRdS", "CER_FOR missing"),
        ("CONFIGURE_PLAYER_AI Guards", "CONFIGURE_PLAYER_AI missing"),
        ("SET_PLAYER_ALLIANCE Guards", "SET_PLAYER_ALLIANCE missing"),
    ] {
        assert!(
            MissionDefinition::from_bytes(mission().replace(old, new).as_bytes()).is_err(),
            "{new}"
        );
    }
}

#[test]
fn rejects_duplicates_in_each_namespace() {
    for addition in [
        "SPECIFY_MTF again.mtf",
        "SET_ZONE EXIT 1 2 3 Blue",
        "DEFINE_OBJECTIVE true FIRST duplicate",
        "ADD_OBJECTIVE_ENABLE first second",
        "SPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_SCRIPTED gUaRdS 4",
        "SPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_SCRIPTED Other 3",
        "CONVERSE_SEQUENCE_DEFINITION\nCONVERSE_IDENTIFIER GREETING\nCONVERSE_ELEMENT Friend voice.wav",
    ] {
        assert!(
            MissionDefinition::from_bytes((mission() + addition).as_bytes()).is_err(),
            "{addition}"
        );
    }
}

#[test]
fn rejects_nonfinite_zones_invalid_delays_and_escaping_resource_paths() {
    for (old, new) in [
        ("exit 10 20 5", "exit NaN 20 5"),
        ("exit 10 20 5", "exit 10 20 0"),
        ("exit 10 20 5", "exit 10 20 inf"),
        ("1 2 3 4", "1 2 inf 4"),
        ("DELAY 100", "DELAY -1"),
        ("DELAY 100", "DELAY 4294967296"),
        ("test.mtf", "../test.mtf"),
        ("ai.txt", "../ai.txt"),
        ("voice.wav", "C:\\voice.wav"),
        ("true first", "maybe first"),
    ] {
        assert!(
            MissionDefinition::from_bytes(mission().replace(old, new).as_bytes()).is_err(),
            "{new}"
        );
    }
}

#[test]
fn rejects_partial_and_misplaced_blocks_without_panicking() {
    for input in [
        "SPECIFY_MTF test.mtf\nCER_FOR_TYPE CET_IMMEDIATE",
        "SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER",
        "SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_IMMEDIATE\nCER_BODY_TYPE CER_BODY_REMOVE_CER",
        "SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_PICKED_UP\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER",
        "SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_IMMEDIATE\nCER_BODY_DEFINITION",
        "SPECIFY_MTF test.mtf\nCONVERSE_SEQUENCE_DEFINITION\nCONVERSE_IDENTIFIER empty",
        "SPECIFY_MTF test.mtf\nCONVERSE_ELEMENT Friend voice.wav",
        "",
    ] {
        assert!(
            MissionDefinition::from_bytes(input.as_bytes()).is_err(),
            "{input}"
        );
    }
    for (old, new) in [
        ("CER_FOR gUaRdS", "CER_FOR gUaRdS\nCER_FOR Guards"),
        ("DELAY 100", "DELAY 100\nCER_BODY_MS_DELAY 50"),
        (
            "CER_BODY_REMOVE_CER",
            "CER_BODY_REMOVE_CER\nCER_FOR_TYPE CET_IMMEDIATE",
        ),
    ] {
        assert!(
            MissionDefinition::from_bytes(mission().replace(old, new).as_bytes()).is_err(),
            "{new}"
        );
    }
}

#[test]
fn inventory_pulsing_requires_a_boolean_identifier() {
    let source = "SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_IMMEDIATE\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SET_PULSATE_ON_MAP_INVENTORY_OBJECTS\nCER_BODY_IDENTIFIER TRUE";
    assert!(MissionDefinition::from_bytes(source.as_bytes()).is_ok());
    assert!(MissionDefinition::from_bytes(source.replace("TRUE", "sometimes").as_bytes()).is_err());
    assert!(
        MissionDefinition::from_bytes(source.replace("CER_BODY_IDENTIFIER TRUE", "").as_bytes())
            .is_err()
    );
}

#[test]
fn preserves_ending_messages_and_explicit_zero_delay() {
    let source = b"SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_SIGNAL_FLARE_POPPED\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_END_CAMPAIGN\nCER_BODY_IDENTIFIER EOC_RETURN_TO_MAIN_MENU\nCER_BODY_MESSAGE \"A; message\"\nCER_BODY_MS_DELAY 0";
    let m = MissionDefinition::from_bytes(source).unwrap();
    let action = &m.responses[0].actions[0];
    assert_eq!(
        action.identifier.as_deref(),
        Some("EOC_RETURN_TO_MAIN_MENU")
    );
    assert_eq!(action.message.as_deref(), Some("A; message"));
    assert_eq!(action.delay_ms, Some(0));
}

#[test]
fn help_preserves_paragraphs_codepage_and_pause_flags() {
    let source = b"HELP_DEF\nHELP_ID Tip\nHELP_CONTEXT\nHELP_DO_NOT_PAUSE_GAME\nHELP_DIMS 500 200\nHELP_POS 0 -10\nHELP_BODY \"caf\xe9; text\" ; comment\nHELP_BODY \"\"\nHELP_TUTOR_NEXT Other\nHELP_DEF\nHELP_ID Other\nHELP_BODY \"Next\"";
    let h = HelpDefinitions::from_bytes(source).unwrap();
    let tip = h.get("tIP").unwrap();
    assert_eq!(tip.body[0].words[1], "café; text");
    assert_eq!(tip.body[1].words[1], "");
    assert!(tip.context);
    assert!(!tip.pause_game);
    assert_eq!(tip.dimensions, Some([500, 200]));
    assert_eq!(tip.position, Some([0, -10]));
    assert_eq!(tip.metadata[0].words, ["HELP_TUTOR_NEXT", "Other"]);
    assert!(h.get("other").unwrap().pause_game);
    assert!(h.repaired_quote_lines.is_empty());
}

#[test]
fn narrowly_repairs_missing_help_body_quote_without_relaxing_general_config() {
    let source = b"HELP_DEF\nHELP_ID tip\nHELP_BODY \"unterminated; still text\nHELP_BODY \"next\"";
    let h = HelpDefinitions::from_bytes(source).unwrap();
    assert_eq!(h.repaired_quote_lines, [3]);
    assert_eq!(h.entries[0].body[0].words[1], "unterminated; still text");
    assert_eq!(h.entries[0].body[1].words[1], "next");
    assert!(config_lines(source).is_err());
    for input in [
        "HELP_DEF\nHELP_ID \"broken",
        "HELP_DEF\nHELP_ID tip\nHELP_BODY unquoted\"broken",
        "HELP_DEF\nHELP_ID tip\nHELP_BODY \"first\" \"broken",
    ] {
        assert!(
            HelpDefinitions::from_bytes(input.as_bytes()).is_err(),
            "{input}"
        );
    }
}

#[test]
fn help_rejects_missing_duplicate_and_invalid_fields() {
    for input in [
        "HELP_BODY \"out of block\"",
        "HELP_DEF",
        "HELP_DEF\nHELP_DEF\nHELP_ID second",
        "HELP_DEF\nHELP_ID one\nHELP_ID two",
        "HELP_DEF\nHELP_ID one\nHELP_DEF\nHELP_ID ONE",
        "HELP_DEF\nHELP_ID one\nHELP_DIMS 0 10",
        "HELP_DEF\nHELP_ID one\nHELP_DIMS 1 NaN",
        "HELP_DEF\nHELP_ID one\nHELP_POS 0 2147483648",
        "HELP_DEF\nHELP_ID one\nHELP_DIMS 1 10\nHELP_DIMS 2 10",
        "HELP_DEF\nHELP_ID one\nHELP_BODY \"one\" extra",
        "",
    ] {
        assert!(
            HelpDefinitions::from_bytes(input.as_bytes()).is_err(),
            "{input}"
        );
    }
}

#[test]
fn parsers_reject_nul_and_excessive_input() {
    let huge = vec![b' '; 16 * 1024 * 1024 + 1];
    for bytes in [b"X\0Y".as_slice(), huge.as_slice()] {
        assert!(MissionDefinition::from_bytes(bytes).is_err());
        assert!(HelpDefinitions::from_bytes(bytes).is_err());
    }
}
