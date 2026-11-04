//! Оригинальная игровая панель, с независимыми от соотношения сторон краями.
use crate::{
    hud_layout::{self, Layout},
    inventory::Inventory,
    party::Party,
    player, radar, selection, training,
    tutorial::Tutorial,
    ui_controls::{ButtonVisualState, UiButton},
};
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, ConfigLine, config_lines};
use std::{collections::BTreeMap, error::Error};

const DEFINITION: &str = "config/window_bases/bottombar_640x480.txt";
const HEIGHT: f32 = 195.0;
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub scale: f32,
    pub screen: Vec2,
}
impl Fit {
    pub fn new(screen: Vec2) -> Self {
        let screen = screen.max(Vec2::ONE);
        Self {
            scale: screen.min_element() / 900.0,
            screen,
        }
    }
    pub fn origin(self) -> Vec2 {
        Vec2::new(0.0, self.screen.y - HEIGHT * self.scale)
    }
    pub fn rect(self, r: Rect) -> Rect {
        Rect::from_corners(
            r.min * self.scale + self.origin(),
            r.max * self.scale + self.origin(),
        )
    }
    pub fn node(self, r: Rect) -> Node {
        Node {
            position_type: PositionType::Absolute,
            left: px(r.min.x * self.scale),
            top: px(r.min.y * self.scale),
            width: px(r.width() * self.scale),
            height: px(r.height() * self.scale),
            ..default()
        }
    }
}
struct Texture {
    image: Option<Image>,
    handle: Option<Handle<Image>>,
    size: Vec2,
}
#[derive(Resource)]
pub struct Hud {
    lines: Vec<ConfigLine>,
    textures: BTreeMap<String, Texture>,
    frames: Vec<Rect>,
    pub layout: Layout,
    pub fit: Fit,
    pub diagnostics: bool,
    pub(crate) radar: radar::Radar,
    injected: Vec<KeyCode>,
    pub command_posture: Option<crate::stance::Posture>,
    source: serde_json::Value,
    pub inventory: Option<crate::inventory_ui::Appearance>,
}
#[derive(Component)]
pub struct Diagnostic;
#[derive(Component)]
pub(crate) struct Root;
#[derive(Component)]
pub(crate) enum Piece {
    Quad(usize),
    Block(usize),
    Advice(usize),
    Clock,
    Tooltip,
}
#[derive(Component)]
pub(crate) struct CommandButton(pub String);
impl Hud {
    pub fn prepare(catalog: &AssetCatalog) -> Result<Self, Box<dyn Error>> {
        let lines = config_lines(&catalog.read(DEFINITION)?)?;
        let layout = hud_layout::parse(&lines, Vec2::new(1600.0, 900.0))?;
        let frames =
            hud_layout::button_frames(&config_lines(&catalog.read("config/image_defs.txt")?)?)?;
        if layout
            .advice
            .iter()
            .any(|a| a.frame.is_some_and(|f| f >= frames.len()))
        {
            return Err("нет кадра кнопки HUD".into());
        }
        let mut textures = BTreeMap::new();
        let mut sources = Vec::new();
        for (name, path) in [
            ("simu1_ck.bmp", "textures/interface/simu1_ck.bmp"),
            ("simu3_ck.bmp", "textures/interface/simu3_ck.bmp"),
            ("radar", "textures/terrain/training/training_radar_ck.jpg"),
        ] {
            let mut image = crate::asset_mesh::image(catalog, path)?.image;
            image.sampler = if name.ends_with("_ck.bmp") {
                bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    ..bevy::image::ImageSamplerDescriptor::linear()
                })
            } else {
                bevy::image::ImageSampler::linear()
            };
            // В пустых чёрных областях этих двух атласов есть шум RGB 1/2.
            // Он не должен превращаться в крапинки поверх мира. Другие текстуры не меняем.
            let mut noise_removed = 0;
            if name.ends_with("_ck.bmp")
                && let Some(data) = &mut image.data
            {
                for pixel in data.as_chunks_mut::<4>().0 {
                    if pixel[3] == 255 && pixel[..3].iter().all(|v| *v <= 2) {
                        pixel[3] = 0;
                        noise_removed += 1;
                    }
                }
            }
            let size = image.size().as_vec2();
            sources.push(serde_json::json!({"path":path,"origin":catalog.source_label(path)?,"size":size.to_array(),"colorkey_noise_removed":noise_removed}));
            textures.insert(
                name.into(),
                Texture {
                    image: Some(image),
                    handle: None,
                    size,
                },
            );
        }
        if layout
            .quads
            .iter()
            .any(|q| q.uv.is_some() && !textures.contains_key(&q.texture))
        {
            return Err("неподдержанная текстура HUD".into());
        }
        let source = serde_json::json!({"definition":DEFINITION,"origin":catalog.source_label(DEFINITION)?,"textures":sources,"geometry_from_original":true,"command_frames":frames.len(),"minimap_fog_of_war":true,"disabled_commands":["overwatch","reload","formations","clock_speed"]});
        Ok(Self {
            lines,
            textures,
            frames,
            layout,
            fit: Fit::new(Vec2::new(1280.0, 720.0)),
            diagnostics: false,
            radar: radar::Radar::new(),
            injected: Vec::new(),
            command_posture: None,
            source,
            inventory: Some(crate::inventory_ui::Appearance::prepare(catalog)?),
        })
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"source":self.source,"scale":self.fit.scale,"screen":self.fit.screen.to_array(),"zoom":self.radar.zoom,"radar_locked":self.radar.locked,"diagnostics":self.diagnostics,"regions":self.layout.regions.iter().map(|r|[r.min.to_array(),r.max.to_array()]).collect::<Vec<_>>()})
    }
    #[allow(dead_code)] // Используется проверкой освобождения сцены в shadow-game.
    pub fn image_handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.textures
            .values()
            .filter_map(|t| t.handle.as_ref())
            .chain(self.radar.handle.iter())
            .chain(self.inventory.iter().flat_map(|a| a.image_handles()))
    }
    fn image(&self, name: &str, rect: Rect, canvas: Vec2) -> ImageNode {
        let t = &self.textures[name];
        ImageNode::new(t.handle.clone().expect("HUD setup once"))
            .with_rect(Rect::from_corners(
                rect.min * t.size / canvas,
                rect.max * t.size / canvas,
            ))
            .with_mode(NodeImageMode::Stretch)
    }
}
fn selected_posture(john: &player::Player, party: &Party) -> Option<crate::stance::Posture> {
    match (john.selected, party.jack.selected && party.recruited) {
        (true, false) => Some(john.posture),
        (false, true) => Some(party.jack.posture),
        (true, true) if john.posture == party.jack.posture => Some(john.posture),
        _ => None,
    }
}
fn enabled(id: &str) -> bool {
    matches!(
        id,
        "b_halt"
            | "b_stand"
            | "b_crouch"
            | "b_prone"
            | "b_inventory"
            | "b_track"
            | "b_menu"
            | "b_radar_zoom_in"
            | "b_radar_zoom_out"
            | "b_radar_mode"
            | "r_radar"
    )
}
fn key(id: &str) -> Option<KeyCode> {
    match id {
        "b_halt" => Some(KeyCode::KeyH),
        "b_inventory" => Some(KeyCode::Space),
        "b_track" => Some(KeyCode::KeyT),
        "b_menu" => Some(KeyCode::Escape),
        _ => None,
    }
}
fn tooltip(id: &str) -> &'static str {
    match id {
        "b_halt" => "Остановить · H",
        "b_stand" => "Стоять · S",
        "b_crouch" => "Присесть · C",
        "b_prone" => "Лечь · P",
        "b_inventory" => "Инвентарь · Space",
        "b_track" => "Следить за выбранным бойцом · T",
        "b_menu" => "Меню · Esc",
        "b_radar_zoom_in" => "Приблизить радар",
        "b_radar_zoom_out" => "Отдалить радар",
        "r_radar" => "ЛКМ / перетаскивание — переместить камеру",
        "r_member1" | "r_member2" => "ЛКМ — центрировать камеру на бойце",
        "b_overwatch" => "Наблюдение — ещё не реализовано",
        "b_reload" => "Перезарядка — ещё не реализована",
        "b_form_line" | "b_form_arrow" | "b_form_custom" => "Построения — ещё не реализованы",
        "b_radar_mode" => "Закрепить карту / поворачивать вслед за камерой",
        "b_clock" => "Скорость времени — ещё не реализована",
        _ => "",
    }
}
pub fn setup(
    mut commands: Commands,
    hud: Option<ResMut<Hud>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    camera: Single<Entity, With<training::MapCamera>>,
    window: Single<&Window>,
) {
    let Some(mut h) = hud else { return };
    h.fit = Fit::new(Vec2::new(window.width(), window.height()));
    h.layout = hud_layout::parse(&h.lines, h.fit.screen / h.fit.scale).expect("validated HD HUD");
    let radar_source = h
        .textures
        .get_mut("radar")
        .expect("radar source")
        .image
        .as_ref()
        .expect("HUD setup once")
        .clone();
    h.radar.setup(&radar_source, &mut images);
    for t in h.textures.values_mut() {
        t.handle = Some(images.add(t.image.take().expect("HUD setup once")));
    }
    let root = commands
        .spawn((
            player::SceneEntity,
            Root,
            UiTargetCamera(*camera),
            GlobalZIndex(14),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                bottom: px(0),
                width: percent(100),
                height: px(HEIGHT * h.fit.scale),
                ..default()
            },
        ))
        .id();
    for (i, q) in h.layout.quads.iter().enumerate() {
        let mut e = commands.spawn((
            Piece::Quad(i),
            h.fit.node(q.rect),
            Pickable::IGNORE,
            ChildOf(root),
        ));
        if let Some(mut uv) = q.uv {
            if q.tile_x {
                uv.max.x = uv.min.x + q.rect.width();
            }
            e.insert(h.image(&q.texture, uv, q.canvas));
        } else {
            // UI-градиент усредняет небольшую разницу прозрачности между левым и правым краем.
            let top = q.colors[0].to_srgba();
            let bottom = q.colors[2].to_srgba();
            e.insert(BackgroundGradient::from(LinearGradient::new(
                LinearGradient::TO_BOTTOM,
                vec![
                    ColorStop::percent(
                        Color::srgba(
                            top.red,
                            top.green,
                            top.blue,
                            (top.alpha + q.colors[1].to_srgba().alpha) * 0.5,
                        ),
                        0.0,
                    ),
                    ColorStop::percent(
                        Color::srgba(
                            bottom.red,
                            bottom.green,
                            bottom.blue,
                            (bottom.alpha + q.colors[3].to_srgba().alpha) * 0.5,
                        ),
                        100.0,
                    ),
                ],
            )));
        }
    }
    for (i, r) in h.layout.regions.iter().enumerate() {
        commands.spawn((
            Piece::Block(i),
            selection::UiBlock,
            h.fit.node(*r),
            Pickable::IGNORE,
            ChildOf(root),
        ));
    }
    for (i, a) in h.layout.advice.iter().enumerate() {
        if a.frame.is_none() && a.id != "r_radar" {
            continue;
        }
        let mut e = commands.spawn((
            Piece::Advice(i),
            CommandButton(a.id.clone()),
            UiButton,
            h.fit.node(a.rect),
            ChildOf(root),
        ));
        if let Some(frame) = a.frame {
            e.insert(h.image("simu1_ck.bmp", h.frames[frame], Vec2::splat(256.0)));
        } else {
            e.insert(ImageNode::new(h.radar.handle.clone().expect("radar setup")));
        }
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    if let Some(a) = &mut h.inventory {
        a.setup(&mut images, font.clone());
    }
    for (piece, color) in [
        (Piece::Clock, Color::srgb(0.9, 0.65, 0.12)),
        (Piece::Tooltip, Color::srgb(0.85, 0.85, 0.7)),
    ] {
        commands.spawn((
            piece,
            Text::default(),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(13.0 * h.fit.scale),
                ..default()
            },
            TextColor(color),
            Node::default(),
            Pickable::IGNORE,
            ChildOf(root),
        ));
    }
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    hud: Option<ResMut<Hud>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    buttons: Query<(&CommandButton, &ButtonVisualState)>,
    inv: Option<Res<Inventory>>,
    t: Option<Res<Tutorial>>,
    john: Option<ResMut<player::Player>>,
    party: Option<ResMut<Party>>,
    preview: Res<training::Preview>,
    mut orbit: ResMut<training::Orbit>,
    window: Single<&Window>,
    mut selection: Option<ResMut<selection::Selection>>,
) {
    let Some(mut h) = hud else { return };
    for k in h.injected.drain(..) {
        keys.release(k);
    }
    h.command_posture = None;
    if keys.just_pressed(KeyCode::F3) {
        h.diagnostics = !h.diagnostics;
    }
    let (Some(mut john), Some(mut party), Some(inv), Some(t)) = (john, party, inv, t) else {
        return;
    };
    if t.active.is_some() || t.consumed_input || john.playback.paused {
        h.radar.drag = None;
        return;
    }
    let cursor = selection
        .as_ref()
        .and_then(|s| s.test_cursor)
        .or_else(|| window.cursor_position());
    if h.radar.drag.is_some() {
        // Consume the entire gesture, including a release outside the HUD.
        if let Some(s) = selection.as_mut() {
            s.world_blocked = true;
            s.click_cursor = None;
        }
        mouse.clear_just_pressed(MouseButton::Left);
        if !mouse.pressed(MouseButton::Left) || inv.open {
            h.radar.drag = None;
        } else if let (Some(cursor), Some(grid), Some(view)) = (cursor, &preview.grid, h.radar.drag)
        {
            let rect = h.fit.rect(h.layout.advice("r_radar").rect);
            set_camera(
                &mut orbit,
                view.world((cursor - rect.min) / rect.size(), grid),
            );
            john.follow_camera = false;
            party.jack.follow_camera = false;
        }
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some((button, _)) = buttons
        .iter()
        .find(|(_, s)| **s == ButtonVisualState::Pressed)
    else {
        return;
    };
    let id = button.0.as_str();
    if inv.open && id != "b_inventory" {
        return;
    }
    if id == "r_member1" || id == "r_member2" {
        orbit.target = if id == "r_member1" {
            john.camera_target()
        } else {
            party.jack.camera_target()
        };
        john.follow_camera = false;
        party.jack.follow_camera = false;
    } else if let Some(p) = match id {
        "b_stand" => Some(crate::stance::Posture::Stand),
        "b_crouch" => Some(crate::stance::Posture::Crouch),
        "b_prone" => Some(crate::stance::Posture::Prone),
        _ => None,
    } {
        h.command_posture = Some(p);
    } else if let Some(k) = key(id) {
        // Команды проходят тот же путь отмены действий, что и физическая клавиатура.
        if !keys.pressed(k) {
            keys.press(k);
            h.injected.push(k);
        }
    } else if id == "b_radar_zoom_in" {
        h.radar.zoom = (h.radar.zoom * 2.0).min(8.0);
    } else if id == "b_radar_zoom_out" {
        h.radar.zoom = (h.radar.zoom * 0.5).max(1.0);
    } else if id == "b_radar_mode" {
        h.radar.locked = !h.radar.locked;
    } else if id == "r_radar"
        && let Some(grid) = &preview.grid
        && let Some(cursor) = cursor
    {
        let rect = h.fit.rect(h.layout.advice("r_radar").rect);
        if rect.contains(cursor) {
            let view = h.radar.view(&orbit, grid);
            h.radar.drag = Some(view);
            set_camera(
                &mut orbit,
                view.world((cursor - rect.min) / rect.size(), grid),
            );
            john.follow_camera = false;
            party.jack.follow_camera = false;
        }
    }
    // Keep the physical hold for radar dragging and UI picking; consume only the click.
    mouse.clear_just_pressed(MouseButton::Left);
}
fn set_camera(orbit: &mut training::Orbit, focus: Vec3) {
    // The actual camera is shifted left to leave room for the HUD. Align its focus,
    // rather than the unshifted tracking target, with the requested map position.
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    orbit.target = focus + right * orbit.distance * 0.12;
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn visuals(
    hud: Option<ResMut<Hud>>,
    window: Single<&Window>,
    john: Option<Res<player::Player>>,
    party: Option<Res<Party>>,
    inv: Option<Res<Inventory>>,
    t: Option<Res<Tutorial>>,
    preview: Res<training::Preview>,
    orbit: Res<training::Orbit>,
    combat: Option<Res<crate::combat::Combat>>,
    visibility: Option<Res<crate::perception::VisibilityState>>,
    projection: Single<&Projection, With<training::MapCamera>>,
    mut images: ResMut<Assets<Image>>,
    buttons: Query<(&CommandButton, &ButtonVisualState)>,
    mut roots: Query<&mut Node, (With<Root>, Without<Piece>)>,
    mut pieces: Query<
        (
            &Piece,
            &mut Node,
            Option<&mut ImageNode>,
            Option<&mut Text>,
            Option<&mut TextFont>,
        ),
        Without<Root>,
    >,
) {
    let (Some(mut h), Some(john), Some(party), Some(inv), Some(t)) = (hud, john, party, inv, t)
    else {
        return;
    };
    if let Some(visibility) = &visibility
        && let Some(source) = h.source.as_object_mut()
    {
        source
            .entry("perception")
            .or_insert_with(|| visibility.source.clone());
    }
    let screen = Vec2::new(window.width(), window.height());
    if screen != h.fit.screen {
        h.fit = Fit::new(screen);
        h.layout = hud_layout::parse(&h.lines, screen / h.fit.scale).expect("validated HD HUD");
    }
    let fit = h.fit;
    for mut n in &mut roots {
        n.height = px(HEIGHT * fit.scale);
        n.display = Display::Flex;
    }
    let hover = buttons
        .iter()
        .find(|(_, s)| **s != ButtonVisualState::None)
        .map(|(b, _)| b.0.as_str());
    if !inv.open
        && let Some(grid) = &preview.grid
    {
        let extent = radar::extent(grid);
        let view = h.radar.view(&orbit, grid);
        let mut contacts = Vec::new();
        for (actor, color, sound_id) in [
            (&*john, radar::FRIEND, crate::hearing::ActorId::John),
            (
                &party.jack,
                if party.recruited {
                    radar::FRIEND
                } else {
                    radar::NEUTRAL
                },
                crate::hearing::ActorId::Jack,
            ),
        ] {
            if color == radar::NEUTRAL && visibility.as_ref().is_some_and(|v| !v.neutral) {
                continue;
            }
            contacts.push(radar::Contact {
                position: radar::uv(actor.position, extent),
                color,
                selected: actor.selected && (color == radar::FRIEND),
                friendly: true,
                noise: if actor.selected && color == radar::FRIEND {
                    visibility
                        .as_ref()
                        .and_then(|v| v.hearing.as_ref())
                        .and_then(|h| h.radius(sound_id))
                        .map(|(position, radius)| radar::NoiseRing {
                            center: radar::uv(position, extent),
                            radius: Vec2::splat(radius) / extent,
                        })
                } else {
                    None
                },
            });
        }
        if let Some(combat) = combat {
            for i in 0..combat.guard_count() {
                let guard = combat.guard_at(i);
                if guard.alive() && guard.detected && !guard.carried {
                    contacts.push(radar::Contact {
                        position: radar::uv(guard.guard.position, extent),
                        color: radar::ENEMY,
                        selected: false,
                        friendly: false,
                        noise: None,
                    });
                }
            }
        }
        let fov = match &**projection {
            Projection::Perspective(p) => p.fov,
            _ => std::f32::consts::FRAC_PI_4,
        };
        let footprint = radar::footprint(&orbit, fov, screen.x / screen.y, extent);
        let center = radar::uv(training::camera_focus(&orbit), extent);
        let size = (h.layout.advice("r_radar").rect.size() * fit.scale * window.scale_factor())
            .ceil()
            .as_uvec2();
        h.radar.paint(
            size,
            view,
            contacts,
            footprint,
            center,
            visibility.as_deref(),
            &mut images,
        );
    }
    for (piece, mut node, image, text, font) in &mut pieces {
        let hide_in_inventory = inv.open
            && match *piece {
                Piece::Quad(i) => h.layout.quads[i].rect.min.x < fit.screen.x / fit.scale - 184.0,
                Piece::Block(i) => h.layout.regions[i].min.x < fit.screen.x / fit.scale - 184.0,
                Piece::Advice(i) => {
                    h.layout.advice[i].rect.min.x < fit.screen.x / fit.scale - 184.0
                }
                Piece::Clock => true,
                Piece::Tooltip => false,
            };
        match *piece {
            Piece::Quad(i) => {
                *node = fit.node(h.layout.quads[i].rect);
                let q = &h.layout.quads[i];
                if q.tile_x
                    && let Some(mut image) = image
                {
                    let mut uv = q.uv.expect("tile has UV");
                    uv.max.x = uv.min.x + q.rect.width();
                    let size = h.textures[&q.texture].size;
                    image.rect = Some(Rect::from_corners(
                        uv.min * size / q.canvas,
                        uv.max * size / q.canvas,
                    ));
                }
            }
            Piece::Block(i) => *node = fit.node(h.layout.regions[i]),
            Piece::Advice(i) => {
                let a = &h.layout.advice[i];
                *node = fit.node(a.rect);
                if let Some(mut image) = image
                    && a.id != "r_radar"
                {
                    let selected = match a.id.as_str() {
                        "b_stand" => {
                            selected_posture(&john, &party) == Some(crate::stance::Posture::Stand)
                        }
                        "b_crouch" => {
                            selected_posture(&john, &party) == Some(crate::stance::Posture::Crouch)
                        }
                        "b_prone" => {
                            selected_posture(&john, &party) == Some(crate::stance::Posture::Prone)
                        }
                        "b_track" => john.follow_camera || party.jack.follow_camera,
                        "b_radar_mode" => h.radar.locked,
                        _ => false,
                    };
                    let brightness = if !enabled(&a.id) {
                        0.45
                    } else if hover == Some(a.id.as_str()) || selected {
                        1.0
                    } else {
                        0.75
                    };
                    image.color = Color::srgba(brightness, brightness, brightness, 1.0);
                }
            }
            Piece::Clock => {
                *node = fit.node(h.layout.advice("r_clock").rect);
                if let Some(mut text) = text {
                    let s = t.clock as u32;
                    **text = format!("{:02}:{:02}", s / 60, s % 60);
                }
                if let Some(mut font) = font {
                    font.font_size = FontSize::Px(12.0 * fit.scale);
                }
            }
            Piece::Tooltip => {
                *node = fit.node(Rect::new(
                    195.0,
                    -26.0,
                    fit.screen.x / fit.scale - 200.0,
                    0.0,
                ));
                if let Some(mut text) = text {
                    **text = hover
                        .map(tooltip)
                        .unwrap_or(if john.route.failed || party.jack.route.failed {
                            "Путь к выбранной точке перекрыт"
                        } else {
                            ""
                        })
                        .into();
                }
                if let Some(mut font) = font {
                    font.font_size = FontSize::Px(15.0 * fit.scale);
                }
            }
        }
        if hide_in_inventory {
            node.display = Display::None;
        }
    }
}
pub fn diagnostics(hud: Option<Res<Hud>>, mut nodes: Query<&mut Node, With<Diagnostic>>) {
    let Some(h) = hud else { return };
    for mut n in &mut nodes {
        n.display = if h.diagnostics {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Last выполняется также в меню паузы: виртуальный Esc не остаётся зажатым.
pub fn release_keys(hud: Option<ResMut<Hud>>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    if let Some(mut h) = hud {
        for key in h.injected.drain(..) {
            keys.release(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hud() -> Hud {
        Hud {
            lines: vec![],
            textures: BTreeMap::new(),
            frames: vec![],
            layout: Layout {
                quads: vec![],
                advice: vec![],
                regions: vec![],
                face: Rect::default(),
                stride: 50.0,
            },
            fit: Fit::new(Vec2::new(1280.0, 720.0)),
            diagnostics: false,
            radar: radar::Radar::new(),
            injected: vec![],
            command_posture: None,
            source: serde_json::Value::Null,
            inventory: None,
        }
    }
    fn app(id: &str) -> App {
        let (party, john, t, grid) = crate::party::tests::fixture();
        let mut app = App::new();
        app.insert_resource(hud())
            .insert_resource(party)
            .insert_resource(john)
            .insert_resource(t)
            .insert_resource(crate::inventory::tests::inventory())
            .insert_resource(training::Preview {
                scene: None,
                grid: Some(grid),
                screenshot: None,
                focus: Vec3::ZERO,
                span: 100.0,
                playable: true,
            })
            .insert_resource(training::Orbit {
                target: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.6,
                distance: 10.0,
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        app.world_mut().spawn(Window::default());
        app.world_mut().spawn((
            UiButton,
            CommandButton(id.into()),
            ButtonVisualState::Pressed,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app
    }
    #[test]
    fn button_commands_do_not_leak_world_clicks_or_remain_held() {
        let mut app = app("b_halt");
        app.update();
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::KeyH)
        );
        assert!(
            !app.world()
                .resource::<ButtonInput<MouseButton>>()
                .just_pressed(MouseButton::Left)
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.update();
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyH)
        );
    }
    #[test]
    fn prone_button_ignores_ctrl_without_turning_into_ctrl_p() {
        let mut app = app("b_prone");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        app.update();
        assert_eq!(
            app.world().resource::<Hud>().command_posture,
            Some(crate::stance::Posture::Prone)
        );
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyP)
        );
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::ControlLeft)
        );
    }
    #[test]
    fn menu_click_releases_virtual_escape_before_the_pause_screen() {
        let mut app = app("b_menu");
        app.add_systems(Last, release_keys);
        app.update();
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::Escape)
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.clear();
        keys.press(KeyCode::Escape);
        assert!(keys.just_pressed(KeyCode::Escape));
    }
    #[test]
    fn inventory_keeps_its_toggle_but_blocks_world_commands() {
        let mut app = app("b_reload");
        app.update();
        assert!(app.world().resource::<Hud>().injected.is_empty());
        let mut app = self::app("b_inventory");
        app.world_mut().resource_mut::<Inventory>().open = true;
        app.update();
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::Space)
        );
        let mut app = self::app("b_crouch");
        app.world_mut().resource_mut::<Inventory>().open = true;
        app.update();
        assert!(app.world().resource::<Hud>().injected.is_empty());
        let mut app = self::app("b_inventory");
        app.world_mut()
            .resource_mut::<player::Player>()
            .playback
            .paused = true;
        app.update();
        assert!(app.world().resource::<Hud>().injected.is_empty());
    }
    #[test]
    fn hd_and_ultrawide_preserve_sprite_aspect_and_bottom_anchor() {
        for screen in [
            Vec2::new(1280.0, 720.0),
            Vec2::new(1680.0, 720.0),
            Vec2::new(1920.0, 1080.0),
            Vec2::new(3440.0, 1440.0),
        ] {
            let fit = Fit::new(screen);
            let r = fit.rect(Rect::new(11.0, 48.0, 147.0, 184.0));
            assert!((r.width() - r.height()).abs() < 0.001);
            assert!((screen.y - r.max.y - 11.0 * fit.scale).abs() < 0.001);
            assert!(
                (fit.rect(Rect::new(195.0, 96.0, 237.0, 160.0)).width() / fit.scale - 42.0).abs()
                    < 0.001
            );
        }
    }
    #[test]
    fn diagnostics_are_hidden_until_f3() {
        let mut app = app("b_reload");
        app.add_systems(Update, diagnostics.after(input));
        let e = app.world_mut().spawn((Diagnostic, Node::default())).id();
        app.update();
        assert_eq!(app.world().get::<Node>(e).unwrap().display, Display::None);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::F3);
        app.update();
        assert_eq!(app.world().get::<Node>(e).unwrap().display, Display::Flex);
    }
}
