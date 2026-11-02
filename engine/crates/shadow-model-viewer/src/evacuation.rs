//! Учебная зона эвакуации, исходные жесты/callback и успешный выход CER.
//! Рисунок дыма временный: исходная smoke_light + RAW, наши billboard-частицы.
use crate::{
    combat,
    inventory::Inventory,
    map_scene::WORLD_SCALE,
    party::Party,
    player,
    stance::Posture,
    tutorial::Tutorial,
    ui_controls::{ButtonVisualState, UiButton},
    vehicle::Vehicle,
};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};
use shadow_formats::{AssetCatalog, CampaignEventType as Event, MissionDefinition};
use std::{collections::BTreeSet, error::Error};

#[derive(Clone, Copy)]
struct Gesture {
    clip: usize,
    callback: f32,
}
#[derive(Clone, Copy)]
struct Order {
    member: u8,
    gesture: Option<Gesture>,
}
#[derive(Resource)]
pub struct Evacuation {
    pub center: Vec2,
    pub radius: f32,
    primary: BTreeSet<String>,
    gestures: [[Gesture; 2]; 2],
    order: Option<Order>,
    pub popped: u32,
    pub signal_position: Option<Vec3>,
    started: Option<f64>,
    texture: Option<Image>,
    pub source: serde_json::Value,
}
#[derive(Component)]
pub(crate) struct Panel;
#[derive(Component)]
pub(crate) struct Status;
#[derive(Component)]
pub struct Signal(pub u8);
#[derive(Component)]
pub(crate) struct Puff(u32);
impl Evacuation {
    pub fn prepare(
        catalog: &AssetCatalog,
        mission: &MissionDefinition,
        john: &mut player::Player,
        party: &mut Party,
    ) -> Result<Self, Box<dyn Error>> {
        let zones: Vec<_> = mission
            .zones
            .iter()
            .filter(|z| z.id.eq_ignore_ascii_case("CS_ZONE_EXTRACTION"))
            .collect();
        let [zone] = zones.as_slice() else {
            return Err("нужна одна зона CS_ZONE_EXTRACTION".into());
        };
        let responses: Vec<_> = mission
            .responses
            .iter()
            .filter(|r| crate::tutorial::extraction_response(r))
            .collect();
        let [response] = responses.as_slice() else {
            return Err("нужен один поддержанный выход обучения по сигналу".into());
        };
        let primary: BTreeSet<_> = mission
            .objectives
            .iter()
            .filter(|o| o.primary)
            .map(|o| o.id.to_ascii_lowercase())
            .collect();
        if primary.is_empty() {
            return Err("нет основных задач для эвакуации".into());
        }
        let sequence_lines =
            shadow_formats::config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
        let mut motions = vec![];
        let mut prepare = |p: &mut player::Player| -> Result<[Gesture; 2], Box<dyn Error>> {
            let mut gestures = [Gesture {
                clip: 0,
                callback: 0.0,
            }; 2];
            for (i, name) in ["MSEQ_STAND_POP_FLARE", "MSEQ_CROUCH_POP_FLARE"]
                .into_iter()
                .enumerate()
            {
                let cog = crate::stance::default_cog(
                    &sequence_lines,
                    if i == 0 { "STAND" } else { "CROUCH" },
                )?;
                let (clip, frame) = combat::sequence_with_cog(
                    catalog,
                    name,
                    Some("MCB_POP_SIGNAL_FLARE"),
                    &p.rig.as_ref().ok_or("сигнал готовится до сцены")?.bones,
                    Some(cog),
                )?;
                let frame = frame.unwrap();
                motions.push(serde_json::json!({"actor":p.name,"sequence":name,"motion":clip.name,"frames":clip.frames.len(),"fps":clip.fps,"callback_frame":frame,"callback_seconds":frame as f32/clip.fps}));
                gestures[i] = Gesture {
                    clip: p.clips.len(),
                    callback: frame as f32 / clip.fps,
                };
                p.clips.push(clip);
            }
            Ok(gestures)
        };
        let gestures = [prepare(john)?, prepare(&mut party.jack)?];
        let path = "textures/object/smoke_light.bmp";
        let texture = crate::asset_mesh::image(catalog, path)?.image;
        let center = Vec2::new(zone.center[0], -zone.center[1]) * WORLD_SCALE;
        let radius = zone.radius * WORLD_SCALE;
        let source = serde_json::json!({"mission":"campaign/training/training.txt","zone":zone.id,"center_source_units":zone.center,"radius_source_units":zone.radius,"color":zone.color,"primary_objectives":primary,"response_line":response.line,"end_campaign":"EOC_RETURN_TO_MAIN_MENU","delay_ms":response.actions[0].delay_ms.unwrap_or(0),"delay_exe":"0x443791 maps END_CAMPAIGN to 10; 0x4494bb..0x4494fd passes body[0x10] to global event queue","motions":motions,"smoke_texture":path,"smoke_texture_origin":catalog.source_label(path)?,"smoke_visual_original_verified":false,"zone_marker":"terrain-following ribbon; prototype visual","readiness_source":"help_combat_18 and mission comment: all primary objectives, all mercenaries in zone, outside Hummer","prone_rule_prototype":"change to crouch before signal"});
        Ok(Self {
            center,
            radius,
            primary,
            gestures,
            order: None,
            popped: 0,
            signal_position: None,
            started: None,
            texture: Some(texture),
            source,
        })
    }
    pub fn inside(&self, p: Vec3) -> bool {
        p.is_finite() && Vec2::new(p.x, p.z).distance(self.center) <= self.radius
    }
    pub fn ready(
        &self,
        john: &player::Player,
        party: &Party,
        t: &Tutorial,
        occupied: bool,
    ) -> bool {
        self.popped == 0
            && self.order.is_none()
            && t.end_campaign.is_none()
            && party.recruited
            && !occupied
            && self.primary.is_subset(&t.completed)
            && self.inside(john.position)
            && self.inside(party.jack.position)
    }
    pub fn request(
        &mut self,
        member: u8,
        john: &mut player::Player,
        party: &mut Party,
        t: &Tutorial,
        occupied: bool,
    ) -> bool {
        if !matches!(member, 1 | 2)
            || !self.ready(john, party, t, occupied)
            || john.playback.paused
            || t.pausing()
        {
            return false;
        }
        let p = if member == 1 { john } else { &mut party.jack };
        if !p.selected
            || p.changing_posture()
            || p.action.is_some()
            || p.pickup_seconds.is_some()
            || p.destination.is_some()
        {
            return false;
        }
        self.order = Some(Order {
            member,
            gesture: None,
        });
        if p.posture == Posture::Prone {
            p.request_posture(Posture::Crouch);
        } else {
            self.start(p, member);
        }
        true
    }
    fn start(&mut self, p: &mut player::Player, member: u8) {
        let gesture =
            self.gestures[usize::from(member - 1)][usize::from(p.posture != Posture::Stand)];
        p.begin_action(gesture.clip);
        self.order = Some(Order {
            member,
            gesture: Some(gesture),
        });
    }
    pub fn update(
        &mut self,
        john: &mut player::Player,
        party: &mut Party,
        t: &mut Tutorial,
        occupied: bool,
        inventory_open: bool,
    ) {
        if inventory_open || john.playback.paused || t.pausing() {
            return;
        }
        let Some(order) = self.order else {
            return;
        };
        if self.popped == 0
            && (!party.recruited
                || occupied
                || !self.primary.is_subset(&t.completed)
                || !self.inside(john.position)
                || !self.inside(party.jack.position))
        {
            let p = if order.member == 1 {
                john
            } else {
                &mut party.jack
            };
            if order
                .gesture
                .is_some_and(|g| p.action.is_some_and(|a| a.0 == g.clip))
            {
                p.finish_pickup();
            }
            self.order = None;
            return;
        }
        let p = if order.member == 1 {
            john
        } else {
            &mut party.jack
        };
        if p.changing_posture() {
            return;
        }
        let Some(g) = order.gesture else {
            self.start(p, order.member);
            return;
        };
        let Some((clip, seconds)) = p.action else {
            self.order = None;
            return;
        };
        if clip != g.clip {
            self.order = None;
            return;
        }
        if self.popped == 0 && seconds >= g.callback {
            self.popped = 1;
            self.signal_position = Some(p.position);
            self.started = Some(t.clock);
            t.fire(Event::SignalFlarePopped, "");
            t.tick(0.0, false);
        }
        if seconds >= p.clips[clip].duration() {
            p.finish_pickup();
            self.order = None;
        }
    }
    fn cancel(&mut self, john: &mut player::Player, party: &mut Party) {
        if let Some(order) = self.order.take() {
            let p = if order.member == 1 {
                john
            } else {
                &mut party.jack
            };
            if order
                .gesture
                .is_some_and(|g| p.action.is_some_and(|a| a.0 == g.clip))
            {
                p.finish_pickup();
            }
        }
    }
    pub fn restart(&mut self) {
        self.order = None;
        self.popped = 0;
        self.signal_position = None;
        self.started = None;
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"center_metres":self.center.to_array(),"radius_metres":self.radius,"popped":self.popped,"signal_position_metres":self.signal_position.map(|v|v.to_array()),"signaling_member":self.order.map(|o|o.member),"source":self.source})
    }
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    e: Option<ResMut<Evacuation>>,
    john: Option<ResMut<player::Player>>,
    party: Option<ResMut<Party>>,
    t: Option<Res<Tutorial>>,
    v: Option<Res<Vehicle>>,
    inv: Option<Res<Inventory>>,
    buttons: Query<(&Signal, &ButtonVisualState)>,
) {
    let (Some(mut e), Some(mut john), Some(mut party), Some(t), Some(v), Some(inv)) =
        (e, john, party, t, v, inv)
    else {
        return;
    };
    if keys.just_pressed(KeyCode::Home) && t.active.is_none() && !inv.open {
        e.restart();
    }
    if keys.just_pressed(KeyCode::KeyH)
        && e.order.is_some_and(|o| {
            if o.member == 1 {
                john.selected
            } else {
                party.jack.selected
            }
        })
    {
        e.cancel(&mut john, &mut party);
    }
    if inv.open
        || inv.consumed_input
        || t.active.is_some()
        || t.consumed_input
        || !mouse.just_pressed(MouseButton::Left)
    {
        return;
    }
    if let Some((button, _)) = buttons
        .iter()
        .find(|(_, s)| **s == ButtonVisualState::Pressed)
    {
        e.request(button.0, &mut john, &mut party, &t, v.occupied);
        mouse.reset(MouseButton::Left); // Кнопка сигнала не выдаёт одновременно приказ по земле.
    }
}
pub fn advance(
    e: Option<ResMut<Evacuation>>,
    john: Option<ResMut<player::Player>>,
    party: Option<ResMut<Party>>,
    t: Option<ResMut<Tutorial>>,
    v: Option<Res<Vehicle>>,
    inv: Option<Res<Inventory>>,
) {
    if let (Some(mut e), Some(mut john), Some(mut party), Some(mut t), Some(v), Some(inv)) =
        (e, john, party, t, v, inv)
    {
        e.update(&mut john, &mut party, &mut t, v.occupied, inv.open);
    }
}
// Граница в координатах мира: одна горизонтальная окружность уходит под холмы.
fn zone_mesh(e: &Evacuation, grid: &shadow_formats::TerrainGrid) -> Mesh {
    let segments = 256_u32;
    let mut points = Vec::with_capacity((segments as usize + 1) * 2);
    for i in 0..=segments {
        let angle = i as f32 * std::f32::consts::TAU / segments as f32;
        for radius in [e.radius * 0.995, e.radius * 1.005] {
            let p = Vec3::new(
                e.center.x + angle.cos() * radius,
                0.0,
                e.center.y + angle.sin() * radius,
            );
            points.push(
                p.with_y(player::surface(grid, p).unwrap_or(0.0) + 0.12)
                    .to_array(),
            );
        }
    }
    let indices = (0..segments)
        .flat_map(|i| {
            let a = i * 2;
            [a, a + 1, a + 2, a + 2, a + 1, a + 3]
        })
        .collect();
    let count = points.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
    .with_inserted_indices(Indices::U32(indices))
}

