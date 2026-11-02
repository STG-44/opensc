//! Рамка в логических координатах окна. Щелчок передаётся миру после отпускания.
use crate::{inventory::Inventory, party::Party, player, training, tutorial::Tutorial};
use bevy::prelude::*;

const THRESHOLD: f32 = 6.0;
#[derive(Clone, Copy, Debug)]
struct Gesture {
    start: Vec2,
    end: Vec2,
    additive: bool,
    original: u8,
    dragged: bool,
    synthetic: bool,
}
impl Gesture {
    fn update(&mut self, cursor: Vec2) {
        self.end = cursor;
        self.dragged |= self.start.distance(cursor) >= THRESHOLD;
    }
    fn rect(self) -> Rect {
        Rect::from_corners(self.start, self.end)
    }
    fn result(self, inside: u8) -> u8 {
        inside | if self.additive { self.original } else { 0 }
    }
}
#[derive(Resource, Default)]
pub struct Selection {
    gesture: Option<Gesture>,
    pub click_cursor: Option<Vec2>,
    pub world_blocked: bool,
    pub test_cursor: Option<Vec2>,
}
/// Прямоугольник панели перехватывает мышь, включая промежутки между кнопками.
#[derive(Component)]
pub struct UiBlock;
#[derive(Component)]
pub(crate) struct Overlay;

