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

pub fn sync(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    mut buttons: Query<(Entity, &Hovered, Has<Pressed>, &mut ButtonVisualState), With<UiButton>>,
) {
    let held = mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right);
    for (entity, hovered, pressed, mut state) in &mut buttons {
        // Opening a modal hides the pressed HUD button before the release can
        // reach it. A leftover widget marker must not activate on another click.
        if pressed && !held {
            commands.entity(entity).remove::<Pressed>();
        }
        state.set_if_neq(if pressed && held && hovered.get() {
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
            .init_resource::<ButtonInput<MouseButton>>()
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
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
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
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
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
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Changes>()
            .add_systems(Update, (sync, count).chain());
        app.world_mut().spawn(UiButton);
        app.update();
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Changes>().0, 1);
    }

    #[test]
    fn hidden_pressed_button_does_not_capture_later_clicks_after_modal_closes() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, sync);
        let inventory = app
            .world_mut()
            .spawn((UiButton, Hovered(true), Pressed))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(
            *app.world().get::<ButtonVisualState>(inventory).unwrap(),
            ButtonVisualState::Pressed
        );
        // The inventory modal hides the HUD. PointerRelease goes to the modal,
        // so deliberately do not send a widget release to the inventory button.
        app.world_mut().entity_mut(inventory).insert(Hovered(false));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert!(!app.world().entity(inventory).contains::<Pressed>());
        let other = app
            .world_mut()
            .spawn((UiButton, Hovered(true), Pressed))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(
            *app.world().get::<ButtonVisualState>(inventory).unwrap(),
            ButtonVisualState::None
        );
        assert_eq!(
            *app.world().get::<ButtonVisualState>(other).unwrap(),
            ButtonVisualState::Pressed
        );
    }
}
