//! Оригинальная игровая панель, с независимыми от соотношения сторон краями.
use crate::{
    hud_layout::{self, Layout},
    inventory::Inventory,
    party::Party,
    player, selection, training,
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
    zoom: f32,
    injected: Vec<KeyCode>,
    pub command_posture: Option<crate::stance::Posture>,
    source: serde_json::Value,
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
    Marker(u8),
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
        let source = serde_json::json!({"definition":DEFINITION,"origin":catalog.source_label(DEFINITION)?,"textures":sources,"geometry_from_original":true,"command_frames":frames.len(),"minimap_fog_of_war":false,"disabled_commands":["overwatch","reload","formations","radar_mode","clock_speed"]});
        Ok(Self {
            lines,
            textures,
            frames,
            layout,
            fit: Fit::new(Vec2::new(1280.0, 720.0)),
            diagnostics: false,
            zoom: 2.0,
            injected: Vec::new(),
            command_posture: None,
            source,
        })
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"source":self.source,"scale":self.fit.scale,"screen":self.fit.screen.to_array(),"zoom":self.zoom,"diagnostics":self.diagnostics,"regions":self.layout.regions.iter().map(|r|[r.min.to_array(),r.max.to_array()]).collect::<Vec<_>>()})
    }
    #[allow(dead_code)] // Используется проверкой освобождения сцены в shadow-game.
    pub fn image_handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.textures.values().filter_map(|t| t.handle.as_ref())
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
        "r_radar" => "ЛКМ — переместить камеру",
        "r_member1" | "r_member2" => "ЛКМ — центрировать камеру на бойце",
        "b_overwatch" => "Наблюдение — ещё не реализовано",
        "b_reload" => "Перезарядка — ещё не реализована",
        "b_form_line" | "b_form_arrow" | "b_form_custom" => "Построения — ещё не реализованы",
        "b_radar_mode" => "Режимы радара — ещё не реализованы",
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
            e.insert(h.image(
                "radar",
                Rect::new(0.0, 0.0, 256.0, 256.0),
                Vec2::splat(256.0),
            ));
        }
    }
    for member in 1..=2 {
        commands.spawn((
            Piece::Marker(member),
            UiButton,
            CommandButton(format!("r_member{member}")),
            Node::default(),
            BorderColor::all(Color::srgb(0.2, 0.9, 0.15)),
            BackgroundColor(Color::NONE),
            ChildOf(root),
        ));
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
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
fn radar_crop(center: Vec3, extent: Vec2, zoom: f32) -> Rect {
    let half = 0.5 / zoom;
    let c =
        (Vec2::new(center.x, -center.z) / extent).clamp(Vec2::splat(half), Vec2::splat(1.0 - half));
    Rect::from_corners(c - Vec2::splat(half), c + Vec2::splat(half))
}
fn extent(grid: &shadow_formats::TerrainGrid) -> Vec2 {
    Vec2::new(grid.config.width as f32, grid.config.height as f32)
        * grid.config.edge
        * crate::map_scene::WORLD_SCALE
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
    selection: Option<Res<selection::Selection>>,
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
    if inv.open
        || t.active.is_some()
        || t.consumed_input
        || john.playback.paused
        || !mouse.just_pressed(MouseButton::Left)
    {
        return;
    }
    let Some((button, _)) = buttons
        .iter()
        .find(|(_, s)| **s == ButtonVisualState::Pressed)
    else {
        return;
    };
    let id = button.0.as_str();
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
        h.zoom = (h.zoom * 2.0).min(8.0);
    } else if id == "b_radar_zoom_out" {
        h.zoom = (h.zoom * 0.5).max(1.0);
    } else if id == "r_radar"
        && let Some(grid) = &preview.grid
        && let Some(cursor) = selection
            .as_ref()
            .and_then(|s| s.test_cursor)
            .or_else(|| window.cursor_position())
    {
        let r = h.fit.rect(h.layout.advice("r_radar").rect);
        if r.contains(cursor) {
            let crop = radar_crop(orbit.target, extent(grid), h.zoom);
            let uv = crop.min + (cursor - r.min) / r.size() * crop.size();
            let p = Vec3::new(uv.x * extent(grid).x, 0.0, -uv.y * extent(grid).y);
            orbit.target = p.with_y(player::surface(grid, p).unwrap_or(0.0));
            john.follow_camera = false;
            party.jack.follow_camera = false;
        }
    }
    mouse.reset(MouseButton::Left);
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
    buttons: Query<(&CommandButton, &ButtonVisualState)>,
    mut roots: Query<&mut Node, (With<Root>, Without<Piece>)>,
    mut pieces: Query<
        (
            &Piece,
            &mut Node,
            Option<&mut ImageNode>,
            Option<&mut Text>,
            Option<&mut TextFont>,
            Option<&mut BackgroundColor>,
        ),
        Without<Root>,
    >,
) {
    let (Some(mut h), Some(john), Some(party), Some(inv), Some(t)) = (hud, john, party, inv, t)
    else {
        return;
    };
    let screen = Vec2::new(window.width(), window.height());
    if screen != h.fit.screen {
        h.fit = Fit::new(screen);
        h.layout = hud_layout::parse(&h.lines, screen / h.fit.scale).expect("validated HD HUD");
    }
    let fit = h.fit;
    for mut n in &mut roots {
        n.height = px(HEIGHT * fit.scale);
        n.display = if inv.open {
            Display::None
        } else {
            Display::Flex
        };
    }
    let hover = buttons
        .iter()
        .find(|(_, s)| **s != ButtonVisualState::None)
        .map(|(b, _)| b.0.as_str());
    let crop = preview
        .grid
        .as_ref()
        .map(|g| radar_crop(orbit.target, extent(g), h.zoom));
    for (piece, mut node, image, text, font, bg) in &mut pieces {
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
                if let Some(mut image) = image {
                    if a.id == "r_radar" {
                        if let Some(crop) = crop {
                            let size = h.textures["radar"].size;
                            image.rect = Some(Rect::from_corners(crop.min * size, crop.max * size));
                        }
                    } else {
                        let selected = match a.id.as_str() {
                            "b_stand" => {
                                selected_posture(&john, &party)
                                    == Some(crate::stance::Posture::Stand)
                            }
                            "b_crouch" => {
                                selected_posture(&john, &party)
                                    == Some(crate::stance::Posture::Crouch)
                            }
                            "b_prone" => {
                                selected_posture(&john, &party)
                                    == Some(crate::stance::Posture::Prone)
                            }
                            "b_track" => john.follow_camera || party.jack.follow_camera,
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
            }
            Piece::Marker(member) => {
                let actor = if member == 1 { &*john } else { &party.jack };
                let uv = preview
                    .grid
                    .as_ref()
                    .map(|g| Vec2::new(actor.position.x, -actor.position.z) / extent(g));
                if let (Some(uv), Some(crop)) = (uv, crop)
                    && crop.contains(uv)
                    && (member == 1 || party.recruited)
                {
                    let radar = h.layout.advice("r_radar").rect;
                    let p = radar.min + (uv - crop.min) / crop.size() * radar.size();
                    *node = fit.node(Rect::from_center_size(p, Vec2::splat(7.0)));
                    node.border = UiRect::all(px(fit.scale));
                    node.border_radius = BorderRadius::MAX;
                    if let Some(mut bg) = bg {
                        bg.0 = if actor.selected {
                            Color::srgba(0.2, 0.9, 0.15, 0.3)
                        } else {
                            Color::NONE
                        };
                    }
                } else {
                    node.display = Display::None;
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
                    **text = hover.map(tooltip).unwrap_or("").into();
                }
                if let Some(mut font) = font {
                    font.font_size = FontSize::Px(15.0 * fit.scale);
                }
            }
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
            zoom: 2.0,
            injected: vec![],
            command_posture: None,
            source: serde_json::Value::Null,
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
    fn disabled_and_paused_commands_cannot_issue_game_actions() {
        let mut app = app("b_reload");
        app.update();
        assert!(app.world().resource::<Hud>().injected.is_empty());
        let mut app = self::app("b_inventory");
        app.world_mut().resource_mut::<Inventory>().open = true;
        app.update();
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(KeyCode::Space)
        );
    }
    #[test]
    fn radar_crop_uses_negative_world_z_and_clamps_at_map_edges() {
        let extent = Vec2::new(100.0, 200.0);
        let crop = radar_crop(Vec3::new(50.0, 0.0, -100.0), extent, 2.0);
        assert_eq!(crop, Rect::new(0.25, 0.25, 0.75, 0.75));
        assert_eq!(
            radar_crop(Vec3::ZERO, extent, 2.0),
            Rect::new(0.0, 0.0, 0.5, 0.5)
        );
        assert_eq!(
            radar_crop(Vec3::new(1000.0, 0.0, -1000.0), extent, 2.0),
            Rect::new(0.5, 0.5, 1.0, 1.0)
        );
        assert_eq!(
            radar_crop(Vec3::ZERO, extent, 1.0),
            Rect::new(0.0, 0.0, 1.0, 1.0)
        );
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
