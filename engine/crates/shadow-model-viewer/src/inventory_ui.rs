//! Original inventory bar and responsive three-row pack, using the player's assets.
use crate::{
    hud::{self, Fit},
    hud_layout,
    inventory::{self, Inventory, Slot, Weapon},
    player,
    roster::Roster,
    ui_controls::{ButtonVisualState, UiButton},
};
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, ConfigLine, config_lines};
use std::{collections::BTreeMap, error::Error};
const DEFINITION: &str = "config/window_bases/inventory_bar.txt";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Item {
    Knife,
    Bda,
    Uzi,
    Cutters,
    Grenade,
    Medical,
    Armor,
}
impl Item {
    pub fn weapon(w: Weapon) -> Self {
        match w {
            Weapon::Knife => Self::Knife,
            Weapon::Bda => Self::Bda,
            Weapon::Uzi => Self::Uzi,
            Weapon::Cutters => Self::Cutters,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Knife => "Нож",
            Self::Bda => "BDA",
            Self::Uzi => "UZI",
            Self::Cutters => "Кусачки",
            Self::Grenade => "Осколочная граната",
            Self::Medical => "Аптечка",
            Self::Armor => "Бронежилет",
        }
    }
}
struct Bitmap {
    image: Option<Image>,
    handle: Option<Handle<Image>>,
    size: Vec2,
}
struct Icon {
    bitmap: Bitmap,
    slots: UVec2,
}
pub struct Appearance {
    lines: Vec<ConfigLine>,
    textures: BTreeMap<String, Bitmap>,
    icons: BTreeMap<Item, Icon>,
    pub font: Handle<Font>,
    pub profiles: [Vec<(String, String)>; 2],
}
/// Descriptions are not used by icons; originals contain an unterminated description
/// in outfits.txt. Keep structural fields strict and preserve their source line numbers.
fn icon_profile(bytes: &[u8]) -> Result<Vec<ConfigLine>, Box<dyn Error>> {
    if bytes.len() > 16 * 1024 * 1024 || bytes.contains(&0) {
        return Err("invalid item profile text".into());
    }
    let mut selected = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if line.len() > 16384 {
            return Err("item profile line too long".into());
        }
        let key = line
            .split(|b| b.is_ascii_whitespace())
            .find(|s| !s.is_empty());
        if key == Some(b"OBJECT_DESC_LINE".as_slice()) {
            selected.push(b'\n');
        } else {
            selected.extend_from_slice(line);
        }
    }
    Ok(config_lines(&selected)?)
}
fn skill_name(name: &str) -> &str {
    match name {
        "Handguns" => "Пистолеты",
        "SMG" => "Пистолеты-пулемёты",
        "Assault Weapons" => "Автоматы",
        "Sniper" => "Снайперское оружие",
        "Melee Weapons" => "Ближний бой",
        "Heavy Weapons" => "Тяжёлое оружие",
        "Grenades" => "Гранаты",
        "Perception" => "Восприятие",
        "Stealth" => "Скрытность",
        "Infiltration" => "Проникновение",
        "Demolitions" => "Подрывное дело",
        "Medic" => "Медицина",
        other => other,
    }
}
impl Appearance {
    pub fn prepare(catalog: &AssetCatalog) -> Result<Self, Box<dyn Error>> {
        let all = config_lines(&catalog.read(DEFINITION)?)?;
        let mut lines = Vec::new();
        let mut i = 0;
        // Grid and empty equipment silhouettes are separate layers; keep frame geometry exact.
        while i < all.len() {
            if all[i].words[0] == "WINDOW_BASE_GEOMETRY" {
                let end = all[i + 1..]
                    .iter()
                    .position(|l| l.words[0] == "WINDOW_BASE_GEOMETRY")
                    .map_or(all.len(), |n| i + 1 + n);
                if all[i..end].iter().any(|l| {
                    l.words.first().is_some_and(|v| v == "GEOMETRY_TEXTURE")
                        && l.words.get(1).is_some_and(|v| {
                            matches!(
                                v.as_str(),
                                "invntry1_ck.bmp" | "simu3_ck.bmp" | "screen_black.bmp"
                            )
                        })
                }) {
                    lines.extend_from_slice(&all[i..end]);
                }
                i = end;
            } else {
                lines.push(all[i].clone());
                i += 1;
            }
        }
        let layout = hud_layout::window(&lines, Vec2::new(1600.0, 900.0))?;
        for name in [
            "r_portrait",
            "r_name",
            "r_primary",
            "r_secondary",
            "r_head",
            "r_body",
            "r_inventory",
            "b_merc_prev",
            "b_merc_next",
            "b_skills",
        ] {
            if !layout.advice.iter().any(|a| a.id == name) {
                return Err(format!("missing original inventory region {name}").into());
            }
        }
        let mut textures = BTreeMap::new();
        for name in ["invntry1_ck.bmp", "simu3_ck.bmp", "invntry_background1.bmp"] {
            let mut image =
                crate::asset_mesh::image(catalog, &format!("textures/interface/{name}"))?.image;
            if name.ends_with("_ck.bmp")
                && let Some(data) = &mut image.data
            {
                for p in data.as_chunks_mut::<4>().0 {
                    if p[..3].iter().all(|c| *c <= 2) {
                        p[3] = 0;
                    }
                }
            }
            let size = image.size().as_vec2();
            textures.insert(
                name.into(),
                Bitmap {
                    image: Some(image),
                    handle: None,
                    size,
                },
            );
        }
        let mut icons = BTreeMap::new();
        for (id, file, name) in [
            (Item::Knife, "knives", "Knife"),
            (Item::Bda, "guns", "BDA"),
            (Item::Uzi, "guns", "UZI"),
            (Item::Cutters, "tools", "Fence Cutters"),
            (Item::Grenade, "grenades", "Frag Grenade"),
            (Item::Medical, "tools", "Medical Kit"),
            (Item::Armor, "outfits", "Flak Jacket"),
        ] {
            let path = format!("config/inventory_object_profiles/{file}.txt");
            let profile =
                icon_profile(&catalog.read(&path)?).map_err(|e| format!("{path}: {e}"))?;
            let block = inventory::block(&profile, "OBJECT_NAME", name, "OBJECT_DEF")?;
            let model = inventory::field(block, "OBJECT_MODEL")?;
            let model = player::model_path(catalog, &format!("{model}_I"))
                .or_else(|_| player::model_path(catalog, model))?;
            let slots = if let Some(line) = block.iter().find(|l| l.words[0] == "OBJECT_SLOT_USE") {
                if line.words.len() != 3 {
                    return Err("invalid inventory slot dimensions".into());
                }
                UVec2::new(line.words[1].parse()?, line.words[2].parse()?)
            } else {
                UVec2::ONE
            }; // Grenades inherit the original one-cell default.
            if slots.min_element() == 0 || slots.max_element() > 3 {
                return Err("invalid item slot dimensions".into());
            }
            let image = crate::item_icon::render(
                catalog,
                &model,
                !matches!(id, Item::Medical | Item::Armor | Item::Grenade),
            )?;
            let size = image.size().as_vec2();
            icons.insert(
                id,
                Icon {
                    bitmap: Bitmap {
                        image: Some(image),
                        handle: None,
                        size,
                    },
                    slots,
                },
            );
        }
        let mut profiles = Vec::new();
        for name in ["John", "Jack"] {
            let (_, p) = player::profile_for(catalog, name)?;
            profiles.push(
                p.properties
                    .iter()
                    .filter_map(|l| {
                        if l.words.len() == 3 && l.words[0] == "SKILL" {
                            Some((skill_name(&l.words[1]).into(), l.words[2].clone()))
                        } else {
                            None
                        }
                    })
                    .collect(),
            );
        }
        Ok(Self {
            lines,
            textures,
            icons,
            font: Handle::default(),
            profiles: profiles.try_into().unwrap(),
        })
    }
    pub fn setup(&mut self, images: &mut Assets<Image>, font: Handle<Font>) {
        self.font = font;
        for b in self
            .textures
            .values_mut()
            .chain(self.icons.values_mut().map(|i| &mut i.bitmap))
        {
            b.handle = Some(images.add(b.image.take().expect("inventory images initialized once")));
        }
    }
    pub fn image_handles(&self) -> impl Iterator<Item = &Handle<Image>> {
        self.textures
            .values()
            .chain(self.icons.values().map(|i| &i.bitmap))
            .filter_map(|b| b.handle.as_ref())
    }
    pub fn icon(&self, item: Item) -> ImageNode {
        ImageNode::new(
            self.icons[&item]
                .bitmap
                .handle
                .clone()
                .expect("inventory icon uploaded"),
        )
        .with_mode(NodeImageMode::Stretch)
    }
    fn texture(&self, name: &str, rect: Rect, canvas: Vec2) -> ImageNode {
        let b = &self.textures[name];
        ImageNode::new(b.handle.clone().expect("inventory texture uploaded"))
            .with_rect(Rect::from_corners(
                rect.min * b.size / canvas,
                rect.max * b.size / canvas,
            ))
            .with_mode(NodeImageMode::Stretch)
    }
}
#[derive(Component)]
pub(crate) struct Hover(String);
#[derive(Component)]
pub(crate) struct Hint;
#[derive(Component)]
#[allow(dead_code)] // Read by the shadow-game UI smoke; the map viewer shares this renderer.
pub(crate) struct ItemImage(pub Item);
#[allow(clippy::too_many_arguments)]
fn text(
    commands: &mut Commands,
    parent: Entity,
    a: &Appearance,
    fit: Fit,
    rect: Rect,
    value: impl Into<String>,
    color: Color,
    size: f32,
) -> Entity {
    commands
        .spawn((
            Text::new(value),
            TextFont {
                font: a.font.clone().into(),
                font_size: FontSize::Px((size * fit.scale).max(11.0)),
                ..default()
            },
            TextColor(color),
            fit.node(rect),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id()
}
fn button(
    commands: &mut Commands,
    parent: Entity,
    fit: Fit,
    rect: Rect,
    slot: Slot,
    hint: &str,
) -> Entity {
    commands
        .spawn((
            UiButton,
            slot,
            Hover(hint.into()),
            fit.node(rect),
            BackgroundColor(Color::NONE),
            ChildOf(parent),
        ))
        .id()
}
fn icon(commands: &mut Commands, parent: Entity, a: &Appearance, fit: Fit, rect: Rect, item: Item) {
    let size = a.icons[&item].bitmap.size;
    let scale = ((rect.size() - Vec2::splat(6.0)) / size).min_element();
    let r = Rect::from_center_size(rect.center(), size * scale);
    commands.spawn((
        ItemImage(item),
        a.icon(item),
        fit.node(r),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
}
fn amount(commands: &mut Commands, parent: Entity, a: &Appearance, fit: Fit, rect: Rect, n: u32) {
    text(
        commands,
        parent,
        a,
        fit,
        Rect::new(
            rect.min.x + 2.0,
            rect.min.y + 1.0,
            rect.max.x,
            rect.min.y + 18.0,
        ),
        n.to_string(),
        Color::srgb(0.25, 0.8, 1.0),
        12.0,
    );
}
/// Stable first-fit packing respects OBJECT_SLOT_USE, including the 2x2 medical kit.
pub fn pack(dimensions: &[UVec2], columns: usize) -> Vec<UVec2> {
    let mut used = vec![false; columns * 3];
    let mut result = Vec::new();
    for d in dimensions {
        let mut found = None;
        for y in 0..=3 - d.y as usize {
            for x in 0..=columns - d.x as usize {
                if (y..y + d.y as usize)
                    .all(|yy| (x..x + d.x as usize).all(|xx| !used[yy * columns + xx]))
                {
                    found = Some(UVec2::new(x as u32, y as u32));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if let Some(p) = found {
            for y in p.y..p.y + d.y {
                for x in p.x..p.x + d.x {
                    used[y as usize * columns + x as usize] = true;
                }
            }
            result.push(p);
        } else {
            result.push(UVec2::MAX);
        }
    }
    result
}
#[allow(clippy::too_many_arguments)]
pub fn draw(
    commands: &mut Commands,
    parent: Entity,
    a: &Appearance,
    h: &hud::Hud,
    inv: &Inventory,
    john: &player::Player,
    roster: &Roster,
    party: Option<&crate::party::Party>,
    grenade: Option<&crate::grenade::Grenade>,
    medical: Option<&crate::medical::Medical>,
    armor: Option<&crate::armor::Armor>,
) {
    let fit = h.fit;
    let layout =
        hud_layout::window(&a.lines, fit.screen / fit.scale).expect("validated inventory layout");
    let r = |id| layout.advice(id).rect;
    let is_john = inv.view_member == 1;
    let actor = if is_john {
        john
    } else {
        &party.expect("Jack inventory requires recruited party").jack
    };
    let blue = Color::srgb(0.25, 0.8, 1.0);
    for q in &layout.quads {
        let mut node = commands.spawn((fit.node(q.rect), Pickable::IGNORE, ChildOf(parent)));
        if let Some(mut uv) = q.uv {
            if q.tile_x {
                uv.max.x = uv.min.x + q.rect.width();
            }
            node.insert(a.texture(&q.texture, uv, q.canvas));
        } else if q.texture == "screen_black.bmp" {
            node.insert(BackgroundColor(Color::srgba(0., 0., 0., 0.94)));
        }
    }
    let face = button(
        commands,
        parent,
        fit,
        r("r_portrait"),
        Slot::Patient,
        "Здоровье выбранного бойца",
    );
    commands.spawn((
        roster.image(inv.view_member),
        Node {
            width: percent(100),
            height: percent(100),
            ..default()
        },
        Pickable::IGNORE,
        ChildOf(face),
    ));
    let health = if is_john {
        medical.map(|m| m.health / m.maximum).unwrap_or(1.0)
    } else {
        1.0
    };
    let fr = r("r_portrait");
    commands.spawn((
        fit.node(Rect::new(
            fr.min.x,
            fr.max.y + 1.0,
            fr.min.x + fr.width() * health.clamp(0.0, 1.0),
            fr.max.y + 4.0,
        )),
        BackgroundColor(Color::srgb(0.15, 0.9, 0.1)),
        Pickable::IGNORE,
        ChildOf(parent),
    ));
    text(
        commands,
        parent,
        a,
        fit,
        r("r_name"),
        actor.display_name.to_uppercase(),
        blue,
        12.0,
    );
    for (name, slot, index) in [
        ("b_merc_prev", Slot::Previous, 0),
        ("b_merc_next", Slot::Next, 1),
        ("b_skills", Slot::Skills, 2),
    ] {
        let b = button(
            commands,
            parent,
            fit,
            r(name),
            slot,
            if index == 2 {
                "Характеристики бойца"
            } else {
                "Предыдущий / следующий боец"
            },
        );
        let uv = [
            Rect::new(77., 91., 100., 115.),
            Rect::new(101., 91., 124., 115.),
            Rect::new(112., 122., 144., 154.),
        ][index];
        commands.spawn((
            a.texture("invntry1_ck.bmp", uv, Vec2::splat(256.0)),
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(b),
        ));
    }
    for (name, uv) in [
        ("r_primary", Rect::new(0., 0., 128., 42.)),
        ("r_secondary", Rect::new(0., 42., 128., 84.)),
        ("r_head", Rect::new(0., 85., 64., 126.)),
        ("r_body", Rect::new(64., 85., 128., 126.)),
    ] {
        let mut silhouette = a.texture("invntry_background1.bmp", uv, Vec2::splat(128.0));
        silhouette.color = Color::srgba(0.5, 0.5, 0.5, 1.0);
        commands.spawn((
            silhouette,
            fit.node(r(name)),
            Pickable::IGNORE,
            ChildOf(parent),
        ));
    }
    button(
        commands,
        parent,
        fit,
        h.layout.advice("b_inventory").rect,
        Slot::Close,
        "Инвентарь · Space: закрыть",
    );
    let primary = button(
        commands,
        parent,
        fit,
        r("r_primary"),
        Slot::Primary,
        "Основное оружие · перетащи предмет из рюкзака",
    );
    if is_john && let Some(w) = inv.weapon(john) {
        let item = Item::weapon(w);
        icon(commands, parent, a, fit, r("r_primary"), item);
        if let Some(n) = rounds(inv, john) {
            amount(commands, parent, a, fit, r("r_primary"), n);
        }
        commands.entity(primary).insert(Hover(item.name().into()));
    }
    if is_john && let Some(g) = grenade.filter(|g| g.secondary && g.count > 0) {
        icon(commands, parent, a, fit, r("r_secondary"), Item::Grenade);
        amount(commands, parent, a, fit, r("r_secondary"), g.count);
    }
    if is_john {
        let body = button(
            commands,
            parent,
            fit,
            r("r_body"),
            Slot::BodyArmor,
            "Бронежилет · ЛКМ: надеть / снять",
        );
        if armor.is_some_and(|a| a.worn) {
            icon(commands, parent, a, fit, r("r_body"), Item::Armor);
        }
        commands
            .entity(body)
            .insert(Hover("Тело · бронежилет".into()));
    }
    let source_grid = r("r_inventory");
    let cell = source_grid.size() / Vec2::new(4., 3.);
    let columns = ((fit.screen.x / fit.scale - 184.0 - source_grid.min.x) / cell.x)
        .floor()
        .max(4.0) as usize;
    for y in 0..3 {
        for x in 0..columns {
            let p = source_grid.min + cell * Vec2::new(x as f32, y as f32);
            let cell_rect = Rect::from_corners(p, p + cell);
            let b = button(
                commands,
                parent,
                fit,
                cell_rect,
                Slot::Backpack,
                "Рюкзак · перетащи оружие в основной слот",
            );
            commands.entity(b).insert((
                Node {
                    border: UiRect::all(px(fit.scale.max(0.7))),
                    ..fit.node(cell_rect)
                },
                BorderColor::all(Color::srgba(0.2, 0.29, 0.14, 0.7)),
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.94)),
            ));
        }
    }
    let mut items = Vec::new();
    if is_john {
        for w in [Weapon::Knife, Weapon::Bda, Weapon::Uzi, Weapon::Cutters] {
            if inv.owns(w) && inv.weapon(john) != Some(w) {
                items.push((
                    Item::weapon(w),
                    Slot::Stored(w),
                    match w {
                        Weapon::Bda => Some(inv.rounds),
                        Weapon::Uzi => Some(inv.uzi_rounds),
                        _ => None,
                    },
                ));
            }
        }
        if medical.is_some_and(|m| m.ground.collected) {
            items.push((Item::Medical, Slot::Medical, medical.map(|m| m.uses)));
        }
        if armor.is_some_and(|a| a.ground.collected && !a.worn) {
            items.push((Item::Armor, Slot::BodyArmor, None));
        }
    }
    let positions = pack(
        &items
            .iter()
            .map(|(i, _, _)| a.icons[i].slots)
            .collect::<Vec<_>>(),
        columns,
    );
    for ((item, slot, count), p) in items.into_iter().zip(positions) {
        assert_ne!(p, UVec2::MAX, "tutorial inventory exceeds responsive pack");
        let rect = Rect::from_corners(
            source_grid.min + p.as_vec2() * cell,
            source_grid.min + (p + a.icons[&item].slots).as_vec2() * cell,
        );
        button(commands, parent, fit, rect, slot, item.name());
        icon(commands, parent, a, fit, rect, item);
        if let Some(n) = count {
            amount(commands, parent, a, fit, rect, n);
        }
    }
    // Space and the original right-hand inventory button close the bar; no modal screen.
    if inv.skills {
        let x = source_grid.min.x;
        let rect = Rect::new(x, -240.0, x + 500.0, 50.0);
        commands.spawn((
            fit.node(rect),
            BackgroundColor(Color::srgba(0.01, 0.03, 0.01, 0.94)),
            Pickable::IGNORE,
            ChildOf(parent),
        ));
        let mut lines = vec![actor.display_name.clone()];
        if is_john && let Some(m) = medical {
            lines.push(format!("Здоровье: {:.0}/{:.0}", m.health, m.maximum));
        }
        lines.extend(
            a.profiles[(inv.view_member - 1) as usize]
                .iter()
                .map(|(k, v)| format!("{k}: {v}")),
        );
        text(commands, parent, a, fit, rect, lines.join("\n"), blue, 15.0);
    }
    let hint = text(
        commands,
        parent,
        a,
        fit,
        Rect::new(source_grid.min.x, 38., fit.screen.x / fit.scale - 190., 60.),
        if let Some(w) = inv.drag {
            format!("Перенос: {} → основной слот", w.name())
        } else {
            "Инвентарь · Space: закрыть · оружие: перетащить · аптечка: ПКМ".into()
        },
        Color::srgb(0.8, 0.76, 0.48),
        13.0,
    );
    commands.entity(hint).insert(Hint);
}
pub fn rounds(inv: &Inventory, actor: &player::Player) -> Option<u32> {
    match inv.weapon(actor) {
        Some(Weapon::Bda) => Some(inv.rounds),
        Some(Weapon::Uzi) => Some(inv.uzi_rounds),
        _ => None,
    }
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    mut inv: Option<ResMut<Inventory>>,
    party: Option<Res<crate::party::Party>>,
    john: Option<Res<player::Player>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    buttons: Query<(&Slot, &ButtonVisualState)>,
    t: Option<Res<crate::tutorial::Tutorial>>,
) {
    let (Some(inv), Some(john)) = (&mut inv, john) else {
        return;
    };
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input) || john.playback.paused {
        return;
    }
    if !inv.open {
        if keys.just_pressed(KeyCode::Space)
            && !john.selected
            && party
                .as_ref()
                .is_some_and(|p| p.recruited && p.jack.selected)
        {
            inv.view_member = 2;
            inv.open = true;
            inv.consumed_input = true;
            keys.reset(KeyCode::Space);
        }
        return;
    }
    let pressed = |s| {
        mouse.just_pressed(MouseButton::Left)
            && buttons
                .iter()
                .any(|(slot, state)| *slot == s && *state == ButtonVisualState::Pressed)
    };
    if keys.just_pressed(KeyCode::Escape)
        || pressed(Slot::Close)
        || (inv.view_member == 2 && keys.just_pressed(KeyCode::Space))
    {
        inv.open = false;
        inv.drag = None;
        inv.skills = false;
        inv.consumed_input = true;
        keys.reset(KeyCode::Space);
        keys.reset(KeyCode::Escape);
        return;
    }
    if pressed(Slot::Previous) || pressed(Slot::Next) {
        inv.view_member = if inv.view_member == 1 && party.as_ref().is_some_and(|p| p.recruited) {
            2
        } else {
            1
        };
        inv.skills = false;
        inv.drag = None;
        inv.consumed_input = true;
    }
    if pressed(Slot::Skills) || keys.just_pressed(KeyCode::F2) {
        inv.skills = !inv.skills;
        keys.reset(KeyCode::F2);
    }
}
#[allow(clippy::too_many_arguments)]
pub fn ui(
    mut commands: Commands,
    inv: Option<Res<Inventory>>,
    actor: Option<Res<player::Player>>,
    panels: Query<Entity, With<inventory::InventoryPanel>>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
    hud: Option<Res<hud::Hud>>,
    roster: Option<Res<Roster>>,
    party: Option<Res<crate::party::Party>>,
    grenade: Option<Res<crate::grenade::Grenade>>,
    medical: Option<Res<crate::medical::Medical>>,
    armor: Option<Res<crate::armor::Armor>>,
    mut previous: Local<Option<Vec<u64>>>,
) {
    let (Some(inv), Some(actor), Some(h), Some(roster)) = (inv, actor, hud, roster) else {
        return;
    };
    let Some(a) = &h.inventory else { return };
    let state = vec![
        inv.open as u64,
        inv.view_member as u64,
        inv.skills as u64,
        inv.owns_knife as u64,
        inv.collected as u64,
        inv.owns_uzi as u64,
        inv.owns_cutters as u64,
        inv.rounds as u64,
        inv.uzi_rounds as u64,
        inv.weapon(&actor).map_or(0, |w| w as u64 + 1),
        inv.drag.map_or(0, |w| w as u64 + 1),
        grenade.as_ref().map_or(0, |g| g.count as u64),
        grenade.as_ref().is_some_and(|g| g.secondary) as u64,
        medical.as_ref().map_or(0, |m| m.uses as u64),
        medical.as_ref().is_some_and(|m| m.ground.collected) as u64,
        medical.as_ref().map_or(0, |m| m.health.to_bits() as u64),
        armor.as_ref().is_some_and(|a| a.worn) as u64,
        armor.as_ref().is_some_and(|a| a.ground.collected) as u64,
        h.fit.screen.x.to_bits() as u64,
        h.fit.screen.y.to_bits() as u64,
    ];
    if previous.as_ref() == Some(&state) && (!inv.open || !panels.is_empty()) {
        return;
    }
    *previous = Some(state);
    for e in &panels {
        commands.entity(e).despawn();
    }
    if !inv.open {
        return;
    }
    let parent = commands
        .spawn((
            player::SceneEntity,
            inventory::InventoryPanel,
            UiTargetCamera(*camera),
            GlobalZIndex(16),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                bottom: px(0),
                width: percent(100),
                height: px(195.0 * h.fit.scale),
                ..default()
            },
        ))
        .id();
    draw(
        &mut commands,
        parent,
        a,
        &h,
        &inv,
        &actor,
        &roster,
        party.as_deref(),
        grenade.as_deref(),
        medical.as_deref(),
        armor.as_deref(),
    );
}
pub fn hover(
    inv: Option<Res<Inventory>>,
    mut buttons: Query<(&Hover, &Slot, &ButtonVisualState, &mut BackgroundColor)>,
    mut hints: Query<&mut Text, With<Hint>>,
) {
    let Some(inv) = inv.filter(|i| i.open) else {
        return;
    };
    let mut hint = None;
    for (h, slot, state, mut bg) in &mut buttons {
        if *state != ButtonVisualState::None {
            bg.0 = Color::srgba(0.25, 0.45, 0.1, 0.25);
            hint = Some(h.0.clone());
        } else {
            bg.0 = if *slot == Slot::Backpack {
                Color::srgba(0., 0., 0., 0.94)
            } else {
                Color::NONE
            };
        }
    }
    for mut text in &mut hints {
        **text = if let Some(w) = inv.drag {
            format!("Перенос: {} → основной слот", w.name())
        } else {
            hint.clone().unwrap_or_else(|| {
                "Инвентарь · Space: закрыть · оружие: перетащить · аптечка: ПКМ".into()
            })
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_unused_description_does_not_hide_equipment() {
        let bytes=b"OBJECT_DEF outfit\nOBJECT_NAME \"Flak Jacket\"\nOBJECT_DESC_LINE \"original missing quote\nOBJECT_MODEL Vest\nOBJECT_SLOT_USE 2 2\n";
        let profile = icon_profile(bytes).unwrap();
        assert_eq!(profile[2].words, ["OBJECT_MODEL", "Vest"]);
        assert_eq!(profile[2].number, 4);
        assert!(icon_profile(b"OBJECT_MODEL \"broken\n").is_err());
        assert!(icon_profile(b"OBJECT_NAME Vest\0").is_err());
    }
    #[test]
    fn multi_cell_items_pack_without_overlap() {
        let dimensions = [
            UVec2::new(2, 2),
            UVec2::new(2, 1),
            UVec2::new(1, 1),
            UVec2::new(1, 1),
        ];
        let result = pack(&dimensions, 4);
        let mut occupied = std::collections::BTreeSet::new();
        for (p, d) in result.into_iter().zip(dimensions) {
            assert_ne!(p, UVec2::MAX);
            assert!(p.x + d.x <= 4 && p.y + d.y <= 3);
            for y in p.y..p.y + d.y {
                for x in p.x..p.x + d.x {
                    assert!(occupied.insert((x, y)));
                }
            }
        }
        assert_eq!(occupied.len(), 8);
    }
    #[test]
    fn weapon_counter_tracks_selected_magazine_not_shots_or_capacity() {
        let mut inv = crate::inventory::tests::inventory();
        let (_, mut actor, _, _) = crate::pickup::tests::fixture();
        inv.collected = true;
        inv.rounds = 7;
        inv.owns_uzi = true;
        inv.uzi_rounds = 23;
        assert!(inv.equip(Weapon::Bda, &mut actor));
        assert_eq!(rounds(&inv, &actor), Some(7));
        inv.take_round_for(Weapon::Bda);
        assert_eq!(rounds(&inv, &actor), Some(6));
        assert!(inv.equip(Weapon::Knife, &mut actor));
        assert_eq!(rounds(&inv, &actor), None);
    }
}