#[allow(dead_code)] // Вызывается при смене экранов в shadow-game.
pub fn cancel(selection: Option<ResMut<Selection>>) {
    if let Some(mut selection) = selection {
        selection.gesture = None;
        selection.click_cursor = None;
    }
}
pub fn setup(mut commands: Commands, camera: Single<Entity, With<training::MapCamera>>) {
    commands.spawn((
        player::SceneEntity,
        Overlay,
        UiTargetCamera(*camera),
        GlobalZIndex(200),
        bevy::picking::Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            border: UiRect::all(px(1)),
            ..default()
        },
        BorderColor::all(Color::srgb(0.4, 1.0, 0.55)),
        BackgroundColor(Color::srgba(0.2, 0.8, 0.4, 0.12)),
    ));
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn input(
    selection: Option<ResMut<Selection>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    john: Option<ResMut<player::Player>>,
    party: Option<ResMut<Party>>,
    tutorial: Option<Res<Tutorial>>,
    inventory: Option<Res<Inventory>>,
    preview: Res<training::Preview>,
    view: (
        Single<&Window>,
        Single<(&Camera, &GlobalTransform), With<training::MapCamera>>,
    ),
    blocks: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
            &Node,
        ),
        With<UiBlock>,
    >,
    buttons: Query<&crate::ui_controls::ButtonVisualState, With<crate::ui_controls::UiButton>>,
    sources: (
        Option<Res<crate::gate::Gate>>,
        Option<Res<crate::vehicle::Vehicle>>,
        Option<Res<crate::armor::Armor>>,
        Option<Res<crate::medical::Medical>>,
        Option<Res<crate::pickup::Smoke>>,
    ),
) {
    let (Some(mut selection), Some(mut john), Some(mut party), Some(t), Some(inv)) =
        (selection, john, party, tutorial, inventory)
    else {
        return;
    };
    selection.click_cursor = None;
    let (window, camera) = view;
    let injected = selection
        .test_cursor
        .or(party.test_cursor)
        .or_else(|| sources.0.as_ref().and_then(|s| s.test_cursor))
        .or_else(|| sources.1.as_ref().and_then(|s| s.test_cursor))
        .or_else(|| sources.2.as_ref().and_then(|s| s.test_cursor))
        .or_else(|| sources.3.as_ref().and_then(|s| s.test_cursor))
        .or_else(|| sources.4.as_ref().and_then(|s| s.cursor));
    let synthetic = injected.is_some();
    // Старые фикстуры сбрасывают кнопку и тестовый курсор на следующем кадре.
    let fixture_release = selection
        .gesture
        .filter(|g| g.synthetic && !mouse.pressed(MouseButton::Left))
        .map(|g| g.end);
    let cursor = injected
        .or(fixture_release)
        .or_else(|| window.cursor_position());
    let ui = |cursor: Vec2| {
        blocks.iter().any(|(node, tr, visible, style)| {
            visible.get()
                && style.display != Display::None
                && node.contains_point(*tr, cursor * window.scale_factor())
        })
    };
    selection.world_blocked = cursor.is_some_and(ui)
        || buttons
            .iter()
            .any(|s| *s == crate::ui_controls::ButtonVisualState::Pressed);
    let blocked = (!window.focused && !synthetic && fixture_release.is_none())
        || t.active.is_some()
        || t.consumed_input
        || inv.open
        || john.playback.paused
        || preview.screenshot.is_some()
        || [KeyCode::Home, KeyCode::Escape, KeyCode::Space]
            .iter()
            .any(|k| keys.just_pressed(*k));
    if blocked {
        selection.gesture = None;
        return;
    }
    if selection.gesture.is_some() && mouse.just_pressed(MouseButton::Right) {
        selection.gesture = None;
        mouse.reset(MouseButton::Right);
        mouse.clear_just_pressed(MouseButton::Left);
        return;
    }
    let Some(cursor) = cursor.filter(|c| {
        c.is_finite() && c.x >= 0.0 && c.y >= 0.0 && c.x <= window.width() && c.y <= window.height()
    }) else {
        selection.gesture = None;
        return;
    };
    if selection.gesture.is_none() && mouse.just_pressed(MouseButton::Left) {
        if buttons
            .iter()
            .any(|s| *s == crate::ui_controls::ButtonVisualState::Pressed)
        {
            return;
        }
        if selection.world_blocked {
            mouse.clear_just_pressed(MouseButton::Left);
            return;
        }
        selection.gesture = Some(Gesture {
            start: cursor,
            end: cursor,
            additive: keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight),
            original: u8::from(john.selected) | (u8::from(party.jack.selected) << 1),
            dragged: false,
            synthetic,
        });
    }
    if let Some(gesture) = &mut selection.gesture {
        mouse.clear_just_pressed(MouseButton::Left);
        gesture.update(cursor);
        if !mouse.pressed(MouseButton::Left) {
            let gesture = selection.gesture.take().unwrap();
            if gesture.dragged {
                let (camera, tr) = *camera;
                let inside = |actor: &player::Player| {
                    camera
                        .world_to_viewport(tr, actor.camera_target())
                        .is_ok_and(|p| {
                            p.x >= 0.0
                                && p.x <= window.width()
                                && p.y >= 0.0
                                && p.y <= window.height()
                                && !ui(p)
                                && gesture.rect().contains(p)
                        })
                };
                let john_inside = !sources.1.as_ref().is_some_and(|v| v.occupied) && inside(&john);
                let jack_inside = party.recruited && inside(&party.jack);
                let mask = gesture.result(u8::from(john_inside) | (u8::from(jack_inside) << 1));
                party.box_select(mask, &mut john);
            } else if !selection.world_blocked {
                selection.click_cursor = Some(gesture.start);
                // Однокадровый импульс: не оставляем искусственно удержанную кнопку.
                mouse.press(MouseButton::Left);
                mouse.release(MouseButton::Left);
            }
        }
    }
}
pub fn visuals(selection: Option<Res<Selection>>, mut overlay: Query<&mut Node, With<Overlay>>) {
    for mut node in &mut overlay {
        if let Some(g) = selection
            .as_ref()
            .and_then(|s| s.gesture)
            .filter(|g| g.dragged)
        {
            let rect = g.rect();
            node.display = Display::Flex;
            node.left = px(rect.min.x);
            node.top = px(rect.min.y);
            node.width = px(rect.width());
            node.height = px(rect.height());
        } else {
            node.display = Display::None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gesture() -> Gesture {
        Gesture {
            start: Vec2::new(400.0, 200.0),
            end: Vec2::new(400.0, 200.0),
            additive: false,
            original: 1,
            dragged: false,
            synthetic: false,
        }
    }
    #[test]
    fn threshold_is_logical_and_drag_is_sticky() {
        let mut g = gesture();
        g.update(g.start + Vec2::X * 5.99);
        assert!(!g.dragged);
        g.update(g.start + Vec2::Y * 6.0);
        assert!(g.dragged);
        g.update(g.start);
        assert!(g.dragged);
    }
    #[test]
    fn all_four_directions_normalize_to_same_rectangle() {
        for (start, end) in [
            (Vec2::new(400.0, 200.0), Vec2::new(600.0, 400.0)),
            (Vec2::new(600.0, 400.0), Vec2::new(400.0, 200.0)),
            (Vec2::new(400.0, 400.0), Vec2::new(600.0, 200.0)),
            (Vec2::new(600.0, 200.0), Vec2::new(400.0, 400.0)),
        ] {
            let mut g = gesture();
            g.start = start;
            g.update(end);
            assert_eq!(g.rect(), Rect::new(400.0, 200.0, 600.0, 400.0));
        }
    }
    #[test]
    fn replace_empty_and_ctrl_union() {
        let mut g = gesture();
        assert_eq!(g.result(0), 0);
        assert_eq!(g.result(2), 2);
        g.additive = true;
        assert_eq!(g.result(0), 1);
        assert_eq!(g.result(2), 3);
    }
    #[test]
    fn selecting_masks_does_not_issue_orders_or_recruit() {
        let (mut party, mut john, _, _) = crate::party::tests::fixture();
        let john_position = john.position;
        let jack_position = party.jack.position;
        party.box_select(2, &mut john);
        assert!(!john.selected);
        assert!(!party.jack.selected);
        assert!(!party.recruited);
        party.recruited = true;
        party.box_select(3, &mut john);
        assert!(john.selected && party.jack.selected);
        party.box_select(0, &mut john);
        assert!(!john.selected && !party.jack.selected);
        assert_eq!(john.position, john_position);
        assert_eq!(party.jack.position, jack_position);
        assert!(john.destination.is_none() && party.jack.destination.is_none());
    }
    #[test]
    fn deselection_clears_follow_camera() {
        let (mut party, mut john, _, _) = crate::party::tests::fixture();
        party.recruited = true;
        john.follow_camera = true;
        party.jack.follow_camera = true;
        party.box_select(0, &mut john);
        assert!(!john.follow_camera && !party.jack.follow_camera);
    }
    #[derive(Resource, Default)]
    struct Clicks(Vec<Vec2>);
    fn observe(
        mouse: Res<ButtonInput<MouseButton>>,
        selection: Res<Selection>,
        mut clicks: ResMut<Clicks>,
    ) {
        if mouse.just_pressed(MouseButton::Left) {
            clicks.0.push(selection.click_cursor.unwrap_or(Vec2::ZERO));
        }
    }
    fn input_app() -> App {
        let (party, john, t, grid) = crate::party::tests::fixture();
        let mut app = App::new();
        app.insert_resource(party)
            .insert_resource(john)
            .insert_resource(t)
            .insert_resource(crate::inventory::tests::inventory())
            .insert_resource(training::Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 0.0,
                grid: Some(grid),
                playable: true,
            })
            .insert_resource(Selection {
                test_cursor: Some(Vec2::new(400.0, 300.0)),
                ..default()
            })
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Clicks>()
            .add_systems(Update, (input, observe).chain());
        app.world_mut().spawn(Window {
            focused: true,
            ..default()
        });
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::default(),
            training::MapCamera,
        ));
        app
    }
    #[test]
    fn click_waits_for_release_and_replays_original_coordinates_once() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<Clicks>().0.is_empty());
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert_eq!(
            app.world().resource::<Clicks>().0,
            vec![Vec2::new(400.0, 300.0)]
        );
        assert!(
            !app.world()
                .resource::<ButtonInput<MouseButton>>()
                .pressed(MouseButton::Left)
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert_eq!(app.world().resource::<Clicks>().0.len(), 1);
    }
    #[test]
    fn left_part_of_world_is_available_without_the_old_debug_sidebar() {
        let mut app = input_app();
        app.world_mut().resource_mut::<Selection>().test_cursor = Some(Vec2::new(100.0, 200.0));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<Selection>().gesture.is_some());
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert_eq!(
            app.world().resource::<Clicks>().0,
            vec![Vec2::new(100.0, 200.0)]
        );
    }
    #[test]
    fn first_drag_after_inventory_closes_cannot_leak_a_world_click() {
        let mut app = input_app();
        app.world_mut().resource_mut::<Inventory>().consumed_input = true;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<Selection>().gesture.is_some());
        app.world_mut().resource_mut::<Selection>().test_cursor = Some(Vec2::new(440.0, 340.0));
        app.update();
        assert!(app.world().resource::<Selection>().gesture.unwrap().dragged);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<Clicks>().0.is_empty());
    }
    #[test]
    fn inventory_cancels_hold_without_a_click_after_closing() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        app.world_mut().resource_mut::<Inventory>().open = true;
        app.update();
        assert!(app.world().resource::<Selection>().gesture.is_none());
        app.world_mut().resource_mut::<Inventory>().open = false;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<Clicks>().0.is_empty());
    }
}

