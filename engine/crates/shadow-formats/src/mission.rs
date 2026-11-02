//! Декларации миссии и CER (Campaign Event Responses), без исполнения событий.
//! Порядок действий и исходные задержки сохраняются: это ещё не планировщик.
use crate::{ConfigLine, Error, Result, config_lines, normalize_path};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignEventType {
    Immediate,
    ObjectPickedUp,
    HumanUnconscious,
    HumanKilled,
    AllLocalCommandosKilled,
    ObjectActivated,
    BodyWindowClosed,
    VehicleOccupied,
    AllPlayerInfantryKilled,
    FenceCut,
    ObjectInConverseRange,
    SignalFlarePopped,
    Unknown(String),
}
impl CampaignEventType {
    fn parse(value: &str) -> Self {
        match value {
            "CET_IMMEDIATE" => Self::Immediate,
            "CET_OBJECT_PICKED_UP" => Self::ObjectPickedUp,
            "CET_HUMAN_UNCONSIOUS" => Self::HumanUnconscious,
            "CET_HUMAN_KILLED" => Self::HumanKilled,
            "CET_ALL_LOCAL_NS_COMMANDOS_KILLED" => Self::AllLocalCommandosKilled,
            "CET_OBJECT_ACTIVATED" => Self::ObjectActivated,
            "CET_BODY_WINDOW_CLOSED" => Self::BodyWindowClosed,
            "CET_VEHICLE_OCCUPIED" => Self::VehicleOccupied,
            "CET_ALL_PLAYERS_INFANTRY_KILLED" => Self::AllPlayerInfantryKilled,
            "CET_FENCE_CUT" => Self::FenceCut,
            "CET_OBJECT_ENCOUNTERED_WITHIN_CONVERSE_RANGE" => Self::ObjectInConverseRange,
            "CET_SIGNAL_FLARE_POPPED" => Self::SignalFlarePopped,
            _ => Self::Unknown(value.to_owned()),
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Self::Immediate => "CET_IMMEDIATE",
            Self::ObjectPickedUp => "CET_OBJECT_PICKED_UP",
            Self::HumanUnconscious => "CET_HUMAN_UNCONSIOUS",
            Self::HumanKilled => "CET_HUMAN_KILLED",
            Self::AllLocalCommandosKilled => "CET_ALL_LOCAL_NS_COMMANDOS_KILLED",
            Self::ObjectActivated => "CET_OBJECT_ACTIVATED",
            Self::BodyWindowClosed => "CET_BODY_WINDOW_CLOSED",
            Self::VehicleOccupied => "CET_VEHICLE_OCCUPIED",
            Self::AllPlayerInfantryKilled => "CET_ALL_PLAYERS_INFANTRY_KILLED",
            Self::FenceCut => "CET_FENCE_CUT",
            Self::ObjectInConverseRange => "CET_OBJECT_ENCOUNTERED_WITHIN_CONVERSE_RANGE",
            Self::SignalFlarePopped => "CET_SIGNAL_FLARE_POPPED",
            Self::Unknown(value) => value,
        }
    }
    fn requires_subject(&self) -> bool {
        !matches!(
            self,
            Self::Immediate
                | Self::AllLocalCommandosKilled
                | Self::SignalFlarePopped
                | Self::Unknown(_)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampaignActionType {
    EnableObjective,
    ShowContextHelp,
    SetInventoryPulse,
    EndCampaign,
    RemoveResponse,
    CompleteObjective,
    CompleteObjectiveNoAudio,
    TriggerConversation,
    GiveLocalControl,
    Unknown(String),
}
impl CampaignActionType {
    fn parse(value: &str) -> Self {
        match value {
            "CER_BODY_ENABLE_OBJECTIVE" => Self::EnableObjective,
            "CER_BODY_SHOW_CONTEXT_HELP" => Self::ShowContextHelp,
            "CER_BODY_SET_PULSATE_ON_MAP_INVENTORY_OBJECTS" => Self::SetInventoryPulse,
            "CER_BODY_END_CAMPAIGN" => Self::EndCampaign,
            "CER_BODY_REMOVE_CER" => Self::RemoveResponse,
            "CER_BODY_NOTIFY_OBJECTIVE_COMPLETE" => Self::CompleteObjective,
            "CER_BODY_NOTIFY_OBJECTIVE_COMPLETE_NO_AUDIO" => Self::CompleteObjectiveNoAudio,
            "CER_BODY_TRIGGER_CONVERSE" => Self::TriggerConversation,
            "CER_BODY_GIVE_LNS_PLAYER_OBJECT_CONTROL" => Self::GiveLocalControl,
            _ => Self::Unknown(value.to_owned()),
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Self::EnableObjective => "CER_BODY_ENABLE_OBJECTIVE",
            Self::ShowContextHelp => "CER_BODY_SHOW_CONTEXT_HELP",
            Self::SetInventoryPulse => "CER_BODY_SET_PULSATE_ON_MAP_INVENTORY_OBJECTS",
            Self::EndCampaign => "CER_BODY_END_CAMPAIGN",
            Self::RemoveResponse => "CER_BODY_REMOVE_CER",
            Self::CompleteObjective => "CER_BODY_NOTIFY_OBJECTIVE_COMPLETE",
            Self::CompleteObjectiveNoAudio => "CER_BODY_NOTIFY_OBJECTIVE_COMPLETE_NO_AUDIO",
            Self::TriggerConversation => "CER_BODY_TRIGGER_CONVERSE",
            Self::GiveLocalControl => "CER_BODY_GIVE_LNS_PLAYER_OBJECT_CONTROL",
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug)]
pub struct MissionZone {
    pub line: usize,
    pub id: String,
    /// Горизонтальные X/Y оригинала, ещё без преобразования в координаты Bevy.
    pub center: [f32; 2],
    pub radius: f32,
    pub color: String,
}
#[derive(Debug)]
pub struct MissionObjective {
    pub line: usize,
    pub id: String,
    pub primary: bool,
    pub description: String,
    pub zone: Option<String>,
}
#[derive(Debug)]
pub struct MissionObjectiveLink {
    pub line: usize,
    pub on_complete: String,
    pub enable: String,
}
#[derive(Debug)]
pub struct MissionPlayer {
    pub line: usize,
    pub multiplayer: bool,
    pub kind: String,
    pub name: String,
    pub id: i32,
}
#[derive(Debug)]
pub struct MissionAi {
    pub line: usize,
    pub player: String,
    pub file: String,
}
#[derive(Debug)]
pub struct MissionAction {
    pub line: usize,
    pub kind: CampaignActionType,
    pub identifier: Option<String>,
    pub message: Option<String>,
    /// None — поле отсутствует. Значение и порядок не интерпретируются как абсолютное время.
    pub delay_ms: Option<u32>,
    pub extra: Vec<ConfigLine>,
}
#[derive(Debug)]
pub struct MissionResponse {
    pub line: usize,
    pub event: CampaignEventType,
    pub subject: Option<String>,
    pub actions: Vec<MissionAction>,
    pub extra: Vec<ConfigLine>,
}
#[derive(Debug)]
pub struct MissionConversation {
    pub line: usize,
    pub id: String,
    /// CONVERSE_ELEMENT: участник, аудиоресурс и дополнительные исходные параметры.
    pub elements: Vec<ConfigLine>,
    pub extra: Vec<ConfigLine>,
}
#[derive(Debug)]
pub struct MissionDiagnostic {
    pub line: usize,
    pub message: String,
}
#[derive(Debug)]
pub struct MissionDefinition {
    pub map: String,
    pub ambient: Option<String>,
    pub initial_view: Option<[f32; 4]>,
    pub players: Vec<MissionPlayer>,
    pub alliances: Vec<ConfigLine>,
    pub ai: Vec<MissionAi>,
    pub zones: Vec<MissionZone>,
    pub objectives: Vec<MissionObjective>,
    pub objective_links: Vec<MissionObjectiveLink>,
    pub responses: Vec<MissionResponse>,
    pub conversations: Vec<MissionConversation>,
    pub globals: Vec<ConfigLine>,
    pub diagnostics: Vec<MissionDiagnostic>,
}

#[derive(Default)]
struct ActionBuilder {
    line: usize,
    kind: Option<CampaignActionType>,
    identifier: Option<String>,
    message: Option<String>,
    delay_ms: Option<u32>,
    extra: Vec<ConfigLine>,
}
impl ActionBuilder {
    fn finish(self) -> Result<MissionAction> {
        let kind = self
            .kind
            .ok_or_else(|| invalid(self.line, "нет CER_BODY_TYPE"))?;
        if !matches!(
            kind,
            CampaignActionType::RemoveResponse | CampaignActionType::Unknown(_)
        ) && self.identifier.is_none()
        {
            return Err(invalid(self.line, "действию нужен CER_BODY_IDENTIFIER"));
        }
        if kind == CampaignActionType::SetInventoryPulse {
            boolean(self.identifier.as_deref().unwrap(), self.line)?;
        }
        Ok(MissionAction {
            line: self.line,
            kind,
            identifier: self.identifier,
            message: self.message,
            delay_ms: self.delay_ms,
            extra: self.extra,
        })
    }
}
struct ResponseBuilder {
    line: usize,
    event: Option<CampaignEventType>,
    subject: Option<String>,
    actions: Vec<MissionAction>,
    body: Option<ActionBuilder>,
    extra: Vec<ConfigLine>,
}
enum Block {
    Response(ResponseBuilder),
    Conversation(MissionConversation),
}

fn invalid(line: usize, message: &str) -> Error {
    Error::Invalid(format!("сценарий: строка {line}: {message}"))
}
fn arity(line: &ConfigLine, count: usize) -> Result<()> {
    if line.words.len() != count {
        return Err(invalid(line.number, "неверное число аргументов"));
    }
    Ok(())
}
fn value(line: &ConfigLine) -> Result<String> {
    arity(line, 2)?;
    if line.words[1].is_empty() {
        return Err(invalid(line.number, "пустое значение"));
    }
    Ok(line.words[1].clone())
}
fn set_once<T>(slot: &mut Option<T>, value: T, line: usize) -> Result<()> {
    if slot.is_some() {
        return Err(invalid(line, "повтор поля"));
    }
    *slot = Some(value);
    Ok(())
}
fn number(word: &str, line: usize) -> Result<f32> {
    word.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid(line, "неверное конечное число"))
}
fn boolean(word: &str, line: usize) -> Result<bool> {
    if word.eq_ignore_ascii_case("true") {
        Ok(true)
    } else if word.eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        Err(invalid(line, "ожидается true/false"))
    }
}

impl MissionDefinition {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let lines = config_lines(bytes)?;
        if lines.len() > 100_000 {
            return Err(invalid(0, "слишком много команд"));
        }
        let mut result = Self {
            map: String::new(),
            ambient: None,
            initial_view: None,
            players: Vec::new(),
            alliances: Vec::new(),
            ai: Vec::new(),
            zones: Vec::new(),
            objectives: Vec::new(),
            objective_links: Vec::new(),
            responses: Vec::new(),
            conversations: Vec::new(),
            globals: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut block = None;
        for line in lines {
            let w = &line.words;
            match w[0].as_str() {
                "CER_DEFINITION" => {
                    arity(&line, 1)?;
                    result.finish_block(block.take())?;
                    block = Some(Block::Response(ResponseBuilder {
                        line: line.number,
                        event: None,
                        subject: None,
                        actions: Vec::new(),
                        body: None,
                        extra: Vec::new(),
                    }));
                }
                "CONVERSE_SEQUENCE_DEFINITION" => {
                    arity(&line, 1)?;
                    result.finish_block(block.take())?;
                    block = Some(Block::Conversation(MissionConversation {
                        line: line.number,
                        id: String::new(),
                        elements: Vec::new(),
                        extra: Vec::new(),
                    }));
                }
                key if key.starts_with("CER_") => {
                    let Some(Block::Response(response)) = &mut block else {
                        return Err(invalid(line.number, "CER-поле вне CER_DEFINITION"));
                    };
                    match key {
                        "CER_FOR_TYPE" | "CER_FOR" => {
                            if response.body.is_some() || !response.actions.is_empty() {
                                return Err(invalid(line.number, "условие после тела CER"));
                            }
                            if key == "CER_FOR_TYPE" {
                                let event = CampaignEventType::parse(&value(&line)?);
                                if matches!(event, CampaignEventType::Unknown(_)) {
                                    result.warn(line.number, "неизвестный тип события сохранён");
                                }
                                set_once(&mut response.event, event, line.number)?;
                            } else {
                                set_once(&mut response.subject, value(&line)?, line.number)?;
                            }
                        }
                        "CER_BODY_DEFINITION" => {
                            arity(&line, 1)?;
                            if let Some(body) = response.body.take() {
                                response.actions.push(body.finish()?);
                            }
                            response.body = Some(ActionBuilder {
                                line: line.number,
                                ..Default::default()
                            });
                        }
                        "CER_BODY_TYPE"
                        | "CER_BODY_IDENTIFIER"
                        | "CER_BODY_MESSAGE"
                        | "CER_BODY_MS_DELAY" => {
                            let body = response.body.as_mut().ok_or_else(|| {
                                invalid(line.number, "поле тела без CER_BODY_DEFINITION")
                            })?;
                            match key {
                                "CER_BODY_TYPE" => {
                                    let kind = CampaignActionType::parse(&value(&line)?);
                                    if matches!(kind, CampaignActionType::Unknown(_)) {
                                        result
                                            .warn(line.number, "неизвестный тип действия сохранён");
                                    }
                                    set_once(&mut body.kind, kind, line.number)?;
                                }
                                "CER_BODY_IDENTIFIER" => {
                                    set_once(&mut body.identifier, value(&line)?, line.number)?
                                }
                                "CER_BODY_MESSAGE" => {
                                    arity(&line, 2)?;
                                    set_once(&mut body.message, w[1].clone(), line.number)?;
                                }
                                _ => {
                                    let delay = value(&line)?.parse::<u32>().map_err(|_| {
                                        invalid(line.number, "неверная задержка в миллисекундах")
                                    })?;
                                    set_once(&mut body.delay_ms, delay, line.number)?;
                                }
                            }
                        }
                        _ => {
                            result.warn(line.number, "неизвестное CER-поле сохранено");
                            if let Some(body) = &mut response.body {
                                body.extra.push(line);
                            } else {
                                response.extra.push(line);
                            }
                        }
                    }
                }
                key if key.starts_with("CONVERSE_") => {
                    let Some(Block::Conversation(conversation)) = &mut block else {
                        return Err(invalid(line.number, "CONVERSE-поле вне определения"));
                    };
                    match key {
                        "CONVERSE_IDENTIFIER" => {
                            if !conversation.id.is_empty() {
                                return Err(invalid(line.number, "повтор CONVERSE_IDENTIFIER"));
                            }
                            conversation.id = value(&line)?;
                        }
                        "CONVERSE_ELEMENT" => {
                            if w.len() < 3 || w[1].is_empty() {
                                return Err(invalid(line.number, "неверная реплика"));
                            }
                            normalize_path(&w[2])?;
                            conversation.elements.push(line);
                        }
                        _ => {
                            result.warn(line.number, "неизвестное CONVERSE-поле сохранено");
                            conversation.extra.push(line);
                        }
                    }
                }
                _ => {
                    result.finish_block(block.take())?;
                    match w[0].as_str() {
                        "SPECIFY_MTF" => {
                            if !result.map.is_empty() {
                                return Err(invalid(line.number, "повтор карты"));
                            }
                            result.map = normalize_path(&value(&line)?)?;
                        }
                        "SPECIFY_AMBIENT" => set_once(
                            &mut result.ambient,
                            normalize_path(&value(&line)?)?,
                            line.number,
                        )?,
                        "SPECIFY_VIEW_INITIAL" => {
                            arity(&line, 5)?;
                            let view = [
                                number(&w[1], line.number)?,
                                number(&w[2], line.number)?,
                                number(&w[3], line.number)?,
                                number(&w[4], line.number)?,
                            ];
                            set_once(&mut result.initial_view, view, line.number)?;
                        }
                        "SPM_PLAYER_MANAGER_ADD_PLAYER" | "MPM_PLAYER_MANAGER_ADD_PLAYER" => {
                            arity(&line, 4)?;
                            let id = w[3]
                                .parse::<i32>()
                                .ok()
                                .filter(|v| *v >= 0)
                                .ok_or_else(|| invalid(line.number, "неверный ID игрока"))?;
                            if w[1].is_empty() || w[2].is_empty() {
                                return Err(invalid(line.number, "пустое имя/тип игрока"));
                            }
                            result.players.push(MissionPlayer {
                                line: line.number,
                                multiplayer: w[0].starts_with("MPM"),
                                kind: w[1].clone(),
                                name: w[2].clone(),
                                id,
                            });
                        }
                        "SET_PLAYER_ALLIANCE" => {
                            if !(2..=3).contains(&w.len()) {
                                return Err(invalid(line.number, "неверный союз"));
                            }
                            result.alliances.push(line);
                        }
                        "CONFIGURE_PLAYER_AI" => {
                            arity(&line, 3)?;
                            result.ai.push(MissionAi {
                                line: line.number,
                                player: w[1].clone(),
                                file: normalize_path(&w[2])?,
                            });
                        }
                        "SET_ZONE" => {
                            arity(&line, 6)?;
                            let radius = number(&w[4], line.number)?;
                            if radius <= 0.0 || w[1].is_empty() || w[5].is_empty() {
                                return Err(invalid(line.number, "неверная зона"));
                            }
                            result.zones.push(MissionZone {
                                line: line.number,
                                id: w[1].clone(),
                                center: [number(&w[2], line.number)?, number(&w[3], line.number)?],
                                radius,
                                color: w[5].clone(),
                            });
                        }
                        "DEFINE_OBJECTIVE" => {
                            if !(4..=5).contains(&w.len()) || w[2].is_empty() || w[3].is_empty() {
                                return Err(invalid(line.number, "неверное задание"));
                            }
                            result.objectives.push(MissionObjective {
                                line: line.number,
                                id: w[2].clone(),
                                primary: boolean(&w[1], line.number)?,
                                description: w[3].clone(),
                                zone: w.get(4).cloned(),
                            });
                        }
                        "ADD_OBJECTIVE_ENABLE" => {
                            arity(&line, 3)?;
                            result.objective_links.push(MissionObjectiveLink {
                                line: line.number,
                                on_complete: w[1].clone(),
                                enable: w[2].clone(),
                            });
                        }
                        _ => result.globals.push(line),
                    }
                }
            }
        }
        result.finish_block(block)?;
        if result.map.is_empty() {
            return Err(invalid(0, "нет SPECIFY_MTF"));
        }
        result.validate()?;
        Ok(result)
    }

    fn warn(&mut self, line: usize, message: &str) {
        self.diagnostics.push(MissionDiagnostic {
            line,
            message: message.to_owned(),
        });
    }
    fn finish_block(&mut self, block: Option<Block>) -> Result<()> {
        match block {
            None => {}
            Some(Block::Response(mut response)) => {
                if let Some(body) = response.body {
                    response.actions.push(body.finish()?);
                }
                if response.event.is_none()
                    && response.subject.is_none()
                    && response.actions.is_empty()
                    && response.extra.is_empty()
                {
                    self.warn(response.line, "пустой CER_DEFINITION пропущен");
                    return Ok(());
                }
                let event = response
                    .event
                    .ok_or_else(|| invalid(response.line, "нет CER_FOR_TYPE"))?;
                if response.actions.is_empty()
                    || (event.requires_subject() && response.subject.is_none())
                {
                    return Err(invalid(
                        response.line,
                        "CER без действий или необходимого CER_FOR",
                    ));
                }
                self.responses.push(MissionResponse {
                    line: response.line,
                    event,
                    subject: response.subject,
                    actions: response.actions,
                    extra: response.extra,
                });
            }
            Some(Block::Conversation(conversation)) => {
                if conversation.id.is_empty() || conversation.elements.is_empty() {
                    return Err(invalid(conversation.line, "диалог без ID или реплик"));
                }
                self.conversations.push(conversation);
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        fn names<'a>(
            items: impl Iterator<Item = (&'a str, usize)>,
        ) -> Result<BTreeMap<String, usize>> {
            let mut names = BTreeMap::new();
            for (index, (id, line)) in items.enumerate() {
                if names.insert(id.to_ascii_lowercase(), index).is_some() {
                    return Err(invalid(line, "повтор имени"));
                }
            }
            Ok(names)
        }
        let zones = names(self.zones.iter().map(|z| (z.id.as_str(), z.line)))?;
        let objectives = names(self.objectives.iter().map(|o| (o.id.as_str(), o.line)))?;
        let conversations = names(self.conversations.iter().map(|c| (c.id.as_str(), c.line)))?;
        let mut players = BTreeSet::new();
        let mut player_ids = BTreeSet::new();
        let mut player_names = BTreeSet::new();
        for player in &self.players {
            if !player_names.insert((player.multiplayer, player.name.to_ascii_lowercase()))
                || !player_ids.insert((player.multiplayer, player.id))
            {
                return Err(invalid(player.line, "повтор игрока или его ID"));
            }
            players.insert(player.name.to_ascii_lowercase());
        }
        for ai in &self.ai {
            if !players.contains(&ai.player.to_ascii_lowercase()) {
                return Err(invalid(ai.line, "ИИ ссылается на неизвестного игрока"));
            }
        }
        for alliance in &self.alliances {
            if alliance.words[1..]
                .iter()
                .any(|name| !players.contains(&name.to_ascii_lowercase()))
            {
                return Err(invalid(alliance.number, "неизвестный игрок в союзе"));
            }
        }
        for objective in &self.objectives {
            if objective
                .zone
                .as_ref()
                .is_some_and(|z| !zones.contains_key(&z.to_ascii_lowercase()))
            {
                return Err(invalid(objective.line, "неизвестная зона задания"));
            }
        }
        let mut incoming = vec![0; objectives.len()];
        let mut edges = vec![Vec::new(); objectives.len()];
        let mut seen_edges = BTreeSet::new();
        for link in &self.objective_links {
            let from = objectives.get(&link.on_complete.to_ascii_lowercase());
            let to = objectives.get(&link.enable.to_ascii_lowercase());
            let (Some(&from), Some(&to)) = (from, to) else {
                return Err(invalid(link.line, "связь с неизвестным заданием"));
            };
            if !seen_edges.insert((from, to)) {
                return Err(invalid(link.line, "повтор связи заданий"));
            }
            incoming[to] += 1;
            edges[from].push(to);
        }
        let mut ready: VecDeque<_> = incoming
            .iter()
            .enumerate()
            .filter_map(|(i, n)| (*n == 0).then_some(i))
            .collect();
        let mut visited = 0;
        while let Some(index) = ready.pop_front() {
            visited += 1;
            for &next in &edges[index] {
                incoming[next] -= 1;
                if incoming[next] == 0 {
                    ready.push_back(next);
                }
            }
        }
        if visited != objectives.len() {
            return Err(invalid(0, "цикл включения заданий"));
        }
        for response in &self.responses {
            if response.event == CampaignEventType::AllPlayerInfantryKilled
                && !players.contains(&response.subject.as_deref().unwrap().to_ascii_lowercase())
            {
                return Err(invalid(
                    response.line,
                    "CER ссылается на неизвестного игрока",
                ));
            }
            for action in &response.actions {
                let id = action
                    .identifier
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let exists = match action.kind {
                    CampaignActionType::EnableObjective
                    | CampaignActionType::CompleteObjective
                    | CampaignActionType::CompleteObjectiveNoAudio => objectives.contains_key(&id),
                    CampaignActionType::TriggerConversation => conversations.contains_key(&id),
                    _ => true,
                };
                if !exists {
                    return Err(invalid(
                        action.line,
                        "неизвестное задание или диалог в действии",
                    ));
                }
            }
        }
        Ok(())
    }
}
