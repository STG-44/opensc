//! Сверка миссии со связанными ресурсами; игровой мир не запускается.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shadow_formats::{
    AssetCatalog, CampaignActionType, CampaignEventType, ConfigLine, HelpDefinitions,
    MissionDefinition, MtfMap, config_lines, normalize_path,
};
use std::{collections::BTreeSet, error::Error};

fn lines(values: &[ConfigLine]) -> Value {
    json!(
        values
            .iter()
            .map(|l| json!({"line":l.number,"words":l.words}))
            .collect::<Vec<_>>()
    )
}
fn resource(catalog: &AssetCatalog, path: &str) -> Result<(Vec<u8>, Value), Box<dyn Error>> {
    let path = normalize_path(path)?;
    let bytes = catalog.read(&path)?;
    let info = json!({"path":path,"source":catalog.source_label(&path)?,
        "bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))});
    Ok((bytes, info))
}
fn relative(folder: &str, path: &str) -> Result<String, Box<dyn Error>> {
    Ok(normalize_path(&format!("{folder}/{path}"))?)
}

pub fn describe(catalog: &AssetCatalog, path: &str) -> Result<Value, Box<dyn Error>> {
    let path = normalize_path(path)?;
    let (folder, filename) = path
        .rsplit_once('/')
        .ok_or("сценарий должен иметь каталог ресурсов")?;
    let basename = filename
        .strip_suffix(".txt")
        .ok_or("сценарий должен быть .txt")?;
    let (bytes, source) = resource(catalog, &path)?;
    let mission = MissionDefinition::from_bytes(&bytes)?;
    let map_path = relative("maps", &mission.map)?;
    let (map_bytes, map_source) = resource(catalog, &map_path)?;
    let map = MtfMap::from_bytes(&map_bytes)?;
    let (help_bytes, help_source) = resource(catalog, "config/help_window_defs.txt")?;
    let help = HelpDefinitions::from_bytes(&help_bytes)?;
    let mut help_ids = BTreeSet::new();
    for response in &mission.responses {
        if response.event == CampaignEventType::BodyWindowClosed {
            help_ids.insert(response.subject.as_deref().unwrap().to_ascii_lowercase());
        }
        for action in &response.actions {
            if action.kind == CampaignActionType::ShowContextHelp {
                help_ids.insert(action.identifier.as_deref().unwrap().to_ascii_lowercase());
            }
        }
    }
    let mut context_help = Vec::new();
    for id in &help_ids {
        let entry = help
            .get(id)
            .ok_or_else(|| format!("сценарий ссылается на отсутствующую подсказку {id}"))?;
        context_help.push(json!({"id":entry.id,"line":entry.line,"context":entry.context,
            "pause_game":entry.pause_game,"original_position":entry.position,
            "original_dimensions":entry.dimensions,"body":entry.body.iter().map(|l|
                json!({"line":l.number,"text":l.words[1]})).collect::<Vec<_>>(),"metadata":lines(&entry.metadata)}));
    }
    let mut ai = Vec::new();
    for definition in &mission.ai {
        let ai_path = relative(folder, &definition.file)?;
        let (ai_bytes, ai_source) = resource(catalog, &ai_path)?;
        ai.push(
            json!({"line":definition.line,"player":definition.player,"file":definition.file,
            "resource":ai_source,"configuration":lines(&config_lines(&ai_bytes)?)}),
        );
    }
    let ambient = if let Some(file) = &mission.ambient {
        let (ambient_bytes, info) = resource(catalog, &relative(folder, file)?)?;
        Some(json!({"resource":info,"configuration":lines(&config_lines(&ambient_bytes)?)}))
    } else {
        None
    };
    // BASENAME выбирает блок кампании; действия пока сохраняются, а не исполняются.
    let (campaign_bytes, campaign_source) = resource(catalog, "config/campaign_defs.txt")?;
    let campaign_lines = config_lines(&campaign_bytes)?;
    let mut matching = Vec::new();
    let mut start = None;
    for (index, line) in campaign_lines.iter().enumerate() {
        if line.words[0] == "CAMPAIGN_DEF"
            && let Some(first) = start.replace(index)
        {
            let block = &campaign_lines[first..index];
            if is_campaign(block, basename) {
                matching.push(block);
            }
        }
    }
    if let Some(first) = start {
        let block = &campaign_lines[first..];
        if is_campaign(block, basename) {
            matching.push(block);
        }
    }
    if matching.len() > 1 {
        return Err(format!("повтор кампании {basename}").into());
    }
    let campaign = matching.first().map(|block| json!({"resource":campaign_source,"configuration":lines(block),
        "pre_actions":lines(&block.iter().filter(|l|l.words[0] == "PRE_ACTION").cloned().collect::<Vec<_>>()),
        "post_actions":lines(&block.iter().filter(|l|l.words[0] == "POST_ACTION").cloned().collect::<Vec<_>>())}));
    let subjects: BTreeSet<_> = mission
        .responses
        .iter()
        .filter_map(|r| r.subject.as_ref())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let relevant_objects = map.objects.iter().filter(|o| o.inventory || o.class.eq_ignore_ascii_case("Bipedal")
        || subjects.contains(&o.name.to_ascii_lowercase()) || subjects.contains(&o.model.to_ascii_lowercase()))
        .map(|o| json!({"class":o.class,"model":o.model,"name":o.name,"inventory":o.inventory,
            "id":o.id,"source_position":o.position,"source_rotation":o.rotation,"properties":lines(&o.properties)}))
        .collect::<Vec<_>>();
    let responses = mission.responses.iter().map(|r| {
        // Это список кандидатов по исходным именам, а не готовое связывание событий.
        let object_candidates = map.objects.iter().filter(|o|r.subject.as_ref().is_some_and(|s|
            s.eq_ignore_ascii_case(&o.name) || s.eq_ignore_ascii_case(&o.model))).map(|o|o.id).collect::<Vec<_>>();
        let player_candidates = mission.players.iter().filter(|p|!p.multiplayer
            && r.subject.as_ref().is_some_and(|s|s.eq_ignore_ascii_case(&p.name))).map(|p|p.id).collect::<Vec<_>>();
        json!({"line":r.line,"event":r.event.as_str(),"subject":r.subject,
            "map_object_candidates":object_candidates,"single_player_candidates":player_candidates,
            "actions":r.actions.iter().map(|a|json!({"line":a.line,"kind":a.kind.as_str(),
                "identifier":a.identifier,"message":a.message,"delay_ms":a.delay_ms,"extra":lines(&a.extra)})).collect::<Vec<_>>(),
            "extra":lines(&r.extra)})
    }).collect::<Vec<_>>();
    let mut conversations = Vec::new();
    for conversation in &mission.conversations {
        let mut elements = Vec::new();
        for element in &conversation.elements {
            let audio_path = normalize_path(&format!(
                "sounds/speech/{}/{}",
                element.words[1], element.words[2]
            ))?;
            // Шаблон speech/<actor> подтверждён для обучения. Другие соглашения — кандидаты.
            let audio = catalog
                .paths()
                .any(|p| p == audio_path)
                .then(|| resource(catalog, &audio_path))
                .transpose()?;
            elements.push(json!({"line":element.number,"words":element.words,
                "speech_resource_candidate":audio.map(|(_,info)|info)}));
        }
        conversations.push(json!({"line":conversation.line,"id":conversation.id,
            "elements":elements,"extra":lines(&conversation.extra)}));
    }
    Ok(
        json!({"schema_version":1,"source":source,"execution_implemented":false,
        "delay_semantics":"preserved_source_order_not_scheduled","initial_view":mission.initial_view,
        "map":map_source,"campaign":campaign,"ambient":ambient,
        "players":mission.players.iter().map(|p|json!({"line":p.line,"mode":if p.multiplayer {"multiplayer"} else {"singleplayer"},
            "kind":p.kind,"name":p.name,"id":p.id})).collect::<Vec<_>>(),
        "alliances":lines(&mission.alliances),"ai":ai,
        "zones":mission.zones.iter().map(|z|json!({"line":z.line,"id":z.id,"source_center":z.center,
            "source_radius":z.radius,"color":z.color})).collect::<Vec<_>>(),
        "objectives":mission.objectives.iter().map(|o|json!({"line":o.line,"id":o.id,"primary":o.primary,
            "description":o.description,"zone":o.zone})).collect::<Vec<_>>(),
        "objective_links":mission.objective_links.iter().map(|l|json!({"line":l.line,"on_complete":l.on_complete,"enable":l.enable})).collect::<Vec<_>>(),
        "responses":responses,"conversations":conversations,"globals":lines(&mission.globals),
        "map_objects":relevant_objects,"help_resource":help_source,"help_total_entries":help.entries.len(),
        "help_repaired_quote_lines":help.repaired_quote_lines,"context_help":context_help,
        "diagnostics":mission.diagnostics.iter().map(|d|json!({"line":d.line,"message":d.message})).collect::<Vec<_>>(),
        "counts":{"zones":mission.zones.len(),"objectives":mission.objectives.len(),"objective_links":mission.objective_links.len(),
            "responses":mission.responses.len(),"actions":mission.responses.iter().map(|r|r.actions.len()).sum::<usize>(),
            "context_help":context_help.len(),"ai_configurations":mission.ai.len(),
            "map_actors":map.objects.iter().filter(|o|o.class.eq_ignore_ascii_case("Bipedal")).count(),
            "map_inventory":map.objects.iter().filter(|o|o.inventory).count()}}),
    )
}

fn is_campaign(block: &[ConfigLine], basename: &str) -> bool {
    block.iter().any(|l| {
        l.words[0] == "BASENAME"
            && l.words
                .get(1)
                .is_some_and(|id| id.eq_ignore_ascii_case(basename))
    })
}