// Нативная проверка использует обычные системы ввода и вычисленную раскладку UI.
#[derive(Clone, Copy, Debug)]
enum Check {
    Mask(u8),
    Recruit,
    NoOrders,
    Cancelled,
    Inventory(bool),
    Center(u8),
    Hud,
    Posture(crate::stance::Posture),
    Follow(bool),
    Zoom(f32),
    Diagnostics(bool),
    End,
}
#[derive(Clone, Copy, Debug)]
enum Step {
    Pointer(Vec2, u8),
    Ctrl(bool),
    Key(KeyCode),
    Portrait(u8, u8),
    Header(u8),
    Command(&'static str),
    Wait(u32),
    Check(Check),
    Capture,
}
#[derive(Resource)]
pub struct NativeSmoke {
    path: std::path::PathBuf,
    screenshot: Option<std::path::PathBuf>,
    steps: std::collections::VecDeque<Step>,
    ready: bool,
    frames: u32,
    ctrl: bool,
    initial: Vec3,
    checks: Vec<serde_json::Value>,
}
impl NativeSmoke {
    pub fn new(path: std::path::PathBuf, screenshot: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            screenshot,
            steps: default(),
            ready: false,
            frames: 0,
            ctrl: false,
            initial: Vec3::ZERO,
            checks: Vec::new(),
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn native_smoke(
    mut commands: Commands,
    s: Option<ResMut<NativeSmoke>>,
    selection: Option<ResMut<Selection>>,
    john: Option<ResMut<player::Player>>,
    party: Option<ResMut<Party>>,
    t: Option<Res<Tutorial>>,
    context: (
        Option<Res<Inventory>>,
        Option<ResMut<crate::vehicle::Vehicle>>,
        Option<Res<crate::roster::Roster>>,
        Option<Res<crate::hud::Hud>>,
    ),
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    view: (
        Single<&Window>,
        Single<(&Camera, &GlobalTransform), With<training::MapCamera>>,
        Res<training::Preview>,
    ),
    mut orbit: ResMut<training::Orbit>,
    mut buttons: Query<(
        &crate::party::Select,
        &mut crate::ui_controls::ButtonVisualState,
    )>,
    blocks: Query<(&ComputedNode, &UiGlobalTransform, Has<crate::roster::Panel>), With<UiBlock>>,
    mut commands_ui: Query<
        (
            &crate::hud::CommandButton,
            &mut crate::ui_controls::ButtonVisualState,
        ),
        Without<crate::party::Select>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    let (
        Some(mut s),
        Some(mut sel),
        Some(mut john),
        Some(mut party),
        Some(t),
        (Some(inv), Some(mut vehicle), Some(roster), Some(hud)),
    ) = (s, selection, john, party, t, context)
    else {
        return;
    };
    s.frames += 1;
    assert!(s.frames < 3600, "selection smoke timeout");
    keys.reset_all();
    mouse.reset_all();
    sel.test_cursor = None;
    for (_, mut state) in &mut buttons {
        *state = crate::ui_controls::ButtonVisualState::None;
    }
    for (_, mut state) in &mut commands_ui {
        *state = crate::ui_controls::ButtonVisualState::None;
    }
    if s.frames < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    if t.consumed_input {
        return;
    }
    // Первое окно появляется через задержку сценария. Ждём его до рамок,
    // иначе его правильная отмена жеста делает тест зависимым от FPS.
    if !s.ready
        && !t
            .shown
            .iter()
            .any(|id| id.eq_ignore_ascii_case("help_combat_1"))
    {
        return;
    }

    let (window, camera, preview) = view;
    let (camera, tr) = *camera;
    assert_eq!(roster.image_handles().count(), 2);
    if !s.ready {
        john.playback.paused = false;
        vehicle.occupied = false;
        party.recruited = false;
        john.selected = true;
        party.jack.selected = false;
        party.jack.position = john.position + Vec3::X * 3.0;
        party.jack.position.y =
            player::surface(preview.grid.as_ref().unwrap(), party.jack.position).unwrap();
        s.initial = john.position;
        orbit.target = (john.camera_target() + party.jack.camera_target()) * 0.5;
        s.ready = true;
        return;
    }
    if s.steps.is_empty() && s.checks.is_empty() {
        let j = camera.world_to_viewport(tr, john.camera_target()).unwrap();
        let k = camera
            .world_to_viewport(tr, party.jack.camera_target())
            .unwrap();
        assert!(
            j.distance(k) > 20.0,
            "fixture needs separated projected actors"
        );
        let min = j.min(k) - Vec2::splat(12.0);
        let max = j.max(k) + Vec2::splat(12.0);
        assert!(min.x > 315.0 && max.x < window.width());
        let mut steps = Vec::new();
        let mut drag = |a: Vec2, b: Vec2, mask: u8| {
            steps.extend([
                Step::Pointer(a, 1),
                Step::Pointer(b, 2),
                Step::Pointer(b, 0),
                Step::Check(Check::Mask(mask)),
                Step::Check(Check::NoOrders),
            ]);
        };
        drag(min, max, 1);
        steps.push(Step::Check(Check::Recruit));
        for (a, b) in [
            (min, max),
            (max, min),
            (Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)),
            (Vec2::new(max.x, min.y), Vec2::new(min.x, max.y)),
        ] {
            steps.extend([
                Step::Pointer(a, 1),
                Step::Pointer(b, 2),
                Step::Pointer(b, 0),
                Step::Check(Check::Mask(3)),
                Step::Check(Check::NoOrders),
            ]);
        }
        let ja = j - Vec2::splat(8.0);
        let jb = j + Vec2::splat(8.0);
        let ka = k - Vec2::splat(8.0);
        let kb = k + Vec2::splat(8.0);
        let ea = Vec2::new(340.0, 90.0);
        let eb = ea + Vec2::splat(25.0);
        steps.extend([
            Step::Pointer(ja, 1),
            Step::Pointer(jb, 2),
            Step::Pointer(jb, 0),
            Step::Check(Check::Mask(1)),
            Step::Ctrl(true),
            Step::Pointer(ka, 1),
            Step::Pointer(kb, 2),
            Step::Pointer(kb, 0),
            Step::Check(Check::Mask(3)),
            Step::Ctrl(false),
            Step::Pointer(ea, 1),
            Step::Pointer(eb, 2),
            Step::Pointer(eb, 0),
            Step::Check(Check::Mask(0)),
            Step::Ctrl(true),
            Step::Pointer(ja, 1),
            Step::Pointer(jb, 2),
            Step::Pointer(jb, 0),
            Step::Check(Check::Mask(1)),
            Step::Ctrl(false),
        ]);
        steps.extend([
            Step::Pointer(min, 1),
            Step::Pointer(max, 2),
            Step::Pointer(max, 3),
            Step::Check(Check::Cancelled),
            Step::Check(Check::Mask(1)),
            Step::Check(Check::NoOrders),
        ]);
        steps.extend([
            Step::Header(1),
            Step::Header(0),
            Step::Check(Check::Cancelled),
            Step::Check(Check::NoOrders),
        ]);
        steps.extend([
            Step::Pointer(min, 1),
            Step::Pointer(max, 2),
            Step::Key(KeyCode::Space),
            Step::Check(Check::Cancelled),
            Step::Check(Check::Inventory(true)),
            Step::Key(KeyCode::Space),
            Step::Check(Check::Inventory(false)),
        ]);
        steps.extend([
            Step::Portrait(2, 1),
            Step::Portrait(2, 1),
            Step::Check(Check::Mask(2)),
            Step::Check(Check::Center(2)),
            Step::Portrait(1, 1),
            Step::Portrait(2, 3),
            Step::Check(Check::Mask(2)),
            Step::Check(Check::Inventory(false)),
            Step::Portrait(1, 3),
            Step::Check(Check::Inventory(true)),
            Step::Key(KeyCode::Space),
            Step::Check(Check::Inventory(false)),
            Step::Portrait(1, 1),
            Step::Check(Check::Mask(1)),
        ]);
        steps.extend([
            Step::Check(Check::Hud),
            Step::Command("b_crouch"),
            Step::Wait(600),
            Step::Check(Check::Posture(crate::stance::Posture::Crouch)),
            Step::Command("b_stand"),
            Step::Wait(600),
            Step::Check(Check::Posture(crate::stance::Posture::Stand)),
            Step::Command("b_track"),
            Step::Check(Check::Follow(true)),
            Step::Command("b_track"),
            Step::Check(Check::Follow(false)),
            Step::Command("b_inventory"),
            Step::Check(Check::Inventory(true)),
            Step::Key(KeyCode::Space),
            Step::Check(Check::Inventory(false)),
            Step::Command("b_radar_zoom_in"),
            Step::Check(Check::Zoom(4.0)),
            Step::Command("b_radar_zoom_out"),
            Step::Check(Check::Zoom(2.0)),
            Step::Key(KeyCode::F3),
            Step::Check(Check::Diagnostics(true)),
            Step::Key(KeyCode::F3),
            Step::Check(Check::Diagnostics(false)),
        ]);
        // Портреты и рамка видны одновременно на итоговом снимке.
        steps.extend([
            Step::Pointer(ea, 1),
            Step::Pointer(eb, 2),
            Step::Capture,
            Step::Pointer(eb, 3),
            Step::Key(KeyCode::Home),
            Step::Check(Check::Cancelled),
            Step::Check(Check::End),
        ]);
        s.steps = steps.into();
    }
    if s.ctrl {
        keys.press(KeyCode::ControlLeft);
    }
    let Some(step) = s.steps.pop_front() else {
        return;
    };
    eprintln!(
        "selection smoke: frame={} step={step:?} mask={} recruited={} gesture={:?} camera={:?}",
        s.frames,
        u8::from(john.selected) | (u8::from(party.jack.selected) << 1),
        party.recruited,
        sel.gesture,
        orbit.target
    );
    match step {
        Step::Pointer(p, mode) => {
            sel.test_cursor = Some(p);
            match mode {
                1 | 2 => mouse.press(MouseButton::Left),
                3 => {
                    mouse.press(MouseButton::Left);
                    mouse.press(MouseButton::Right);
                }
                _ => {}
            }
        }
        Step::Header(mode) => {
            let (node, tr, _) = blocks
                .iter()
                .find(|(_, _, panel)| *panel)
                .expect("roster panel");
            let point = tr.affine().translation / window.scale_factor()
                + Vec2::new(0.0, -node.size().y / window.scale_factor() * 0.5 + 4.0);
            sel.test_cursor = Some(point);
            if mode == 1 {
                mouse.press(MouseButton::Left);
            }
        }
        Step::Ctrl(ctrl) => {
            s.ctrl = ctrl;
        }
        Step::Key(key) => {
            keys.press(key);
        }
        Step::Portrait(member, mode) => {
            for (b, mut state) in &mut buttons {
                if b.0 == member {
                    *state = if mode == 3 {
                        crate::ui_controls::ButtonVisualState::Hovered
                    } else {
                        crate::ui_controls::ButtonVisualState::Pressed
                    };
                }
            }
            if mode == 3 {
                mouse.press(MouseButton::Right);
            } else {
                mouse.press(MouseButton::Left);
            }
        }
        Step::Command(id) => {
            let (_, mut state) = commands_ui
                .iter_mut()
                .find(|(b, _)| b.0 == id)
                .expect("original command button");
            *state = crate::ui_controls::ButtonVisualState::Pressed;
            mouse.press(MouseButton::Left);
        }
        Step::Wait(n) => {
            if john.changing_posture() {
                assert!(n > 0, "posture transition timeout");
                s.steps.push_front(Step::Wait(n - 1));
            }
        }
        Step::Capture => {
            // Сохраняем удержание до захвата; рамка не превращается в приказ.
            if let Some(g) = sel.gesture {
                sel.test_cursor = Some(g.end);
                mouse.press(MouseButton::Left);
            }
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            }
        }
        Step::Check(check) => {
            match check {
                Check::Mask(expected) => assert_eq!(
                    u8::from(john.selected) | (u8::from(party.jack.selected) << 1),
                    expected,
                    "checks={:?}, John={:?}, Jack={:?}",
                    s.checks,
                    camera.world_to_viewport(tr, john.camera_target()),
                    camera.world_to_viewport(tr, party.jack.camera_target())
                ),
                Check::Recruit => {
                    assert!(!party.recruited && !party.jack.selected);
                    party.recruited = true;
                }
                Check::NoOrders => {
                    assert!(john.destination.is_none() && party.jack.destination.is_none());
                    assert!(john.position.distance(s.initial) < 0.001);
                }
                Check::Cancelled => assert!(sel.gesture.is_none() && sel.click_cursor.is_none()),
                Check::Inventory(open) => assert_eq!(inv.open, open),
                Check::Center(member) => {
                    let p = if member == 1 {
                        john.camera_target()
                    } else {
                        party.jack.camera_target()
                    };
                    assert!(orbit.target.distance(p) < 0.001);
                }
                Check::Hud => {
                    assert_eq!(hud.image_handles().count(), 3);
                    assert_eq!(hud.layout.regions.len(), 3);
                    assert_eq!(hud.layout.advice.len(), 18);
                    assert!(!hud.diagnostics);
                }
                Check::Posture(p) => {
                    assert_eq!(john.posture, p);
                    assert!(!john.changing_posture());
                }
                Check::Follow(f) => {
                    assert_eq!(john.follow_camera, f);
                }
                Check::Zoom(z) => {
                    assert_eq!(hud.report()["zoom"], serde_json::json!(z));
                }
                Check::Diagnostics(d) => {
                    assert_eq!(hud.diagnostics, d);
                }
                Check::End => {
                    assert!(!party.recruited && john.selected && !party.jack.selected);
                    let report = serde_json::json!({"status":"passed","window":[window.width(),window.height()],"checks":s.checks,"portraits":roster.report(),"hud":hud.report(),"physical_pointer_tested":false,"input":"Bevy ButtonInput and UI button state; real layout and camera projection","limits":"test fixture relocates Jack beside John; no full tutorial route"});
                    std::fs::write(
                        &s.path,
                        serde_json::to_string_pretty(&report).unwrap() + "\n",
                    )
                    .unwrap();
                    exit.write(AppExit::Success);
                }
            }
            s.checks.push(serde_json::json!({"check":match check{Check::Mask(_)=>"selection_mask",Check::Recruit=>"unrecruited_excluded",Check::NoOrders=>"drag_never_orders_or_moves",Check::Cancelled=>"gesture_cancelled",Check::Inventory(_)=>"portrait_inventory_isolated",Check::Center(_)=>"portrait_double_click_centers",Check::Hud=>"original_hud_assets_and_regions",Check::Posture(_)=>"original_stance_buttons",Check::Follow(_)=>"original_tracking_button",Check::Zoom(_)=>"radar_zoom_buttons",Check::Diagnostics(_)=>"debug_hidden_and_f3_toggle",Check::End=>"home_resets"},"passed":true}));
        }
    }
}
