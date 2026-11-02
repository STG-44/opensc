//! Окна фронтенда. Геометрия и текст рисуются заново, без растровых надписей.
//! Незавершённые системы обозначены прямо, без фиктивного запуска кампании/сети.
use super::*;

#[derive(Resource, Default)]
pub(super) struct Navigation {
    pub dirty: bool,
    pub options: usize,
    pub campaign: usize,
    pub mission: usize,
    pub quit_return: Option<Screen>,
}
#[derive(Component)]
pub(super) struct Panel;
#[derive(Component)]
pub(super) struct BaseVolume(f32);
#[derive(Component)]
pub(super) struct ScrollText;

pub(super) fn scroll_text(
    window: Single<&Window>,
    capture: Res<Capture>,
    wheel: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    keys: Res<ButtonInput<KeyCode>>,
    mut regions: Query<(&ComputedNode, &UiGlobalTransform, &mut ScrollPosition), With<ScrollText>>,
) {
    if capture.ignore_input {
        return;
    }
    for (node, transform, mut position) in &mut regions {
        let hovered = window
            .cursor_position()
            .is_some_and(|p| node.contains_point(*transform, p * window.scale_factor()));
        let step = if matches!(wheel.unit, bevy::input::mouse::MouseScrollUnit::Line) {
            35.
        } else {
            1.
        };
        let amount = if keys.just_pressed(KeyCode::PageDown) {
            90.
        } else if keys.just_pressed(KeyCode::PageUp) {
            -90.
        } else if hovered {
            -wheel.delta.y * step
        } else {
            0.
        };
        if amount != 0. {
            position.0.y = (position.0.y + amount).clamp(
                0.,
                ((node.content_size.y - node.size.y) * node.inverse_scale_factor).max(0.),
            );
        }
    }
}

