//! Inspect the actual native UI layout at HD, Full HD and ultrawide sizes.
use super::*;
use crate::{
    hud::{ClockText, Hud},
    inventory::{Inventory, Slot},
    inventory_ui::ItemImage,
};
use bevy::window::PrimaryWindow;
const CASES: [([u32; 2], f64); 5] = [
    ([1280, 720], 0.0),
    ([1920, 1080], 65.0),
    ([2560, 1080], 5999.0),
    ([1280, 720], 6000.0),
    ([1920, 1080], 86400.0),
];
#[derive(Resource)]
pub(super) struct Smoke {
    path: PathBuf,
    stage: u8,
    case: usize,
    frames: u32,
    changed: u32,
    since: std::time::Instant,
    checks: Vec<serde_json::Value>,
}
impl Smoke {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            stage: 0,
            case: 0,
            frames: 0,
            changed: 0,
            since: std::time::Instant::now(),
            checks: vec![],
        }
    }
    fn next(&mut self, stage: u8) {
        self.stage = stage;
        self.changed = self.frames;
        self.since = std::time::Instant::now();
    }
}
fn bounds(world: &World, entity: Entity) -> Rect {
    let node = world.get::<ComputedNode>(entity).unwrap();
    let (_, _, center) = world
        .get::<UiGlobalTransform>(entity)
        .unwrap()
        .to_scale_angle_translation();
    Rect::from_center_size(
        center * node.inverse_scale_factor(),
        node.size() * node.inverse_scale_factor(),
    )
}
fn preserves_aspect(world: &World, entity: Entity, rect: Rect) -> bool {
    let image = world.get::<ImageNode>(entity).unwrap();
    let size = world
        .resource::<Assets<Image>>()
        .get(&image.image)
        .unwrap()
        .size()
        .as_vec2();
    let ratio = size.x / size.y;
    // Each physical edge may round independently. Compare to this actual,
    // already cropped item thumbnail rather than assuming a common icon shape.
    let pixel = world
        .get::<ComputedNode>(entity)
        .unwrap()
        .inverse_scale_factor();
    (rect.width() - ratio * rect.height()).abs() <= (1.0 + ratio) * pixel + 0.01
}
fn snapshot(world: &mut World, path: PathBuf) {
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}
fn hud(world: &mut World) -> serde_json::Value {
    let clock = world
        .query_filtered::<Entity, With<ClockText>>()
        .single(world)
        .unwrap();
    let parent = world.get::<ChildOf>(clock).unwrap().parent();
    let frame = bounds(world, parent);
    let label = bounds(world, clock);
    assert!(
        frame.center().distance(label.center()) < 1.0,
        "clock not centered: {frame:?}/{label:?}"
    );
    let layout = world.get::<bevy::text::TextLayoutInfo>(clock).unwrap();
    assert!(
        layout.glyphs.iter().all(|g| g.line_index == 0),
        "clock wrapped"
    );
    assert!(
        label.width() <= frame.width() + 1.0 && label.height() <= frame.height() + 1.0,
        "clock exceeds window"
    );
    let h = world.resource::<Hud>();
    let expected = h.fit.rect(h.layout.advice("r_clock").rect);
    assert!(frame.center().distance(expected.center()) < 1.0);
    let scale = h.fit.scale;
    let face_height = h.layout.face.height();
    let commands: Vec<_> = world
        .query::<(Entity, &crate::hud::CommandButton)>()
        .iter(world)
        .map(|(e, b)| (e, b.0.clone()))
        .collect();
    let mut count = 0;
    for (e, id) in commands {
        let h = world.resource::<Hud>();
        let a = h.layout.advice(&id);
        if a.frame.is_none() {
            continue;
        }
        let target = h.fit.rect(a.rect);
        let actual = bounds(world, e);
        assert!(
            actual.center().distance(target.center()) < 1.0,
            "{id} moved out of its source window"
        );
        count += 1;
    }
    assert_eq!(count, 16);
    let pictures: Vec<_> = world
        .query::<(Entity, &crate::roster::WeaponPicture)>()
        .iter(world)
        .filter(|(_, p)| p.0 == 1)
        .map(|(e, p)| (e, p.1))
        .collect();
    let mut picture_count = 0;
    for (e, secondary) in pictures {
        let actual = bounds(world, e);
        if actual.size().min_element() <= 0.0 {
            continue;
        }
        let portrait = bounds(world, world.get::<ChildOf>(e).unwrap().parent());
        let center = portrait.min
            + Vec2::splat(scale)
            + Vec2::new(8.0, face_height + if secondary { 17.0 } else { 3.0 } + 6.5) * scale;
        assert!(
            actual.center().distance(center) < 1.0,
            "portrait icon not centered in its row"
        );
        assert!(
            preserves_aspect(world, e, actual),
            "portrait icon stretched: {actual:?}"
        );
        picture_count += 1;
    }
    assert_eq!(picture_count, 2);
    serde_json::json!({"clock":world.get::<Text>(clock).unwrap().0,"clock_frame":[frame.min.to_array(),frame.max.to_array()],
        "clock_label":[label.min.to_array(),label.max.to_array()],"command_icons":count,"portrait_icons":picture_count})
}
fn inventory(world: &mut World) -> usize {
    let icons: Vec<_> = world
        .query::<(Entity, &ItemImage)>()
        .iter(world)
        .map(|(e, i)| (e, i.0, i.1))
        .collect();
    assert!(
        icons.len() >= 7,
        "expected all item kinds in inventory fixture; actual: {:?}",
        icons.iter().map(|(_, item, _)| item).collect::<Vec<_>>()
    );
    for (entity, item, slot) in &icons {
        let target = world.resource::<Hud>().fit.rect(*slot);
        let actual = bounds(world, *entity);
        assert!(
            actual.center().distance(target.center()) < 1.0,
            "{item:?} not centered"
        );
        assert!(
            preserves_aspect(world, *entity, actual),
            "{item:?} distorted: {actual:?}"
        );
    }
    let controls: Vec<_> = world
        .query::<(Entity, &Slot, &Children)>()
        .iter(world)
        .filter(|(_, s, _)| matches!(s, Slot::Previous | Slot::Next | Slot::Skills))
        .map(|(e, _, c)| (e, c.iter().collect::<Vec<_>>()))
        .collect();
    assert_eq!(controls.len(), 3);
    for (parent, children) in controls {
        for child in children {
            if world.get::<ImageNode>(child).is_some() {
                assert!(
                    bounds(world, child)
                        .center()
                        .distance(bounds(world, parent).center())
                        < 1.0
                );
            }
        }
    }
    icons.len()
}
pub(super) fn run(world: &mut World) {
    let mut s = world.remove_resource::<Smoke>().unwrap();
    s.frames += 1;
    assert!(
        s.since.elapsed().as_secs() < 180,
        "HUD check timed out at {}/{}",
        s.case,
        s.stage
    );
    if let Some(mut t) = world.get_resource_mut::<crate::tutorial::Tutorial>() {
        if t.active.is_some() {
            t.close();
        }
        t.consumed_input = false;
        t.clock = CASES[s.case].1;
    }
    let ready = s.frames - s.changed > 12 && s.since.elapsed().as_secs_f32() > 1.0;
    match s.stage {
        0 if ready && *world.resource::<State<Screen>>().get() == Screen::Main => {
            world.write_message(ActionRequest(Action::Training));
            s.next(1);
        }
        1 if ready
            && world.contains_resource::<Hud>()
            && *world.resource::<State<Screen>>().get() == Screen::Training =>
        {
            world.resource_mut::<player::Player>().playback.paused = true;
            world.resource_mut::<crate::pickup::Pickup>().collected = true;
            let model = {
                let mut i = world.resource_mut::<Inventory>();
                i.owns_knife = true;
                i.collected = true;
                i.owns_uzi = true;
                i.owns_cutters = true;
                i.rounds = 7;
                i.uzi_rounds = 23;
                i.gun.model.clone()
            };
            world.resource_mut::<player::Player>().weapon = Some(model);
            let mut g = world.resource_mut::<crate::grenade::Grenade>();
            g.collected = true;
            g.secondary = true;
            g.count = 3;
            let mut m = world.resource_mut::<crate::medical::Medical>();
            m.ground.collected = true;
            m.uses = 4;
            let mut a = world.resource_mut::<crate::armor::Armor>();
            a.ground.collected = true;
            a.worn = true;
            s.next(2);
        }
        2 if ready => {
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            let size = CASES[s.case].0;
            world
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set(size[0] as f32, size[1] as f32);
            world.resource_mut::<Inventory>().open = false;
            s.next(3);
        }
        3 if ready => {
            let mut result = hud(world);
            result["window"] = serde_json::json!(CASES[s.case].0);
            s.checks.push(result);
            snapshot(
                world,
                s.path.parent().unwrap().join(format!("hud-{}.png", s.case)),
            );
            s.next(4);
        }
        4 if ready => {
            world.resource_mut::<Inventory>().open = true;
            s.next(5);
        }
        5 if ready => {
            let count = inventory(world);
            s.checks.last_mut().unwrap()["inventory_icons"] = serde_json::json!(count);
            snapshot(
                world,
                s.path
                    .parent()
                    .unwrap()
                    .join(format!("inventory-{}.png", s.case)),
            );
            s.next(6);
        }
        6 if ready => {
            if s.case + 1 < CASES.len() {
                s.case += 1;
                s.next(2);
            } else {
                std::fs::write(&s.path,serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","cases":s.checks,"states_seeded":true})).unwrap()).unwrap();
                world.write_message(AppExit::Success);
            }
        }
        _ => {}
    }
    world.insert_resource(s);
}
