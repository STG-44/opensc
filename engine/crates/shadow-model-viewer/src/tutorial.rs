//! Ограниченный CER и проверенный по EXE обработчик спасения Джека.
//! Для прочих событий сохранено ограничение одной ненулевой задержки.
use crate::player;
use crate::ui_controls::{ButtonVisualState, UiButton};
use bevy::{prelude::*, text::FontSize};
use shadow_formats::{
    CampaignActionType as Action, CampaignEventType as Event, HelpDefinitions, MissionDefinition,
    MissionResponse,
};
use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
};

/// Только проверенная последовательность Jack допускает несколько полей MS_DELAY.
pub(crate) fn rescue_response(r: &MissionResponse) -> bool {
    r.event == Event::ObjectInConverseRange
        && r.subject
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("Jack"))
        && r.actions.len() == 5
        && r.actions
            .iter()
            .zip([
                (
                    Action::TriggerConversation,
                    Some("beecher_reporting_for_duty"),
                ),
                (Action::CompleteObjectiveNoAudio, Some("obj_rescue_beecher")),
                (Action::GiveLocalControl, Some("Jack")),
                (Action::ShowContextHelp, Some("help_combat_18")),
                (Action::RemoveResponse, None),
            ])
            .all(|(a, (kind, id))| {
                a.kind == kind
                    && a.identifier.as_deref().map(str::to_ascii_lowercase)
                        == id.map(str::to_ascii_lowercase)
            })
}
/// Поддержан только исходный успешный выход обучения; прочие EndCampaign ещё не исполняются.
pub(crate) fn extraction_response(r: &MissionResponse) -> bool {
    r.event == Event::SignalFlarePopped
        && r.subject.is_none()
        && r.extra.is_empty()
        && r.actions.len() == 1
        && r.actions[0].kind == Action::EndCampaign
        && r.actions[0].extra.is_empty()
        && r.actions[0]
            .identifier
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case("EOC_RETURN_TO_MAIN_MENU"))
}
#[derive(Clone)]
pub struct ExternalAction {
    pub kind: Action,
    pub id: String,
}
#[derive(Clone)]
struct Pending {
    due: f64,
    action: Action,
    id: Option<String>,
}
#[derive(Resource)]
pub struct Tutorial {
    mission: MissionDefinition,
    helps: HelpDefinitions,
    removed: BTreeSet<usize>,
    pending: VecDeque<Pending>,
    pub external: VecDeque<ExternalAction>,
    pub end_campaign: Option<String>,
    pub active: Option<String>,
    pub objectives: BTreeSet<String>,
    pub completed: BTreeSet<String>,
    pub pulse: bool,
    pub clock: f64,
    pub events: Vec<String>,
    pub shown: Vec<String>,
    pub unsupported: Vec<usize>,
    // Закрытие окна блокирует ввод мира до следующего кадра.
    pub consumed_input: bool,
}
impl Tutorial {
    pub fn new(mission: MissionDefinition, helps: HelpDefinitions) -> Result<Self, Box<dyn Error>> {
        for response in &mission.responses {
            for action in &response.actions {
                if action.kind == Action::ShowContextHelp
                    && helps
                        .get(action.identifier.as_deref().unwrap_or(""))
                        .is_none()
                {
                    return Err(format!("не найдена подсказка в строке {}", action.line).into());
                }
            }
        }
        let unsupported = mission
            .responses
            .iter()
            .filter(|r| {
                !r.extra.is_empty()
                    || (!rescue_response(r)
                        && r.actions
                            .iter()
                            .filter(|a| a.delay_ms.unwrap_or(0) > 0)
                            .count()
                            > 1)
                    || r.actions.iter().any(|a| {
                        !a.extra.is_empty()
                            || !(extraction_response(r) && a.kind == Action::EndCampaign)
                                && !(rescue_response(r)
                                    && matches!(
                                        a.kind,
                                        Action::TriggerConversation | Action::GiveLocalControl
                                    ))
                                && !matches!(
                                    a.kind,
                                    Action::EnableObjective
                                        | Action::CompleteObjective
                                        | Action::CompleteObjectiveNoAudio
                                        | Action::ShowContextHelp
                                        | Action::SetInventoryPulse
                                        | Action::RemoveResponse
                                )
                    })
            })
            .map(|r| r.line)
            .collect();
        let mut result = Self {
            mission,
            helps,
            removed: BTreeSet::new(),
            pending: VecDeque::new(),
            external: VecDeque::new(),
            end_campaign: None,
            active: None,
            objectives: BTreeSet::new(),
            completed: BTreeSet::new(),
            pulse: false,
            clock: 0.0,
            events: Vec::new(),
            shown: Vec::new(),
            unsupported,
            consumed_input: false,
        };
        result.fire(Event::Immediate, "");
        result.tick(0.0, false);
        Ok(result)
    }
    pub fn pausing(&self) -> bool {
        self.active
            .as_deref()
            .and_then(|id| self.helps.get(id))
            .is_some_and(|h| h.pause_game)
    }
    pub fn fire(&mut self, event: Event, subject: &str) {
        self.events.push(format!("{}:{subject}", event.as_str()));
        for response in &self.mission.responses {
            if response.event != event
                || self.removed.contains(&response.line)
                || self.unsupported.contains(&response.line)
            {
                continue;
            }
            if response.subject.as_deref().is_some_and(|s| {
                !s.eq_ignore_ascii_case("#any#") && !s.eq_ignore_ascii_case(subject)
            }) {
                continue;
            }
            for action in &response.actions {
                if action.kind == Action::RemoveResponse {
                    // Удаление подписки не уничтожает уже назначенную подсказку.
                    self.removed.insert(response.line);
                } else {
                    self.pending.push_back(Pending {
                        due: self.clock
                            + if rescue_response(response)
                                && matches!(
                                    action.kind,
                                    Action::TriggerConversation
                                        | Action::CompleteObjectiveNoAudio
                                        | Action::GiveLocalControl
                                )
                            {
                                // 0x449505/0x449611/0x449781: эти ветви не читают MS_DELAY (поле 0x10).
                                0.0
                            } else {
                                f64::from(action.delay_ms.unwrap_or(0)) / 1000.0
                            },
                        action: action.kind.clone(),
                        id: action.identifier.clone(),
                    });
                }
            }
        }
        // Стабильный порядок для совпадающих сроков, в том числе #any# и knife.
        self.pending
            .make_contiguous()
            .sort_by(|a, b| a.due.total_cmp(&b.due));
    }
    pub fn tick(&mut self, dt: f64, paused: bool) {
        if paused || self.pausing() || !dt.is_finite() || dt < 0.0 {
            return;
        }
        self.clock += dt;
        let mut retained = VecDeque::new();
        while let Some(action) = self.pending.pop_front() {
            if action.due > self.clock
                || (action.action == Action::ShowContextHelp && self.active.is_some())
            {
                retained.push_back(action);
                continue;
            }
            match action.action {
                Action::EnableObjective => {
                    self.objectives
                        .insert(action.id.unwrap().to_ascii_lowercase());
                }
                Action::CompleteObjective | Action::CompleteObjectiveNoAudio => {
                    let id = action.id.unwrap().to_ascii_lowercase();
                    if self.objectives.contains(&id) && self.completed.insert(id.clone()) {
                        for link in &self.mission.objective_links {
                            if link.on_complete.eq_ignore_ascii_case(&id) {
                                self.objectives.insert(link.enable.to_ascii_lowercase());
                            }
                        }
                    }
                }
                Action::SetInventoryPulse => {
                    self.pulse = action.id.is_some_and(|v| v.eq_ignore_ascii_case("true"));
                }
                Action::ShowContextHelp => {
                    let id = action.id.unwrap();
                    self.shown.push(id.clone());
                    self.active = Some(id);
                }
                Action::EndCampaign => {
                    self.end_campaign = action.id.map(|id| id.to_ascii_uppercase());
                }
                Action::TriggerConversation | Action::GiveLocalControl => {
                    self.external.push_back(ExternalAction {
                        kind: action.action,
                        id: action.id.unwrap(),
                    });
                }
                _ => unreachable!("unsupported actions never enter the queue"),
            }
        }
        self.pending = retained;
    }
    pub fn close(&mut self) {
        if let Some(id) = self.active.take() {
            self.fire(Event::BodyWindowClosed, &id);
            self.consumed_input = true;
        }
    }
    pub fn restart(&mut self) {
        self.removed.clear();
        self.pending.clear();
        self.external.clear();
        self.end_campaign = None;
        self.active = None;
        self.objectives.clear();
        self.completed.clear();
        self.clock = 0.0;
        self.pulse = false;
        self.events.clear();
        self.shown.clear();
        self.fire(Event::Immediate, "");
        self.tick(0.0, false);
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"clock":self.clock,"active_help":self.active,"help_pauses_game":self.pausing(),
            "shown_help":self.shown,"events":self.events,"enabled_objectives":self.objectives,
            "inventory_pulse":self.pulse,"unsupported_response_lines":self.unsupported,
            "completed_objectives":self.completed,"external_actions_waiting":self.external.len(),"end_campaign":self.end_campaign,"rescue_delay_dispatch":"EXE action-specific: conversation/goal/control immediate; help delay from event","full_campaign_execution":false})
    }
}