pub(super) fn enter(
    mut commands: Commands,
    state: Res<State<Screen>>,
    context: Res<Context>,
    assets: Res<MenuAssets>,
    camera: Single<Entity, With<MenuCamera>>,
    mut navigation: ResMut<Navigation>,
    mut focus: ResMut<Focus>,
) {
    navigation.dirty = false;
    let canvas = root(
        &mut commands,
        *state.get(),
        *camera,
        &assets,
        context.frame,
        true,
    );
    draw(
        &mut commands,
        canvas,
        *state.get(),
        &context,
        &assets,
        &navigation,
        &mut focus,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn refresh(
    mut commands: Commands,
    state: Res<State<Screen>>,
    context: Res<Context>,
    assets: Res<MenuAssets>,
    mut navigation: ResMut<Navigation>,
    roots: Query<Entity, With<Canvas>>,
    panels: Query<Entity, With<Panel>>,
    mut focus: ResMut<Focus>,
) {
    if !navigation.dirty
        || !matches!(
            state.get(),
            Screen::Options
                | Screen::NewGame
                | Screen::LoadGame
                | Screen::Multiplayer
                | Screen::Quit
        )
    {
        return;
    }
    navigation.dirty = false;
    for entity in &panels {
        commands.entity(entity).despawn();
    }
    if let Some(canvas) = roots.iter().next() {
        draw(
            &mut commands,
            canvas,
            *state.get(),
            &context,
            &assets,
            &navigation,
            &mut focus,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn label(
    commands: &mut Commands,
    parent: Entity,
    assets: &MenuAssets,
    position: (f32, f32),
    size: (f32, f32),
    labels: (&str, &str),
    font: f32,
) -> Entity {
    text(
        commands,
        parent,
        assets,
        Vec2::new(position.0, position.1),
        Vec2::new(size.0, size.1),
        labels.0,
        labels.1,
        font,
        None,
    )
}
#[allow(clippy::too_many_arguments)]
fn control(
    commands: &mut Commands,
    parent: Entity,
    assets: &MenuAssets,
    position: (f32, f32),
    width: f32,
    labels: (&str, &str),
    action: Action,
    order: usize,
) -> Entity {
    let entity = button(
        commands,
        parent,
        assets,
        Vec2::new(position.0, position.1),
        Vec2::new(width, 31.),
        Vec2::new(9., 5.),
        labels.0,
        labels.1,
        Some(action),
        order,
        false,
    );
    commands
        .entity(entity)
        .insert((BorderColor::all(Color::srgb(0.34, 0.35, 0.18)),));
    // Рамка является геометрией UI и сохраняет резкость при смене разрешения.
    commands
        .entity(entity)
        .entry::<Node>()
        .and_modify(|mut node| node.border = UiRect::all(px(1.)));
    entity
}
fn panel(
    commands: &mut Commands,
    canvas: Entity,
    assets: &MenuAssets,
    context: &Context,
    title: (&str, &str),
) -> Entity {
    let entity = commands
        .spawn((
            Panel,
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(px(2.)),
                ..default()
            },
            Placement {
                position: Vec2::new((context.layout.size[0] - 650.) * 0.5, 28.),
                size: Vec2::new(650., 424.),
            },
            Anchor::Center,
            BackgroundColor(Color::srgba(0.035, 0.05, 0.035, 0.98)),
            BorderColor::all(Color::srgb(0.39, 0.40, 0.19)),
            ChildOf(canvas),
        ))
        .id();
    let heading = label(
        commands,
        entity,
        assets,
        (22., 14.),
        (606., 36.),
        title,
        22.,
    );
    commands.entity(heading).insert((
        TextColor(Color::srgb(1., 0.8, 0.23)),
        TextFont {
            font: assets.title_font.clone().into(),
            font_size: FontSize::Px(22.),
            ..default()
        },
    ));
    entity
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    commands: &mut Commands,
    canvas: Entity,
    screen: Screen,
    context: &Context,
    assets: &MenuAssets,
    navigation: &Navigation,
    focus: &mut Focus,
) {
    let title = match screen {
        Screen::Options => ("НАСТРОЙКИ", "OPTIONS"),
        Screen::NewGame => ("НОВАЯ ИГРА", "NEW GAME"),
        Screen::LoadGame => ("ЗАГРУЗИТЬ ИГРУ", "LOAD GAME"),
        Screen::Multiplayer => ("СЕТЕВАЯ ИГРА", "MULTIPLAYER"),
        Screen::Quit => ("ВЫХОД ИЗ ИГРЫ", "EXIT GAME"),
        Screen::Pause => ("ПАУЗА", "PAUSED"),
        Screen::Loading => ("ЗАГРУЗКА ОБУЧЕНИЯ", "LOADING TRAINING"),
        _ => return,
    };
    let body = panel(commands, canvas, assets, context, title);
    focus.1 = false;
    match screen {
        Screen::Options => options(commands, body, context, assets, navigation, focus),
        Screen::NewGame => campaign(commands, body, context, assets, navigation, focus),
        Screen::LoadGame => {
            focus.0 = Some(Action::RefreshSaves);
            label(
                commands,
                body,
                assets,
                (24., 66.),
                (600., 38.),
                ("СОХРАНЁННЫЕ ИГРЫ", "SAVED GAMES"),
                16.,
            );
            label(
                commands,
                body,
                assets,
                (24., 120.),
                (600., 150.),
                (
                    "Сохранений нового движка пока нет.\n\nЗапись и восстановление состояния миссии ещё не реализованы. Сохранения старого EXE пока несовместимы с OpenSC.",
                    "There are no OpenSC saves yet.\n\nMission state saving and loading are not implemented. Saves from the original EXE are not yet compatible.",
                ),
                14.,
            );
            control(
                commands,
                body,
                assets,
                (24., 292.),
                245.,
                ("ОБНОВИТЬ СПИСОК", "REFRESH"),
                Action::RefreshSaves,
                0,
            );
            control(
                commands,
                body,
                assets,
                (380., 366.),
                245.,
                ("НАЗАД", "BACK"),
                Action::Back,
                1,
            );
        }
        Screen::Multiplayer => {
            focus.0 = Some(Action::Back);
            label(
                commands,
                body,
                assets,
                (24., 65.),
                (600., 36.),
                (
                    "TCP/IP · ЛОКАЛЬНАЯ СЕТЬ И ИНТЕРНЕТ",
                    "TCP/IP · LAN AND INTERNET",
                ),
                16.,
            );
            label(
                commands,
                body,
                assets,
                (24., 120.),
                (600., 190.),
                (
                    "Сетевая игра нового движка ещё не реализована.\n\nСоздание сервера, поиск сеансов и подключение появятся вместе с сетевой синхронизацией миссий. Сервер обновлений лаунчера уже работает отдельно.",
                    "OpenSC multiplayer is not implemented yet.\n\nHosting, session discovery and joining require mission synchronization. The launcher update server already runs separately.",
                ),
                14.,
            );
            control(
                commands,
                body,
                assets,
                (380., 366.),
                245.,
                ("НАЗАД", "BACK"),
                Action::Back,
                0,
            );
        }
        Screen::Quit => {
            focus.0 = Some(Action::Back);
            label(
                commands,
                body,
                assets,
                (24., 106.),
                (600., 150.),
                (
                    "Выйти из Shadow Company?\n\nТекущее прохождение обучения не сохраняется.",
                    "Exit Shadow Company?\n\nThe current training session will not be saved.",
                ),
                18.,
            );
            control(
                commands,
                body,
                assets,
                (24., 330.),
                284.,
                ("ВЫЙТИ", "EXIT"),
                Action::ConfirmExit,
                1,
            );
            control(
                commands,
                body,
                assets,
                (340., 330.),
                284.,
                ("ОТМЕНА", "CANCEL"),
                Action::Back,
                0,
            );
        }
        Screen::Pause => {
            focus.0 = Some(Action::Resume);
            for (i, (ru, en, action)) in [
                ("ПРОДОЛЖИТЬ", "RESUME", Action::Resume),
                ("НАСТРОЙКИ", "OPTIONS", Action::Options),
                ("ГЛАВНОЕ МЕНЮ", "MAIN MENU", Action::Main),
                ("ВЫЙТИ ИЗ ИГРЫ", "QUIT GAME", Action::Exit),
            ]
            .into_iter()
            .enumerate()
            {
                control(
                    commands,
                    body,
                    assets,
                    (100., 104. + i as f32 * 57.),
                    450.,
                    (ru, en),
                    action,
                    i,
                );
            }
        }
        Screen::Loading => {
            focus.0 = Some(Action::Back);
            label(
                commands,
                body,
                assets,
                (24., 126.),
                (600., 100.),
                (
                    "Подготовка карты, моделей и текстур…",
                    "Preparing terrain, models and textures…",
                ),
                18.,
            );
            control(
                commands,
                body,
                assets,
                (340., 330.),
                284.,
                ("ОТМЕНА", "CANCEL"),
                Action::Back,
                0,
            );
        }
        _ => {}
    }
    let notice = text(
        commands,
        body,
        assets,
        Vec2::new(24., 405.),
        Vec2::new(600., 16.),
        "",
        "",
        9.,
        Some(Action::Notice),
    );
    commands.entity(notice).insert(Pickable::IGNORE);
}

fn options(
    commands: &mut Commands,
    parent: Entity,
    _context: &Context,
    assets: &MenuAssets,
    navigation: &Navigation,
    focus: &mut Focus,
) {
    for (i, (ru, en, action)) in [
        ("ПАРАМЕТРЫ", "PREFERENCES", Action::DisplayTab),
        ("ЗВУК / ВИДЕО", "AUDIO / VIDEO", Action::AudioTab),
        ("МЫШЬ", "MOUSE", Action::MouseTab),
        ("КЛАВИШИ", "KEYBOARD", Action::KeyboardTab),
    ]
    .into_iter()
    .enumerate()
    {
        let entity = control(
            commands,
            parent,
            assets,
            (22. + i as f32 * 153., 59.),
            147.,
            (ru, en),
            action,
            i,
        );
        if i == navigation.options {
            commands
                .entity(entity)
                .insert(BorderColor::all(Color::srgb(0.94, 0.74, 0.24)));
        }
    }
    let actions: &[Action] = match navigation.options {
        0 => &[Action::Language, Action::TextSize, Action::Background],
        1 => &[Action::Window, Action::Resolution, Action::MasterVolume],
        2 => &[Action::CameraSpeed, Action::EdgeScroll],
        _ => &[],
    };
    focus.0 = actions.first().copied().or(Some(Action::KeyboardTab));
    for (i, &action) in actions.iter().enumerate() {
        control(
            commands,
            parent,
            assets,
            (24., 114. + i as f32 * 50.),
            600.,
            ("", ""),
            action,
            i + 4,
        );
    }
    match navigation.options {
        0 => {
            label(
                commands,
                parent,
                assets,
                (24., 276.),
                (600., 62.),
                (
                    "Изменения вступят в силу после применения.\nОтмена или Esc вернут прежние настройки.",
                    "Changes take effect after applying.\nCancel or Esc keeps your previous settings.",
                ),
                13.,
            );
        }
        1 => {
            label(
                commands,
                parent,
                assets,
                (24., 276.),
                (600., 62.),
                (
                    "Разрешение задаёт размер окна. Полный экран использует размер монитора. Громкость применяется к роликам и звукам игры.",
                    "Resolution sets the window size. Fullscreen uses the monitor size. Volume affects both movies and game sounds.",
                ),
                12.,
            );
        }
        2 => {
            label(
                commands,
                parent,
                assets,
                (24., 232.),
                (600., 100.),
                (
                    "ПКМ + движение мыши — перемещение камеры.\nКолесо — масштаб; ЛКМ — выбор и приказы.\nПрокрутка у краёв экрана включается отдельно.",
                    "Hold RMB and move the mouse to pan the camera.\nWheel: zoom; LMB: select and issue orders.\nScreen edge scrolling can be toggled separately.",
                ),
                13.,
            );
        }
        3 => {
            label(
                commands,
                parent,
                assets,
                (24., 105.),
                (600., 225.),
                (
                    "1 / 2   — выбрать бойца       H — остановить\nS / C / P   — стоять / сидеть / лежать\nПробел   — инвентарь       T — слежение камерой\nNum 4 / 6 / 8 / 5   — перемещение камеры\nNum 1 / 3 / 7 / 9 — поворот; Num 0 / 2 — наклон\nNum + / −   — масштаб       E — выйти из машины\nСтрелки   — управление машиной\nEsc   — пауза / назад; Enter — выбор\n\nПереназначение клавиш ещё не реализовано.",
                    "1 / 2   — select mercenary       H — stop\nS / C / P   — stand / crouch / prone\nSpace   — inventory       T — track camera\nNum 4 / 6 / 8 / 5   — pan camera\nNum 1 / 3 / 7 / 9 — rotate; Num 0 / 2 — tilt\nNum + / −   — zoom       E — leave vehicle\nArrows   — drive vehicle\nEsc   — pause / back; Enter — select\n\nKey rebinding is not implemented yet.",
                ),
                12.,
            );
        }
        _ => {}
    }
    control(
        commands,
        parent,
        assets,
        (24., 366.),
        284.,
        ("ПРИМЕНИТЬ", "APPLY"),
        Action::Apply,
        20,
    );
    control(
        commands,
        parent,
        assets,
        (340., 366.),
        284.,
        ("ОТМЕНА", "CANCEL"),
        Action::Back,
        21,
    );
}

#[derive(Debug)]
struct Mission {
    name: String,
    title: String,
    hidden: bool,
    mercs: Vec<String>,
    stock: Vec<(String, u32)>,
}
fn missions(catalog: &AssetCatalog) -> Result<Vec<Mission>, String> {
    parse_missions(
        &catalog
            .read("config/campaign_defs.txt")
            .map_err(|e| e.to_string())?,
    )
}
fn parse_missions(data: &[u8]) -> Result<Vec<Mission>, String> {
    let lines = shadow_formats::config_lines(data).map_err(|e| e.to_string())?;
    let mut list: Vec<Mission> = Vec::new();
    for line in lines {
        let words = &line.words;
        if words[0] == "BASENAME"
            && let Some(name) = words.get(1)
        {
            // Имена каталогов из оригинала; произвольные пути не принимаются.
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("Неверное имя кампании / Invalid campaign name".into());
            }
            list.push(Mission {
                name: name.clone(),
                title: name.clone(),
                hidden: false,
                mercs: Vec::new(),
                stock: Vec::new(),
            });
        } else if let Some(last) = list.last_mut() {
            match words[0].as_str() {
                "TITLE" => last.title = words[1..].join(" "),
                "EXCLUDE_FROM_CAMPAIGN_TREE" => last.hidden = true,
                "PRE_ACTION" if words.get(1).is_some_and(|w| w == "ROSTER_MERC") => {
                    if let Some(name) = words.get(2)
                        && !last.mercs.contains(name)
                    {
                        last.mercs.push(name.clone());
                    }
                }
                "PRE_ACTION" if words.get(1).is_some_and(|w| w == "DEALER_STOCK_EXPLICIT") => {
                    if let (Some(name), Some(count)) = (
                        words.get(2),
                        words.get(3).and_then(|w| w.parse::<u32>().ok()),
                    ) {
                        last.stock.push((name.clone(), count));
                    }
                }
                _ => {}
            }
        }
    }
    list.retain(|m| !m.hidden);
    if list.is_empty() {
        return Err("Нет описаний кампании / No campaign definitions".into());
    }
    Ok(list)
}
pub(super) fn mission_count(catalog: &AssetCatalog) -> usize {
    missions(catalog).map_or(1, |m| m.len())
}

fn campaign(
    commands: &mut Commands,
    parent: Entity,
    context: &Context,
    assets: &MenuAssets,
    navigation: &Navigation,
    focus: &mut Focus,
) {
    for (i, (ru, en, action)) in [
        ("ПЛАНИРОВАНИЕ", "PLANNING", Action::BriefingTab),
        ("НАЁМНИКИ", "PERSONNEL", Action::TeamTab),
        ("СНАРЯЖЕНИЕ", "EQUIPMENT", Action::EquipmentTab),
    ]
    .into_iter()
    .enumerate()
    {
        let entity = control(
            commands,
            parent,
            assets,
            (22. + i as f32 * 204., 59.),
            198.,
            (ru, en),
            action,
            i,
        );
        if i == navigation.campaign {
            commands
                .entity(entity)
                .insert(BorderColor::all(Color::srgb(0.94, 0.74, 0.24)));
        }
    }
    focus.0 = Some(Action::NextMission);
    control(
        commands,
        parent,
        assets,
        (24., 101.),
        45.,
        ("‹", "‹"),
        Action::PreviousMission,
        3,
    );
    control(
        commands,
        parent,
        assets,
        (578., 101.),
        45.,
        ("›", "›"),
        Action::NextMission,
        4,
    );
    let result = (|| -> Result<(String, String), String> {
        let missions = missions(&context.catalog)?;
        let selected = &missions[navigation.mission % missions.len()];
        let path = format!("campaign/{0}/{0}.txt", selected.name);
        let mission = shadow_formats::MissionDefinition::from_bytes(
            &context.catalog.read(&path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let contents = match navigation.campaign {
            0 => {
                let descriptions = mission
                    .objectives
                    .iter()
                    .map(|o| format!("• {}", o.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                if descriptions.is_empty() {
                    format!("{}\n{}", selected.title, mission.map)
                } else {
                    descriptions
                }
            }
            1 => {
                let names = selected
                    .mercs
                    .iter()
                    .map(|name| {
                        player::profile_for(&context.catalog, name)
                            .map_or_else(|_| name.clone(), |(_, p)| p.display_name())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if names.is_empty() {
                    if context.saved.english {
                        "No new mercenaries are specified for this mission."
                    } else {
                        "Дополнение состава для этой миссии не задано."
                    }
                    .into()
                } else {
                    names
                }
            }
            _ => {
                let title = if context.saved.english {
                    "Mission shop catalogue (view only):"
                } else {
                    "Ассортимент магазина миссии (просмотр):"
                };
                let items = selected
                    .stock
                    .iter()
                    .map(|(name, count)| format!("{name} × {count}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                if items.is_empty() {
                    if context.saved.english {
                        "No new shop stock is specified for this mission."
                    } else {
                        "Дополнение магазина для этой миссии не задано."
                    }
                    .into()
                } else {
                    format!("{title}\n\n{items}")
                }
            }
        };
        Ok((selected.title.clone(), contents))
    })();
    let (heading, contents) = result.unwrap_or_else(|error| {
        warn!("Unable to read campaign data: {error}");
        if context.saved.english {
            ("Resource error".into(), "Could not read campaign data. Check the original game files. See the log for details.".into())
        } else {
            ("Ошибка ресурсов".into(), error)
        }
    });
    label(
        commands,
        parent,
        assets,
        (82., 105.),
        (480., 34.),
        (&heading, &heading),
        16.,
    );
    label(
        commands,
        parent,
        assets,
        (400., 136.),
        (224., 14.),
        ("Колесо / PgUp / PgDn", "Wheel / PgUp / PgDn"),
        9.,
    );
    let viewport = commands
        .spawn((
            ScrollText,
            ScrollPosition::default(),
            Node {
                position_type: PositionType::Absolute,
                overflow: Overflow::scroll_y(),
                ..default()
            },
            Placement {
                position: Vec2::new(24., 153.),
                size: Vec2::new(600., 132.),
            },
            Anchor::Left,
            ChildOf(parent),
        ))
        .id();
    commands.spawn((
        Text::new(&contents),
        Caption {
            ru: contents.clone(),
            en: contents,
            size: 12.,
            action: None,
        },
        TextFont {
            font: assets.font.clone().into(),
            font_size: FontSize::Px(12.),
            ..default()
        },
        TextColor(Color::srgb(0.87, 0.87, 0.8)),
        Node {
            width: percent(100.),
            flex_shrink: 0.,
            ..default()
        },
        ChildOf(viewport),
    ));
    label(
        commands,
        parent,
        assets,
        (24., 292.),
        (600., 65.),
        (
            "Просмотр данных оригинала. Полное прохождение кампании, найм и магазин ещё не реализованы. Доступна учебная миссия.",
            "Original campaign data viewer. Campaign play, hiring and the shop are not implemented yet. The training mission is playable.",
        ),
        12.,
    );
    control(
        commands,
        parent,
        assets,
        (24., 366.),
        284.,
        ("НАЧАТЬ ОБУЧЕНИЕ", "START TRAINING"),
        Action::Training,
        5,
    );
    control(
        commands,
        parent,
        assets,
        (340., 366.),
        284.,
        ("НАЗАД", "BACK"),
        Action::Back,
        6,
    );
}

/// Не используем GlobalVolume: он не меняет уже запущенные звуки.
/// Запоминаем индивидуальную громкость каждого sink, чтобы сохранять баланс.
pub(super) fn apply_audio(
    mut commands: Commands,
    preferences: Res<Preferences>,
    mut sinks: Query<(Entity, &mut AudioSink, Option<&BaseVolume>)>,
) {
    for (entity, mut sink, base) in &mut sinks {
        let value = base.map_or_else(|| sink.volume().to_linear(), |v| v.0);
        if base.is_none() {
            commands.entity(entity).insert(BaseVolume(value));
        }
        let target = bevy::audio::Volume::Linear(value * preferences.master_volume);
        if sink.volume() != target {
            sink.set_volume(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn campaign_browser_excludes_training_and_uses_roster_and_shop_actions() {
        let data = b"CAMPAIGN_DEF\nBASENAME training\nTITLE Training\nEXCLUDE_FROM_CAMPAIGN_TREE\nPRE_ACTION ROSTER_MERC John\nCAMPAIGN_DEF\nBASENAME angola_tutorial\nTITLE \"Left for Dead\"\nPRE_ACTION ROSTER_MERC John\nPRE_ACTION ROSTER_MERC Jack\nPRE_ACTION ROSTER_MERC John\nPRE_ACTION DEALER_STOCK_EXPLICIT \"Flak Jacket\" 2\nPRE_ACTION MPM_DEALER_STOCK_EXPLICIT Uzi 10\nPOST_ACTION ROSTER_MERC WrongStage\n";
        let missions = parse_missions(data).unwrap();
        assert_eq!(missions.len(), 1);
        assert_eq!(missions[0].name, "angola_tutorial");
        assert_eq!(missions[0].title, "Left for Dead");
        assert_eq!(missions[0].mercs, ["John", "Jack"]);
        assert_eq!(missions[0].stock, [("Flak Jacket".into(), 2)]);
        assert!(parse_missions(b"BASENAME ../escape").is_err());
    }
}