#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    e: Option<ResMut<Evacuation>>,
    preview: Res<crate::training::Preview>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let Some(mut e) = e else {
        return;
    };
    commands.spawn((
        player::SceneEntity,
        Mesh3d(meshes.add(zone_mesh(&e, preview.grid.as_ref().unwrap()))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.25, 0.65, 1.0),
            unlit: true,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
    ));
    let texture = images.add(e.texture.take().expect("evacuation setup once"));
    let plane = meshes.add(Rectangle::new(1.0, 1.0));
    for i in 0..12 {
        commands.spawn((
            player::SceneEntity,
            Puff(i),
            Mesh3d(plane.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(texture.clone()),
                base_color: Color::srgba(0.65, 0.85, 0.7, 0.4),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    let panel = commands
        .spawn((
            player::SceneEntity,
            Panel,
            crate::selection::UiBlock,
            UiTargetCamera(*camera),
            GlobalZIndex(20),
            Node {
                position_type: PositionType::Absolute,
                right: px(25),
                top: px(25),
                width: px(340),
                padding: UiRect::all(px(8)),
                border: UiRect::all(px(1)),
                row_gap: px(8),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            BorderColor::all(Color::srgb(0.25, 0.5, 0.15)),
        ))
        .id();
    commands.spawn((
        Status,
        Text::default(),
        TextFont {
            font: font.clone().into(),
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.75, 0.9, 1.0)),
        ChildOf(panel),
    ));
    for (id, label) in [
        (1, "Подать дымовой сигнал · Джон"),
        (2, "Подать дымовой сигнал · Джек"),
    ] {
        let button = commands
            .spawn((
                Signal(id),
                UiButton,
                ButtonVisualState::None,
                Node {
                    height: px(36),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.08, 0.3, 0.22)),
                ChildOf(panel),
            ))
            .id();
        commands.spawn((
            Text::new(label),
            TextFont {
                font: font.clone().into(),
                font_size: bevy::text::FontSize::Px(16.0),
                ..default()
            },
            TextColor(Color::WHITE),
            ChildOf(button),
        ));
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn visuals(
    e: Option<Res<Evacuation>>,
    john: Option<Res<player::Player>>,
    party: Option<Res<Party>>,
    t: Option<Res<Tutorial>>,
    v: Option<Res<Vehicle>>,
    inv: Option<Res<Inventory>>,
    camera: Single<&GlobalTransform, With<crate::training::MapCamera>>,
    mut panels: Query<&mut Node, (With<Panel>, Without<Signal>)>,
    mut buttons: Query<(&Signal, &mut Node), Without<Panel>>,
    mut texts: Query<&mut Text, With<Status>>,
    mut puffs: Query<(&Puff, &mut Transform, &mut Visibility)>,
) {
    let (Some(e), Some(john), Some(party), Some(t), Some(v), Some(inv)) =
        (e, john, party, t, v, inv)
    else {
        return;
    };
    let show = party.recruited
        && !inv.open
        && t.active.is_none()
        && (e.ready(&john, &party, &t, v.occupied) || e.popped > 0);
    for mut node in &mut panels {
        node.display = if show { Display::Flex } else { Display::None };
    }
    let ready = e.ready(&john, &party, &t, v.occupied) && !john.playback.paused;
    for (button, mut node) in &mut buttons {
        let p = if button.0 == 1 { &*john } else { &party.jack };
        node.display = if ready
            && p.selected
            && p.action.is_none()
            && p.destination.is_none()
            && !p.changing_posture()
        {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut text in &mut texts {
        **text = if t.end_campaign.is_some() {
            "Обучение завершено!\nHome — начать заново".into()
        } else if e.popped > 0 {
            "Дымовой сигнал подан.\nОбучение завершено — возвращаемся в меню…".into()
        } else {
            if v.occupied {
                "Выведи Джона из Hummer"
            } else if ready {
                "Зона эвакуации. Выбери бойца и подай сигнал."
            } else {
                "Приведи обоих бойцов в зону эвакуации."
            }
            .into()
        };
    }
    for (puff, mut tr, mut visible) in &mut puffs {
        if let (Some(point), Some(start)) = (e.signal_position, e.started) {
            let age = (t.clock - start) as f32;
            let phase = (age * 0.9 + puff.0 as f32 * 0.12) % 1.6;
            let a = puff.0 as f32 * 2.399;
            tr.translation = point
                + Vec3::new(
                    a.cos() * phase * 0.32,
                    0.5 + phase * 1.6,
                    a.sin() * phase * 0.32,
                );
            tr.scale = Vec3::splat(0.6 + phase * 1.2);
            tr.rotation = camera.rotation();
            *visible = Visibility::Visible;
        } else {
            *visible = Visibility::Hidden;
        }
    }
}

#[derive(Resource)]
pub struct NativeSmoke {
    pub path: std::path::PathBuf,
    screenshot: std::path::PathBuf,
    pub integrated: bool,
    #[allow(dead_code)] // Читает главное приложение; витрина запускается сразу на карте.
    pub menu_started: bool,
    pub stage: u32,
    frames: u32,
    total: u32,
    last: Option<u32>,
    clock: f64,
    john: Vec3,
    jack: Vec3,
    pub checks: Vec<serde_json::Value>,
    pub before_exit: Option<serde_json::Value>,
}
impl NativeSmoke {
    pub fn new(
        path: std::path::PathBuf,
        screenshot: Option<std::path::PathBuf>,
        integrated: bool,
    ) -> Self {
        Self {
            screenshot: screenshot.unwrap_or_else(|| path.with_extension("png")),
            path,
            integrated,
            menu_started: false,
            stage: 0,
            frames: 0,
            total: 0,
            last: None,
            clock: 0.0,
            john: Vec3::ZERO,
            jack: Vec3::ZERO,
            checks: vec![],
            before_exit: None,
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn native_smoke(
    mut commands: Commands,
    s: Option<ResMut<NativeSmoke>>,
    e: Option<Res<Evacuation>>,
    john: Option<ResMut<player::Player>>,
    party: Option<ResMut<Party>>,
    t: Option<ResMut<Tutorial>>,
    v: Option<ResMut<Vehicle>>,
    preview: Res<crate::training::Preview>,
    mut orbit: ResMut<crate::training::Orbit>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut buttons: Query<(&Signal, &mut ButtonVisualState, &Node)>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
    window: Single<&Window>,
    puffs: Query<&Visibility, With<Puff>>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut s), Some(e), Some(mut john), Some(mut party), Some(mut t), Some(mut v)) =
        (s, e, john, party, t, v)
    else {
        return;
    };
    s.total += 1;
    s.frames += 1;
    if s.last != Some(s.stage) {
        eprintln!(
            "evacuation smoke stage {}: signal={} completed={} end={:?}",
            s.stage,
            e.popped,
            t.completed.len(),
            t.end_campaign
        );
        s.last = Some(s.stage);
        s.frames = 0;
    }
    assert!(
        s.total < 2200,
        "evacuation timeout stage {}: {}",
        s.stage,
        e.report()
    );
    keys.reset_all();
    mouse.reset_all();
    party.test_cursor = None;
    for (_, mut state, _) in &mut buttons {
        *state = ButtonVisualState::None;
    }
    if s.total < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    let grid = preview.grid.as_ref().unwrap();
    match s.stage {
        0 => {
            john.playback.paused = false;
            if s.integrated {
                // Фикстура UI-выхода: события предыдущих задач, а не повтор всей дороги/боёв.
                for group in ["AI_Stab", "AI_Shoot", "AI_Grenade", "AI_SMG"] {
                    t.fire(Event::AllPlayerInfantryKilled, group);
                    t.tick(0.0, false);
                }
                t.fire(Event::VehicleOccupied, "AlVhQd-Hummer");
                t.tick(0.0, false);
            }
            assert_eq!(t.completed.len(), 5);
            if v.occupied {
                v.arm_exit();
                for i in 0..16 {
                    let a = i as f32 * std::f32::consts::TAU / 16.0;
                    let point = v.position + Vec3::new(a.cos() * 3.8, 0.0, a.sin() * 3.8);
                    if v.exit(point, &mut john, grid, None) {
                        break;
                    }
                }
            }
            assert!(!v.occupied);
            john.position = party.jack.position + Vec3::Z;
            john.finish_pickup();
            assert!(party.encounter(&mut john, &mut t, true, true));
            party.receive(&mut t);
            assert_eq!(t.completed.len(), 6);
            let base = Vec3::new(e.center.x, 0.0, e.center.y);
            john.position = base + Vec3::new(e.radius - 4.0, 0.0, -3.0);
            john.position.y = player::surface(grid, john.position).unwrap();
            party.jack.position = base + Vec3::X * (e.radius + 1.5);
            party.jack.position.y = player::surface(grid, party.jack.position).unwrap();
            party.jack.finish_pickup();
            party.select(2, &mut john);
            orbit.target = base.with_y(john.position.y);
            orbit.yaw = std::f32::consts::PI;
            orbit.pitch = 0.95;
            orbit.distance = 60.0;
            assert!(!e.ready(&john, &party, &t, false));
            s.checks.push(serde_json::json!({"original_zone":true,"all_primary_objectives":true,"one_outside_blocks_signal":true,"fixture":"previous objectives/events and rescue prepared; travel from Jack to extraction not exercised"}));
            s.stage = 1;
        }
        1 if s.frames > 4 => {
            assert!(
                buttons
                    .iter()
                    .all(|(_, _, node)| node.display == Display::None)
            );
            let target = Vec3::new(e.center.x + e.radius - 4.0, 0.0, e.center.y + 3.0);
            let target = target.with_y(player::surface(grid, target).unwrap());
            let (camera, tr) = *camera;
            if let Ok(cursor) = camera.world_to_viewport(tr, target)
                && cursor.x > 315.0
            {
                party.test_cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                s.stage = 2;
            }
        }
        2 if s.frames > 8 && party.jack.destination.is_none() => {
            assert!(e.ready(&john, &party, &t, false));
            assert_eq!(e.popped, 0);
            keys.press(KeyCode::KeyP);
            s.stage = 3;
        }
        3 if party.jack.posture == Posture::Prone && !party.jack.changing_posture() => {
            for (button, mut state, node) in &mut buttons {
                if button.0 == 2 {
                    assert_ne!(node.display, Display::None);
                    *state = ButtonVisualState::Pressed;
                }
            }
            mouse.press(MouseButton::Left);
            s.stage = 4;
        }
        4 => {
            if let Some(order) = e.order
                && let Some(g) = order.gesture
                && let Some((_, seconds)) = party.jack.action
            {
                assert_eq!(party.jack.posture, Posture::Crouch);
                assert_eq!(e.popped, 0, "callback happened before pause probe");
                if seconds >= g.callback * 0.3 {
                    s.clock = t.clock;
                    s.john = john.position;
                    s.jack = party.jack.position;
                    keys.press(KeyCode::ControlLeft);
                    keys.press(KeyCode::KeyP);
                    s.stage = 5;
                }
            }
        }
        5 if s.frames >= 12 => {
            assert!(john.playback.paused && (t.clock - s.clock).abs() < 1e-6);
            assert_eq!(e.popped, 0);
            assert_eq!(john.position, s.john);
            assert_eq!(party.jack.position, s.jack);
            s.checks.push(serde_json::json!({"mouse_moves_jack_into_zone":true,"selected_jack_button":true,"prone_changes_to_crouch":true,"pause_freezes_callback_and_clock":true}));
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::KeyP);
            s.stage = 6;
        }
        6 if e.popped == 1 => {
            assert!(puffs.iter().any(|v| *v == Visibility::Visible));
            assert!(t.end_campaign.is_none());
            assert_eq!(
                t.events
                    .iter()
                    .filter(|e| e.starts_with("CET_SIGNAL_FLARE_POPPED:"))
                    .count(),
                1
            );
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    s.screenshot.clone(),
                ));
            s.before_exit = Some(
                serde_json::json!({"evacuation":e.report(),"tutorial":t.report(),"party":party.report(&john),"window":[window.width(),window.height()]}),
            );
            s.clock = t.clock;
            s.stage = 7;
        }
        8 if !s.integrated && s.frames > 12 => {
            keys.press(KeyCode::Home);
            s.stage = 9;
        }
        9 if s.frames > 8 => {
            assert_eq!(e.popped, 0);
            assert!(
                e.order.is_none()
                    && e.signal_position.is_none()
                    && t.end_campaign.is_none()
                    && t.completed.is_empty()
                    && !party.recruited
            );
            s.checks
                .push(serde_json::json!({"home_clears_signal_end_and_party":true}));
            let report = serde_json::json!({"status":"passed","checks":s.checks,"before_exit":s.before_exit,"after_reset":e.report(),"physical_pointer_tested":false,"input":"Bevy ButtonInput, viewport rays and project ButtonVisualState","returned_to_main_menu":false});
            std::fs::write(
                &s.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            s.stage = 10;
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

// До смены Screen: в следующем кадре ресурс Tutorial уже будет удалён.
pub fn observe_end(
    s: Option<ResMut<NativeSmoke>>,
    e: Option<Res<Evacuation>>,
    t: Option<Res<Tutorial>>,
) {
    let (Some(mut s), Some(e), Some(t)) = (s, e, t) else {
        return;
    };
    if s.stage == 7 && t.end_campaign.is_some() {
        assert_eq!(t.end_campaign.as_deref(), Some("EOC_RETURN_TO_MAIN_MENU"));
        let delay = e.source["delay_ms"].as_f64().unwrap() / 1000.0;
        assert!(t.clock - e.started.unwrap() + 1e-6 >= delay);
        s.checks.push(serde_json::json!({"original_callback_once":true,"smoke_visible":true,"original_end_delay":true,"delay_ms":e.source["delay_ms"]}));
        s.stage = 8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tutorial() -> Tutorial {
        let mission=MissionDefinition::from_bytes(b"SPECIFY_MTF test.mtf\nDEFINE_OBJECTIVE true goal \"Primary\"\nCER_DEFINITION\nCER_FOR_TYPE CET_SIGNAL_FLARE_POPPED\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_END_CAMPAIGN\nCER_BODY_IDENTIFIER EOC_RETURN_TO_MAIN_MENU\nCER_BODY_MS_DELAY 1500\n").unwrap();
        let help = shadow_formats::HelpDefinitions::from_bytes(
            b"HELP_DEF\nHELP_ID dummy\nHELP_CONTEXT\nHELP_BODY \"Pause\"\n",
        )
        .unwrap();
        let mut t = Tutorial::new(mission, help).unwrap();
        t.objectives.insert("goal".into());
        t.completed.insert("goal".into());
        t
    }
    fn fixture() -> (Evacuation, player::Player, Party, Tutorial) {
        let (mut party, mut john, _, _) = crate::party::tests::fixture();
        party.recruited = true;
        party.jack.selected = true;
        john.selected = true;
        let e = Evacuation {
            center: Vec2::ZERO,
            radius: 2.0,
            primary: ["goal".into()].into(),
            gestures: [[
                Gesture {
                    clip: 3,
                    callback: 0.6,
                },
                Gesture {
                    clip: 2,
                    callback: 0.4,
                },
            ]; 2],
            order: None,
            popped: 0,
            signal_position: None,
            started: None,
            texture: None,
            source: serde_json::Value::Null,
        };
        (e, john, party, tutorial())
    }
    fn progress(
        e: &mut Evacuation,
        john: &mut player::Player,
        party: &mut Party,
        t: &mut Tutorial,
        seconds: f32,
    ) {
        let order = e.order.unwrap();
        let p = if order.member == 1 {
            &mut *john
        } else {
            &mut party.jack
        };
        let clip = order.gesture.unwrap().clip;
        p.action = Some((clip, seconds));
        e.update(john, party, t, false, false);
    }
    #[test]
    fn zone_is_horizontal_inclusive_and_rejects_nonfinite_positions() {
        let (e, _, _, _) = fixture();
        assert!(e.inside(Vec3::new(2.0, 800.0, 0.0)));
        assert!(!e.inside(Vec3::new(2.001, 0.0, 0.0)));
        assert!(!e.inside(Vec3::splat(f32::NAN)));
    }
    #[test]
    fn enabled_objective_is_insufficient_until_every_primary_is_complete() {
        let (mut e, john, party, mut t) = fixture();
        assert!(e.ready(&john, &party, &t, false));
        t.completed.clear();
        assert!(!e.ready(&john, &party, &t, false));
        t.completed.insert("goal".into());
        e.primary.insert("another_primary".into());
        assert!(!e.ready(&john, &party, &t, false));
        t.completed.insert("another_primary".into());
        assert!(e.ready(&john, &party, &t, false));
    }
    #[test]
    fn both_mercenaries_must_be_recruited_in_zone_and_on_foot() {
        let (e, mut john, mut party, t) = fixture();
        assert!(!e.ready(&john, &party, &t, true));
        party.recruited = false;
        assert!(!e.ready(&john, &party, &t, false));
        party.recruited = true;
        party.jack.position.x = 3.0;
        assert!(!e.ready(&john, &party, &t, false));
        party.jack.position.x = 0.0;
        john.position.x = 3.0;
        assert!(!e.ready(&john, &party, &t, false));
    }
    #[test]
    fn only_a_selected_idle_unpaused_member_can_start() {
        let (mut e, mut john, mut party, t) = fixture();
        john.selected = false;
        assert!(!e.request(1, &mut john, &mut party, &t, false));
        john.selected = true;
        john.destination = Some(Vec3::ZERO);
        assert!(!e.request(1, &mut john, &mut party, &t, false));
        john.destination = None;
        john.playback.paused = true;
        assert!(!e.request(1, &mut john, &mut party, &t, false));
        john.playback.paused = false;
        john.action = Some((0, 0.0));
        assert!(!e.request(1, &mut john, &mut party, &t, false));
        john.action = None;
        assert!(!e.request(3, &mut john, &mut party, &t, false));
        assert!(e.request(1, &mut john, &mut party, &t, false));
    }
    #[test]
    fn john_signal_waits_for_original_callback_and_preserves_position() {
        let (mut e, mut john, mut party, mut t) = fixture();
        let pos = john.position;
        assert!(e.request(1, &mut john, &mut party, &t, false));
        progress(&mut e, &mut john, &mut party, &mut t, 0.599);
        assert_eq!(e.popped, 0);
        progress(&mut e, &mut john, &mut party, &mut t, 0.6);
        assert_eq!(e.popped, 1);
        assert_eq!(e.signal_position, Some(pos));
        assert_eq!(
            t.events
                .iter()
                .filter(|v| v.starts_with("CET_SIGNAL_FLARE_POPPED"))
                .count(),
            1
        );
    }
    #[test]
    fn jack_crouch_signal_uses_own_action_without_changing_john_order() {
        let (mut e, mut john, mut party, mut t) = fixture();
        john.destination = Some(Vec3::X);
        let before = john.destination;
        party.jack.posture = Posture::Crouch;
        assert!(e.request(2, &mut john, &mut party, &t, false));
        assert_eq!(party.jack.action, Some((2, 0.0)));
        assert!(john.action.is_none());
        progress(&mut e, &mut john, &mut party, &mut t, 0.4);
        assert_eq!(e.popped, 1);
        assert_eq!(john.destination, before);
    }
    #[test]
    fn canceled_or_replaced_action_before_callback_emits_nothing() {
        for replacement in [None, Some((0, 1.0))] {
            let (mut e, mut john, mut party, mut t) = fixture();
            e.request(1, &mut john, &mut party, &t, false);
            john.action = replacement;
            e.update(&mut john, &mut party, &mut t, false, false);
            assert!(e.order.is_none());
            assert_eq!(e.popped, 0);
            assert!(t.end_campaign.is_none());
        }
    }
    #[test]
    fn leaving_zone_before_callback_cancels_without_signal() {
        let (mut e, mut john, mut party, mut t) = fixture();
        e.request(1, &mut john, &mut party, &t, false);
        party.jack.position.x = 3.0;
        progress(&mut e, &mut john, &mut party, &mut t, 0.7);
        assert_eq!(e.popped, 0);
        assert!(john.action.is_none());
        assert!(e.order.is_none());
    }
    #[test]
    fn leaving_zone_after_callback_does_not_revoke_queued_end() {
        let (mut e, mut john, mut party, mut t) = fixture();
        e.request(1, &mut john, &mut party, &t, false);
        progress(&mut e, &mut john, &mut party, &mut t, 0.7);
        party.jack.position.x = 3.0;
        e.update(&mut john, &mut party, &mut t, false, false);
        t.tick(1.5, false);
        assert_eq!(t.end_campaign.as_deref(), Some("EOC_RETURN_TO_MAIN_MENU"));
    }
    #[test]
    fn signal_finishes_once_and_cannot_be_repeated() {
        let (mut e, mut john, mut party, mut t) = fixture();
        e.request(1, &mut john, &mut party, &t, false);
        progress(&mut e, &mut john, &mut party, &mut t, 1.0);
        assert!(john.action.is_none() && e.order.is_none());
        assert!(!e.request(2, &mut john, &mut party, &t, false));
        e.update(&mut john, &mut party, &mut t, false, false);
        assert_eq!(e.popped, 1);
    }
    #[test]
    fn pause_and_inventory_hold_callback() {
        for inventory in [false, true] {
            let (mut e, mut john, mut party, mut t) = fixture();
            e.request(1, &mut john, &mut party, &t, false);
            john.action = Some((3, 0.8));
            john.playback.paused = !inventory;
            e.update(&mut john, &mut party, &mut t, false, inventory);
            assert_eq!(e.popped, 0);
            john.playback.paused = false;
            e.update(&mut john, &mut party, &mut t, false, false);
            assert_eq!(e.popped, 1);
        }
    }
    #[test]
    fn restart_clears_smoke_and_delayed_end() {
        let (mut e, mut john, mut party, mut t) = fixture();
        e.request(1, &mut john, &mut party, &t, false);
        progress(&mut e, &mut john, &mut party, &mut t, 0.7);
        e.restart();
        t.restart();
        t.tick(10.0, false);
        assert_eq!(e.popped, 0);
        assert!(
            e.signal_position.is_none()
                && e.order.is_none()
                && e.started.is_none()
                && t.end_campaign.is_none()
        );
    }
    #[test]
    fn unrelated_campaign_end_stays_unsupported() {
        let m=MissionDefinition::from_bytes(b"SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_SIGNAL_FLARE_POPPED\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_END_CAMPAIGN\nCER_BODY_IDENTIFIER EOC_ALL_KILLED_RETRY\n").unwrap();
        let help = shadow_formats::HelpDefinitions::from_bytes(
            b"HELP_DEF\nHELP_ID dummy\nHELP_CONTEXT\nHELP_BODY \"x\"\n",
        )
        .unwrap();
        let mut t = Tutorial::new(m, help).unwrap();
        assert_eq!(t.unsupported.len(), 1);
        t.fire(Event::SignalFlarePopped, "");
        t.tick(10.0, false);
        assert!(t.end_campaign.is_none());
    }
    #[test]
    fn original_end_delay_is_relative_to_flare_event() {
        let (mut e, mut john, mut party, mut t) = fixture();
        t.tick(8.0, false);
        e.request(1, &mut john, &mut party, &t, false);
        progress(&mut e, &mut john, &mut party, &mut t, 0.6);
        t.tick(1.499, false);
        assert!(t.end_campaign.is_none());
        t.tick(0.00101, false);
        assert_eq!(t.end_campaign.as_deref(), Some("EOC_RETURN_TO_MAIN_MENU"));
    }
    #[test]
    fn tutorial_pause_holds_scheduled_campaign_end() {
        let mut t = tutorial();
        t.fire(Event::SignalFlarePopped, "");
        t.tick(2.0, true);
        assert!(t.end_campaign.is_none());
        t.tick(1.5, false);
        assert!(t.end_campaign.is_some());
    }
    #[test]
    fn group_selection_still_starts_only_the_clicked_member() {
        let (mut e, mut john, mut party, t) = fixture();
        assert!(e.request(1, &mut john, &mut party, &t, false));
        assert!(!e.request(2, &mut john, &mut party, &t, false));
        assert!(john.action.is_some() && party.jack.action.is_none());
    }
    #[test]
    fn stop_cancels_preparation_and_preserves_replacement_action() {
        let (mut e, mut john, mut party, mut t) = fixture();
        e.order = Some(Order {
            member: 1,
            gesture: None,
        });
        john.action = Some((0, 0.5));
        e.cancel(&mut john, &mut party);
        assert!(e.order.is_none());
        assert_eq!(john.action, Some((0, 0.5)));
        e.update(&mut john, &mut party, &mut t, false, false);
        assert_eq!(e.popped, 0);
    }
}