#[derive(Component)]
pub struct HelpPanel;
#[derive(Component)]
pub struct HelpOk;

/// Соседние строки оригинала — абзацы, а не пиксельные переносы 1999 года.
fn body(help: &shadow_formats::HelpDefinition) -> String {
    let mut result = String::new();
    for line in &help.body {
        let text = line.words[1].trim();
        if text.is_empty() {
            result.push_str("\n\n");
        } else {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push(if text.starts_with("KP ") { '\n' } else { ' ' });
            }
            result.push_str(text);
        }
    }
    result
}
pub fn input(
    tutorial: Option<ResMut<Tutorial>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    buttons: Query<&ButtonVisualState, With<HelpOk>>,
) {
    let Some(mut tutorial) = tutorial else {
        return;
    };
    tutorial.consumed_input = false;
    if tutorial.active.is_some()
        && (keys.just_pressed(KeyCode::Enter)
            || keys.just_pressed(KeyCode::Escape)
            || (mouse.just_pressed(MouseButton::Left)
                && buttons.iter().any(|i| *i == ButtonVisualState::Pressed)))
    {
        tutorial.close();
        keys.reset(KeyCode::Enter);
        keys.reset(KeyCode::Escape);
    }
}
pub fn advance(
    tutorial: Option<ResMut<Tutorial>>,
    actor: Option<Res<player::Player>>,
    inventory: Option<Res<crate::inventory::Inventory>>,
    time: Res<Time>,
) {
    if let Some(mut t) = tutorial {
        t.tick(
            time.delta_secs_f64(),
            actor.is_some_and(|p| p.playback.paused) || inventory.is_some_and(|inv| inv.open),
        );
    }
}
#[allow(clippy::too_many_arguments)]
pub fn ui(
    mut commands: Commands,
    tutorial: Option<Res<Tutorial>>,
    panels: Query<Entity, With<HelpPanel>>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
    window: Single<&Window>,
    mut fonts: ResMut<Assets<Font>>,
    mut previous: Local<Option<(Option<String>, u32, u32)>>,
) {
    let Some(t) = tutorial else {
        return;
    };
    let state = (
        t.active.clone(),
        window.width() as u32,
        window.height() as u32,
    );
    if previous.as_ref() == Some(&state) {
        return;
    }
    *previous = Some(state);
    for panel in &panels {
        commands.entity(panel).despawn();
    }
    let Some(help) = t.active.as_deref().and_then(|id| t.helps.get(id)) else {
        return;
    };
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    let mut content = body(help);
    if help.id.eq_ignore_ascii_case("help_combat_4") {
        content.push_str("\n\nПодбери BDA с золотым кольцом. Space открывает инвентарь; перетащи BDA в основной слот.");
    }
    if help.id.eq_ignore_ascii_case("help_combat_6") {
        content
            .push_str("\n\nПодбери гранату возле второго часового. Она займёт дополнительный слот. ПКМ по третьему часовому — бросок; Shift+ПКМ — бросок в точку.");
    }
    if help.id.eq_ignore_ascii_case("help_combat_8") {
        content.push_str("\n\nПодбери кусачки возле третьего часового. ЛКМ по сетке — разрезать, повторный ЛКМ — пролезть через проход.");
    }
    if help.id.eq_ignore_ascii_case("help_combat_10") {
        content.push_str("\n\nПроход открыт: повторный ЛКМ по сетке запускает пролезание. S/C/P меняют стойку. Подбери UZI за проходом и перетащи его в основной слот через Space.");
    }
    let scale = window.height() / 900.0;
    let dimensions = help.dimensions.unwrap_or([500, 200]);
    let width = (dimensions[0] as f32 * scale).min(window.width() - 32.0);
    let overlay = commands
        .spawn((
            player::SceneEntity,
            HelpPanel,
            UiTargetCamera(*camera),
            GlobalZIndex(500),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: px(window.height() - 195.0 * scale),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .id();
    let mut position = Node {
        width: px(width),
        min_height: px(dimensions[1] as f32 * scale),
        padding: UiRect::all(px(12.0 * scale)),
        border: UiRect::all(px(1)),
        row_gap: px(8.0 * scale),
        flex_direction: FlexDirection::Column,
        ..default()
    };
    if let Some([x, y]) = help.position {
        position.position_type = PositionType::Absolute;
        position.left = px((x as f32 * scale).clamp(0.0, window.width() - width));
        position.top = px((y as f32 * scale).max(0.0));
    }
    let panel = commands
        .spawn((
            position,
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
            BorderColor::all(Color::srgb(0.3, 0.9, 0.05)),
            ChildOf(overlay),
        ))
        .id();
    for (text, size, color) in [
        (
            format!(
                "Обучение · {}",
                if help.pause_game {
                    "пауза"
                } else {
                    "управление камерой"
                }
            ),
            17.0,
            Color::srgb(0.45, 0.85, 0.25),
        ),
        (content, 16.0, Color::srgb(0.94, 0.94, 0.87)),
    ] {
        commands.spawn((
            Text::new(text),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px((size * scale).max(12.0)),
                ..default()
            },
            TextColor(color),
            ChildOf(panel),
        ));
    }
    let button = commands
        .spawn((
            UiButton,
            HelpOk,
            Node {
                padding: UiRect::axes(px(28), px(8)),
                align_self: AlignSelf::FlexEnd,
                ..default()
            },
            BackgroundColor(Color::NONE),
            ChildOf(panel),
        ))
        .id();
    commands.spawn((
        Text::new("OK · Enter"),
        TextFont {
            font: font.into(),
            font_size: FontSize::Px(17.0),
            ..default()
        },
        ChildOf(button),
    ));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn tutorial() -> Tutorial {
        let mission = MissionDefinition::from_bytes(b"SPECIFY_MTF training.mtf\nDEFINE_OBJECTIVE true first \"Guard\"\nCER_DEFINITION\nCER_FOR_TYPE CET_IMMEDIATE\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_ENABLE_OBJECTIVE\nCER_BODY_IDENTIFIER first\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER welcome\nCER_BODY_MS_DELAY 3000\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_PICKED_UP\nCER_FOR knife\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER camera\nCER_BODY_MS_DELAY 2000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\nCER_DEFINITION\nCER_FOR_TYPE CET_BODY_WINDOW_CLOSED\nCER_FOR camera\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER guard\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\n").unwrap();
        let helps = HelpDefinitions::from_bytes(b"HELP_DEF\nHELP_ID welcome\nHELP_CONTEXT\nHELP_BODY \"Hello\"\nHELP_DEF\nHELP_ID camera\nHELP_CONTEXT\nHELP_DO_NOT_PAUSE_GAME\nHELP_BODY \"Camera\"\nHELP_DEF\nHELP_ID guard\nHELP_CONTEXT\nHELP_BODY \"Guard\"\n").unwrap();
        Tutorial::new(mission, helps).unwrap()
    }
    #[test]
    fn delays_pause_flags_and_one_shot_responses_follow_events() {
        let mut t = tutorial();
        assert!(t.objectives.contains("first"));
        t.tick(2.9, false);
        assert!(t.active.is_none());
        t.tick(0.1, false);
        assert_eq!(t.active.as_deref(), Some("welcome"));
        t.tick(10.0, false);
        assert_eq!(t.clock, 3.0);
        t.close();
        t.fire(Event::ObjectPickedUp, "KNIFE");
        t.fire(Event::ObjectPickedUp, "knife");
        t.tick(1.9, false);
        assert!(t.active.is_none());
        t.tick(0.1, false);
        assert_eq!(t.active.as_deref(), Some("camera"));
        assert!(!t.pausing());
        t.tick(1.0, false);
        assert_eq!(t.clock, 6.0);
        t.close();
        t.tick(0.9, false);
        assert!(t.active.is_none());
        t.tick(0.1, false);
        assert_eq!(t.active.as_deref(), Some("guard"));
        assert_eq!(t.shown, ["welcome", "camera", "guard"]);
        t.restart();
        assert!(t.active.is_none());
        assert_eq!(t.clock, 0.0);
        assert_eq!(t.events.len(), 1);
    }
    #[test]
    fn pause_and_invalid_deltas_do_not_advance_script_time() {
        let mut t = tutorial();
        for (dt, paused) in [(10.0, true), (f64::NAN, false), (-1.0, false)] {
            t.tick(dt, paused);
        }
        assert_eq!(t.clock, 0.0);
    }
    #[test]
    fn closing_help_consumes_escape_and_does_not_open_pause_behind_it() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        let mut t = tutorial();
        t.tick(3.0, false);
        app.insert_resource(t);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(app.world().resource::<Tutorial>().active.is_none());
        assert!(app.world().resource::<Tutorial>().consumed_input);
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::Escape)
        );
    }
}
