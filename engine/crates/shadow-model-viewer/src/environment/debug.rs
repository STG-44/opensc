//! In-session weather controls. Preferences are saved by the options menu.
use super::*;
use bevy::picking::Pickable;

#[derive(Resource)]
pub(super) struct Controls {
    original: Options,
    visible: bool,
    cycle_minutes: f32,
}
impl Controls {
    pub(super) fn new(options: &Options) -> Self {
        Self {
            original: options.clone(),
            visible: false,
            cycle_minutes: if options.day_minutes > 0. {
                options.day_minutes
            } else {
                120.
            },
        }
    }
    fn input(&mut self, keys: &ButtonInput<KeyCode>, options: &mut Options, weather: &mut Weather) {
        let backwards = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if keys.just_pressed(KeyCode::F6) {
            if self.visible && backwards {
                *options = self.original.clone();
                weather.set_debug_preset(options.preset);
            } else {
                self.visible = !self.visible;
            }
        }
        if !self.visible {
            return;
        }
        if keys.just_pressed(KeyCode::F7) {
            let index = Preset::ALL
                .iter()
                .position(|p| *p == options.preset)
                .unwrap();
            let step = if backwards { Preset::ALL.len() - 1 } else { 1 };
            options.preset = Preset::ALL[(index + step) % Preset::ALL.len()];
            weather.set_debug_preset(options.preset);
        }
        if keys.just_pressed(KeyCode::F8) {
            options.hour =
                Some((weather.sample().hour + if backwards { -1. } else { 1. }).rem_euclid(24.));
        }
        if keys.just_pressed(KeyCode::F4) {
            if options.day_minutes > 0. {
                self.cycle_minutes = options.day_minutes;
                options.day_minutes = 0.;
            } else {
                options.day_minutes = self.cycle_minutes;
            }
            // Keep the mission's sunlight curve and elapsed game time.
            weather.clock_options.1 = options.day_minutes;
        }
        // Permit lighting previews while gameplay is paused, and restart the
        // clock from its displayed hour when it is frozen or resumed.
        weather.light(options, 0.);
    }
}

#[derive(Component)]
pub(super) struct Panel;

#[allow(clippy::too_many_arguments)]
pub(super) fn input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut options: ResMut<Options>,
    controls: Option<ResMut<Controls>>,
    weather: Option<ResMut<Weather>>,
    cameras: Query<(Entity, &Camera), With<sky::WeatherCamera>>,
    panel: Query<(), With<Panel>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    if !keys.any_just_pressed([KeyCode::F6, KeyCode::F7, KeyCode::F8, KeyCode::F4]) {
        return;
    }
    let (Some(mut controls), Some(mut weather)) = (controls, weather) else {
        return;
    };
    let Some((camera, _)) = cameras.iter().find(|(_, camera)| camera.is_active) else {
        return;
    };
    controls.input(&keys, &mut options, &mut weather);
    if controls.visible && panel.is_empty() {
        let font = fonts.add(Font::from_bytes(
            include_bytes!("../../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
        ));
        commands.spawn((
            player::SceneEntity,
            Panel,
            UiTargetCamera(camera),
            Pickable::IGNORE,
            Text::default(),
            TextFont {
                font: font.into(),
                font_size: bevy::text::FontSize::Px(15.),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.93, 0.86)),
            BackgroundColor(Color::srgba(0.02, 0.035, 0.025, 0.94)),
            Node {
                position_type: PositionType::Absolute,
                top: px(16),
                right: px(16),
                max_width: percent(65),
                padding: UiRect::all(px(12)),
                ..default()
            },
            GlobalZIndex(20),
        ));
    }
}

pub(super) fn panel(
    controls: Option<Res<Controls>>,
    weather: Option<Res<Weather>>,
    options: Res<Options>,
    mut panels: Query<(&mut Text, &mut Visibility), With<Panel>>,
) {
    let (Some(controls), Some(weather)) = (controls, weather) else {
        return;
    };
    for (mut text, mut visibility) in &mut panels {
        visibility.set_if_neq(if controls.visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
        if !controls.visible {
            continue;
        }
        text.set_if_neq(Text::new(format!(
            "F6 — ПОГОДА\n{} · {} · {}\nF7 — сменить погоду\nF8 — время +1 ч\nShift + F7/F8 — назад\nF4 — остановить / запустить часы\nShift + F6 — исходные настройки",
            options.preset.label(), weather.sample().clock_label(),
            if options.day_minutes == 0. { "пауза".to_string() } else { format!("{:.0} мин.", options.day_minutes) },
        )));
    }
}

pub(super) fn cleanup(world: &mut World) {
    if let Some(controls) = world.remove_resource::<Controls>() {
        world.insert_resource(controls.original);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_wrap_and_freeze_the_displayed_time_without_a_clock_jump() {
        let mut options = Options {
            preset: Preset::Clear,
            hour: Some(23.5),
            day_minutes: 30.,
            ..default()
        };
        let daylight = daylight::Daylight {
            rotation: Quat::IDENTITY,
            color: Color::WHITE,
            source: serde_json::json!({}),
            tables: None,
        };
        let mut weather = Weather::new(&daylight, &options);
        let mut controls = Controls::new(&options);
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::F6);
        keys.press(KeyCode::F8);
        controls.input(&keys, &mut options, &mut weather);
        assert_eq!(weather.sample().hour, 0.5);
        weather.advance(30., &options, Vec3::ZERO);
        let displayed = weather.sample().hour;
        keys.reset_all();
        keys.press(KeyCode::F4);
        controls.input(&keys, &mut options, &mut weather);
        assert_eq!(weather.sample().hour, displayed);
        weather.advance(120., &options, Vec3::ZERO);
        assert_eq!(weather.sample().hour, displayed);
        keys.reset_all();
        keys.press(KeyCode::F4);
        controls.input(&keys, &mut options, &mut weather);
        assert_eq!(weather.sample().hour, displayed);
        assert_eq!(options.day_minutes, 30.);
        keys.reset_all();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::F7);
        controls.input(&keys, &mut options, &mut weather);
        assert_eq!(options.preset, Preset::Mission);
        keys.reset_all();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::F7);
        controls.input(&keys, &mut options, &mut weather);
        assert_eq!(options.preset, Preset::Blizzard);
        assert_eq!(weather.sample().profile.snow, 1.);
        keys.reset_all();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::F6);
        controls.input(&keys, &mut options, &mut weather);
        assert_eq!(options, controls.original);

        options.hour = None;
        weather.advance(30., &options, Vec3::ZERO);
        let mission_sun = weather.sample().sun_to;
        let displayed = weather.sample().hour;
        keys.reset_all();
        keys.press(KeyCode::F4);
        controls.input(&keys, &mut options, &mut weather);
        weather.advance(30., &options, Vec3::ZERO);
        assert_eq!(options.hour, None);
        assert_eq!(weather.sample().hour, displayed);
        assert_eq!(weather.sample().sun_to, mission_sun);
    }
}
