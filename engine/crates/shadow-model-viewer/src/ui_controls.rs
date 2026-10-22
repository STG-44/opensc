//! Состояние кнопок проекта поверх Hovered/Pressed из Bevy 0.20.
use bevy::{picking::hover::Hovered, prelude::*, ui::Pressed};

#[derive(Component, Default)]
#[require(Node, bevy::ui_widgets::Button, Hovered, ButtonVisualState)]
pub struct UiButton;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVisualState {
    #[default]
    None,
    Hovered,
    Pressed,
}

pub fn sync(mut buttons: Query<(&Hovered, Has<Pressed>, &mut ButtonVisualState), With<UiButton>>) {
    for (hovered, pressed, mut state) in &mut buttons {
        state.set_if_neq(if pressed {
            ButtonVisualState::Pressed
        } else if hovered.get() {
            ButtonVisualState::Hovered
        } else {
            ButtonVisualState::None
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        camera::NormalizedRenderTarget,
        picking::{
            backend::HitData,
            events::{Pointer, PointerPress, PointerRelease},
            pointer::{Location, PointerButton, PointerId},
        },
        window::WindowRef,
    };

    #[test]
    fn modern_button_pointer_events_reach_project_state() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::ui_widgets::ButtonPlugin))
            .add_systems(Update, sync);
        let button = app.world_mut().spawn(UiButton).id();
        let window = app.world_mut().spawn_empty().id();
        let pointer = Pointer::new(
            PointerId::Mouse,
            Location {
                target: NormalizedRenderTarget::Window(
                    WindowRef::Entity(window).normalize(None).unwrap(),
                ),
                position: Vec2::new(20.0, 20.0),
            },
        );
        let hit = HitData::new(window, 0.0, None, None);
        app.world_mut().entity_mut(button).insert(Hovered(true));
        app.update();
        assert_eq!(
            *app.world().get::<ButtonVisualState>(button).unwrap(),
            ButtonVisualState::Hovered
        );
        app.world_mut().trigger(PointerPress {
            entity: button,
            pointer: pointer.clone(),
            button: PointerButton::Primary,
            hit: hit.clone(),
            count: 1,
        });
        app.update();
        assert_eq!(
            *app.world().get::<ButtonVisualState>(button).unwrap(),
            ButtonVisualState::Pressed
        );
        app.world_mut().trigger(PointerRelease {
            entity: button,
            pointer,
            button: PointerButton::Primary,
            hit,
        });
        app.update();
        assert_eq!(
            *app.world().get::<ButtonVisualState>(button).unwrap(),
            ButtonVisualState::Hovered
        );
        app.world_mut().entity_mut(button).insert(Hovered(false));
        app.update();
        assert_eq!(
            *app.world().get::<ButtonVisualState>(button).unwrap(),
            ButtonVisualState::None
        );
    }

    #[test]
    fn idle_state_does_not_emit_repeated_changes() {
        #[derive(Resource, Default)]
        struct Changes(usize);
        fn count(buttons: Query<(), Changed<ButtonVisualState>>, mut changes: ResMut<Changes>) {
            changes.0 += buttons.iter().count();
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Changes>()
            .add_systems(Update, (sync, count).chain());
        app.world_mut().spawn(UiButton);
        app.update();
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Changes>().0, 1);
    }
}
