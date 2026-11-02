//! Портреты BOTTOM_BAR_FACE из профилей, кадры merc_faces_N из image_defs.
use crate::{
    inventory::Inventory,
    party::{Party, Select},
    player, training,
    tutorial::Tutorial,
    ui_controls::{ButtonVisualState, UiButton},
};
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, ConfigLine, config_lines};
use std::error::Error;

struct Portrait {
    image: Option<Image>,
    rect: Rect,
    handle: Option<Handle<Image>>,
    source: serde_json::Value,
}
#[derive(Resource)]
pub struct Roster {
    portraits: [Portrait; 2],
    last_click: Option<(u8, f64)>,
}
#[derive(Component)]
pub(crate) struct Panel;
#[derive(Component)]
pub(crate) struct Caption;
#[derive(Component)]
pub(crate) struct Health(u8);
#[derive(Debug)]
struct Frame {
    texture: String,
    canvas: Vec2,
    rect: Rect,
}
fn face(properties: &[ConfigLine]) -> Result<(u32, usize), Box<dyn Error>> {
    let entries: Vec<_> = properties
        .iter()
        .filter(|l| l.words[0] == "BOTTOM_BAR_FACE")
        .collect();
    let [line] = entries.as_slice() else {
        return Err("нужно одно поле BOTTOM_BAR_FACE".into());
    };
    if line.words.len() != 3 {
        return Err("BOTTOM_BAR_FACE: нужны страница и кадр".into());
    }
    let page = line.words[1].parse::<u32>()?;
    let index = line.words[2].parse::<usize>()?;
    if !(1..=64).contains(&page) || index >= 256 {
        return Err("BOTTOM_BAR_FACE вне диапазона".into());
    }
    Ok((page, index))
}
fn frame(lines: &[ConfigLine], page: u32, index: usize) -> Result<Frame, Box<dyn Error>> {
    let name = format!("merc_faces_{page}");
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            l.words.first().is_some_and(|w| w == "ANIMSPRITE3D")
                && l.words
                    .get(1)
                    .is_some_and(|w| w.eq_ignore_ascii_case(&name))
        })
        .collect();
    let [(start, header)] = starts.as_slice() else {
        return Err(format!("нужен один атлас {name}").into());
    };
    if header.words.len() != 7 {
        return Err("неподдержанный заголовок ANIMSPRITE3D портрета".into());
    }
    let number = |s: &str| -> Result<u32, Box<dyn Error>> {
        let v = s.parse::<u32>()?;
        if v > 16384 {
            Err("слишком большая координата портрета".into())
        } else {
            Ok(v)
        }
    };
    let width = number(&header.words[3])?;
    let height = number(&header.words[4])?;
    if width == 0 || height == 0 {
        return Err("пустой атлас портрета".into());
    }
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.words[0] == "ENDDEF")
        .ok_or("нет ENDDEF портрета")?
        + start
        + 1;
    let mut frames = Vec::new();
    let mut descriptor = None;
    for l in &lines[start + 1..end] {
        match l.words[0].as_str() {
            "FRAMEDESCRIPTOR" if l.words.len() == 4 && descriptor.is_none() => {
                let n = l.words[1].parse::<usize>()?;
                if n == 0 || n > 256 || l.words[2].parse::<usize>()? != n || l.words[3] != "0" {
                    return Err("неподдержанный FRAMEDESCRIPTOR портрета".into());
                }
                descriptor = Some(n);
            }
            "SPRITEFRAME_XRUN" => {
                if l.words.len() != 6 {
                    return Err("неверный кадр портрета".into());
                }
                let x = number(&l.words[1])?;
                let y = number(&l.words[2])?;
                let w = number(&l.words[3])?;
                let h = number(&l.words[4])?;
                let count = number(&l.words[5])?;
                if w == 0
                    || h == 0
                    || count == 0
                    || count > 256
                    || frames.len() + count as usize > 256
                    || x + w * count > width
                    || y + h > height
                {
                    return Err("кадр портрета вне атласа".into());
                }
                for i in 0..count {
                    frames.push(Rect::new(
                        (x + i * w) as f32,
                        y as f32,
                        (x + (i + 1) * w) as f32,
                        (y + h) as f32,
                    ));
                }
            }
            _ => return Err(format!("неподдержанное поле портрета {}", l.words[0]).into()),
        }
    }
    if descriptor != Some(frames.len()) {
        return Err("число кадров портрета не совпадает с описанием".into());
    }
    Ok(Frame {
        texture: header.words[2].clone(),
        canvas: Vec2::new(width as f32, height as f32),
        rect: *frames.get(index).ok_or("нет кадра портрета")?,
    })
}
fn scaled_rect(frame: &Frame, size: UVec2) -> Result<Rect, Box<dyn Error>> {
    if size.x == 0 || size.y == 0 {
        return Err("пустое изображение портрета".into());
    }
    let scale = size.as_vec2() / frame.canvas;
    Ok(Rect::from_corners(
        frame.rect.min * scale,
        frame.rect.max * scale,
    ))
}
impl Roster {
    pub fn prepare(
        catalog: &AssetCatalog,
        john: &player::Player,
        jack: &player::Player,
    ) -> Result<Self, Box<dyn Error>> {
        let defs = "config/image_defs.txt";
        let lines = config_lines(&catalog.read(defs)?)?;
        let mut portraits = Vec::new();
        for actor in [john, jack] {
            let (path, profile) = player::profile_for(catalog, &actor.name)?;
            let (page, index) = face(&profile.properties)?;
            let frame = frame(&lines, page, index)?;
            let texture = catalog
                .resolve_texture(defs, &frame.texture)?
                .ok_or("не найдена текстура портрета")?;
            let image = crate::asset_mesh::image(catalog, &texture)?.image;
            let rect = scaled_rect(&frame, image.size())?;
            let source = serde_json::json!({"actor":actor.name,"profile":path,"profile_origin":catalog.source_label(&path)?,"page":page,"frame":index,"definition":defs,"definition_origin":catalog.source_label(defs)?,"texture":texture,"texture_origin":catalog.source_label(&texture)?,"canvas":frame.canvas.to_array(),"source_rect":[frame.rect.min.to_array(),frame.rect.max.to_array()],"image_size":image.size().to_array(),"image_rect":[rect.min.to_array(),rect.max.to_array()]});
            portraits.push(Portrait {
                image: Some(image),
                rect,
                handle: None,
                source,
            });
        }
        Ok(Self {
            portraits: portraits.try_into().map_err(|_| "нужны два портрета")?,
            last_click: None,
        })
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"portraits":self.portraits.iter().map(|p|&p.source).collect::<Vec<_>>()})
    }
    pub fn image_handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.portraits.iter().filter_map(|p| p.handle.as_ref())
    }
    fn click(&mut self, member: u8, now: f64) -> bool {
        let double = self
            .last_click
            .is_some_and(|(m, t)| m == member && now >= t && now - t <= 0.4);
        self.last_click = if double { None } else { Some((member, now)) };
        double
    }
}
pub fn setup(
    mut commands: Commands,
    roster: Option<ResMut<Roster>>,
    hud: Option<Res<crate::hud::Hud>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    camera: Single<Entity, With<training::MapCamera>>,
) {
    let (Some(mut roster), Some(hud)) = (roster, hud) else {
        return;
    };
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    let panel = commands
        .spawn((
            player::SceneEntity,
            Panel,
            crate::selection::UiBlock,
            UiTargetCamera(*camera),
            GlobalZIndex(15),
            Pickable::IGNORE,
            Node::default(),
        ))
        .id();
    for (i, portrait) in roster.portraits.iter_mut().enumerate() {
        let handle = images.add(portrait.image.take().expect("portrait setup once"));
        portrait.handle = Some(handle.clone());
        let button = commands
            .spawn((
                Select(i as u8 + 1),
                UiButton,
                Node::default(),
                BorderColor::all(Color::NONE),
                BackgroundColor(Color::NONE),
                ChildOf(panel),
            ))
            .id();
        commands.spawn((
            ImageNode::new(handle)
                .with_rect(portrait.rect)
                .with_mode(NodeImageMode::Stretch),
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(button),
        ));
        commands.spawn((
            Caption,
            Text::new(if i == 0 { "Джон" } else { "Джек" }),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(13.0 * hud.fit.scale),
                ..default()
            },
            TextColor(Color::srgb(0.75, 0.85, 0.64)),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                top: percent(100),
                margin: UiRect::top(px(8.0 * hud.fit.scale)),
                ..default()
            },
            ChildOf(button),
        ));
        let bar = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: percent(100),
                    margin: UiRect::top(px(3.0 * hud.fit.scale)),
                    width: percent(100),
                    height: px(3.0 * hud.fit.scale),
                    ..default()
                },
                BackgroundColor(Color::BLACK),
                Pickable::IGNORE,
                ChildOf(button),
            ))
            .id();
        commands.spawn((
            Health(i as u8 + 1),
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Color::srgb(0.2, 0.8, 0.12)),
            Pickable::IGNORE,
            ChildOf(bar),
        ));
    }
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    roster: Option<ResMut<Roster>>,
    party: Option<ResMut<Party>>,
    john: Option<ResMut<player::Player>>,
    inv: Option<ResMut<Inventory>>,
    t: Option<Res<Tutorial>>,
    vehicle: Option<Res<crate::vehicle::Vehicle>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    buttons: Query<(&Select, &ButtonVisualState)>,
    mut orbit: ResMut<training::Orbit>,
) {
    let (Some(mut roster), Some(mut party), Some(mut john), Some(mut inv), Some(t)) =
        (roster, party, john, inv, t)
    else {
        return;
    };
    if t.active.is_some()
        || t.consumed_input
        || inv.open
        || john.playback.paused
        || [KeyCode::Home, KeyCode::Space, KeyCode::Escape]
            .iter()
            .any(|k| keys.just_pressed(*k))
    {
        return;
    }
    let portrait = buttons
        .iter()
        .find(|(s, b)| s.0 <= 2 && **b != ButtonVisualState::None)
        .map(|(s, b)| (s.0, *b));
    if let Some((member, state)) = portrait {
        if member == 2 && !party.recruited {
            return;
        }
        if mouse.just_pressed(MouseButton::Right) {
            party.select(member, &mut john);
            mouse.reset(MouseButton::Right);
            if member == 1 && !vehicle.as_ref().is_some_and(|v| v.occupied) {
                inv.open = true;
                inv.cancel();
                inv.consumed_input = true;
            }
        }
        if state == ButtonVisualState::Pressed
            && mouse.just_pressed(MouseButton::Left)
            && !keys.pressed(KeyCode::ControlLeft)
            && !keys.pressed(KeyCode::ControlRight)
            && roster.click(member, time.elapsed_secs_f64())
        {
            orbit.target = if member == 1 {
                john.camera_target()
            } else {
                party.jack.camera_target()
            };
        }
    } else if mouse.just_pressed(MouseButton::Left) {
        roster.last_click = None;
    }
}
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn visuals(
    hud: Option<Res<crate::hud::Hud>>,
    john: Option<Res<player::Player>>,
    party: Option<Res<Party>>,
    inv: Option<Res<Inventory>>,
    medical: Option<Res<crate::medical::Medical>>,
    mut panels: Query<
        &mut Node,
        (
            With<Panel>,
            Without<Select>,
            Without<Health>,
            Without<Caption>,
        ),
    >,
    mut buttons: Query<
        (
            &Select,
            &ButtonVisualState,
            &mut Node,
            &mut BorderColor,
            &mut BackgroundColor,
        ),
        (Without<Panel>, Without<Health>, Without<Caption>),
    >,
    mut captions: Query<
        (&mut Node, &mut TextFont),
        (
            With<Caption>,
            Without<Select>,
            Without<Panel>,
            Without<Health>,
        ),
    >,
    mut health: Query<(&Health, &mut Node), (Without<Caption>, Without<Select>, Without<Panel>)>,
) {
    let (Some(h), Some(john), Some(party), Some(inv), Some(m)) = (hud, john, party, inv, medical)
    else {
        return;
    };
    let scale = h.fit.scale;
    for mut node in &mut panels {
        *node = Node {
            position_type: PositionType::Absolute,
            left: px(186.0 * scale),
            bottom: px(0),
            width: px((h.fit.screen.x / scale - 354.0) * scale),
            height: px(125.0 * scale),
            display: if inv.open {
                Display::None
            } else {
                Display::Flex
            },
            ..default()
        };
    }
    for (s, state, mut node, mut border, mut bg) in &mut buttons {
        let f = h.layout.face;
        *node = Node {
            position_type: PositionType::Absolute,
            left: px((f.min.x - 186.0 + (s.0 - 1) as f32 * h.layout.stride) * scale),
            top: px((f.min.y - 70.0) * scale),
            width: px(f.width() * scale),
            height: px(f.height() * scale),
            border: UiRect::all(px(scale)),
            display: if s.0 == 1 || party.recruited {
                Display::Flex
            } else {
                Display::None
            },
            ..default()
        };
        let selected = if s.0 == 1 {
            john.selected
        } else {
            party.jack.selected
        };
        *border = BorderColor::all(if selected {
            Color::srgb(0.45, 0.8, 0.3)
        } else {
            Color::NONE
        });
        bg.0 = if *state == ButtonVisualState::Hovered {
            Color::srgba(0.2, 0.4, 0.1, 0.3)
        } else {
            Color::NONE
        };
    }
    for (mut node, mut font) in &mut captions {
        node.margin.top = px(8.0 * scale);
        font.font_size = FontSize::Px(13.0 * scale);
    }
    for (id, mut node) in &mut health {
        node.width = percent(if id.0 == 1 {
            100.0 * (m.health / m.maximum).clamp(0.0, 1.0)
        } else {
            100.0
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn defs() -> Vec<ConfigLine> {
        config_lines(b"ANIMSPRITE3D merc_faces_2 characters2.bmp 128 128 0.0 1\nFRAMEDESCRIPTOR 6 6 0\nSPRITEFRAME_XRUN 0 0 42 64 3\nSPRITEFRAME_XRUN 0 64 42 64 3\nENDDEF\n").unwrap()
    }
    #[test]
    fn jack_is_middle_of_second_row() {
        let f = frame(&defs(), 2, 4).unwrap();
        assert_eq!(f.rect, Rect::new(42.0, 64.0, 84.0, 128.0));
        assert_eq!(f.texture, "characters2.bmp");
    }
    #[test]
    fn hd_replacements_scale_uv_but_keep_frame() {
        let f = frame(&defs(), 2, 4).unwrap();
        assert_eq!(
            scaled_rect(&f, UVec2::new(1024, 512)).unwrap(),
            Rect::new(336.0, 256.0, 672.0, 512.0)
        );
        assert!(scaled_rect(&f, UVec2::ZERO).is_err());
    }
    #[test]
    fn profile_requires_exact_page_and_frame() {
        assert_eq!(
            face(&config_lines(b"BOTTOM_BAR_FACE 3 0").unwrap()).unwrap(),
            (3, 0)
        );
        for s in [
            "BOTTOM_BAR_FACE 0 0",
            "BOTTOM_BAR_FACE 3 -1",
            "BOTTOM_BAR_FACE 3 256",
            "BOTTOM_BAR_FACE 3",
            "BOTTOM_BAR_FACE 3 0\nBOTTOM_BAR_FACE 3 0",
        ] {
            assert!(face(&config_lines(s.as_bytes()).unwrap()).is_err());
        }
    }
    #[test]
    fn missing_duplicate_and_out_of_range_fail() {
        let mut d = defs();
        assert!(frame(&d, 3, 0).is_err());
        assert!(frame(&d, 2, 6).is_err());
        d.extend(defs());
        assert!(frame(&d, 2, 0).is_err());
    }
    #[test]
    fn corrupt_frames_fail() {
        for (line, word, value) in [
            (2, 3, "0"),
            (2, 5, "0"),
            (2, 5, "300"),
            (2, 1, "125"),
            (3, 2, "65"),
            (1, 1, "5"),
            (1, 2, "5"),
            (0, 3, "0"),
            (0, 3, "NaN"),
        ] {
            let mut d = defs();
            d[line].words[word] = value.into();
            assert!(frame(&d, 2, 0).is_err(), "{line} {word} {value}");
        }
    }
    #[test]
    fn unsupported_opcode_fails() {
        let mut d = defs();
        d[2].words[0] = "UNKNOWN".into();
        assert!(frame(&d, 2, 0).is_err());
    }
    #[test]
    fn portrait_double_click_requires_same_member_and_time() {
        let mut r = Roster {
            portraits: std::array::from_fn(|_| Portrait {
                image: None,
                rect: Rect::default(),
                handle: None,
                source: serde_json::Value::Null,
            }),
            last_click: None,
        };
        assert!(!r.click(1, 1.0));
        assert!(!r.click(2, 1.1));
        assert!(r.click(2, 1.4));
        assert!(!r.click(2, 1.5));
        assert!(!r.click(2, 2.0));
        assert!(!r.click(2, 1.0));
    }
}
