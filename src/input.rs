//! Injects input into the World as if it came from the OS.
//!
//! The cursor is virtual: the OS cursor must never move. bevy_winit's
//! `changed_windows` (in `Last`) warps the OS cursor whenever `Window`'s cursor
//! position differs from its cache and *anything* changed the window that frame.
//! So the injected position is written with `bypass_change_detection()` and
//! the real position is restored by `restore_real_cursor` before `Last`.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardFocusLost, KeyboardInput, NativeKey};
use bevy::ecs::message::{Message, Messages};
use bevy::input::mouse::{MouseButtonInput, MouseMotion, MouseWheel};
use bevy::math::DVec2;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

use crate::command::Button;

pub fn primary_window(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .iter(world)
        .next()
}

/// Sets the cursor position (logical px, top-left origin) and emits `CursorMoved`.
pub fn set_cursor(world: &mut World, pos: Vec2) {
    let Some(window) = primary_window(world) else { return };
    let last = write_cursor(world, window, Some(pos));
    let ev = CursorMoved { window, position: pos, delta: last.map(|l| pos - l) };
    world.write_message(WindowEvent::CursorMoved(ev.clone()));
    world.write_message(ev);
}

/// Re-asserts the virtual cursor without emitting events (undoes real-mouse hover).
/// `None` hides the real cursor from the game (batch mode before the first injection).
pub fn hold_cursor(world: &mut World, pos: Option<Vec2>) {
    if let Some(window) = primary_window(world) {
        write_cursor(world, window, pos);
    }
}

/// Batch mode: drop the OS's real mouse/keyboard messages (and their `WindowEvent` mirrors) so only virtual input
/// reaches the game, wherever the user's real mouse is. Must run after
/// `MessageUpdateSystems` and before injecting.
pub fn drop_real_input(world: &mut World) {
    fn clear<M: Message>(world: &mut World) {
        if let Some(mut m) = world.get_resource_mut::<Messages<M>>() {
            m.clear();
        }
    }
    clear::<MouseButtonInput>(world);
    clear::<MouseMotion>(world);
    clear::<MouseWheel>(world);
    clear::<KeyboardInput>(world);
    clear::<CursorMoved>(world);
    // focus loss would release_all() held virtual keys
    clear::<KeyboardFocusLost>(world);
    // bevy_picking reads WindowEvent, not the per-type messages above
    if let Some(mut m) = world.get_resource_mut::<Messages<WindowEvent>>() {
        let keep: Vec<_> = m.drain().filter(|e| !is_input(e)).collect();
        m.write_batch(keep);
    }
}

fn is_input(e: &WindowEvent) -> bool {
    matches!(
        e,
        WindowEvent::CursorMoved(_)
            | WindowEvent::CursorEntered(_)
            | WindowEvent::CursorLeft(_)
            | WindowEvent::MouseButtonInput(_)
            | WindowEvent::MouseMotion(_)
            | WindowEvent::MouseWheel(_)
            | WindowEvent::KeyboardInput(_)
            | WindowEvent::KeyboardFocusLost(_)
    )
}

/// The OS-reported cursor, saved while a virtual one is in place.
#[derive(Resource)]
pub struct RealCursor(pub Option<DVec2>);

fn write_cursor(world: &mut World, window: Entity, pos: Option<Vec2>) -> Option<Vec2> {
    let mut q = world.query::<&mut Window>();
    let Ok(mut w) = q.get_mut(world, window) else { return None };
    let last = w.cursor_position();
    let real = w.physical_cursor_position().map(|v| v.as_dvec2());
    let scale = w.resolution.scale_factor() as f64;
    w.bypass_change_detection()
        .set_physical_cursor_position(pos.map(|p| p.as_dvec2() * scale));
    // keep the first saved value: later writes in the same frame see our own virtual cursor
    if !world.contains_resource::<RealCursor>() {
        world.insert_resource(RealCursor(real));
    }
    last
}

/// Puts the OS-reported cursor back before bevy_winit syncs the window (runs in `PostUpdate`, after game logic).
pub fn restore_real_cursor(world: &mut World) {
    let Some(RealCursor(real)) = world.remove_resource::<RealCursor>() else { return };
    let Some(window) = primary_window(world) else { return };
    if let Ok(mut w) = world.query::<&mut Window>().get_mut(world, window) {
        w.bypass_change_detection().set_physical_cursor_position(real);
    }
}

pub fn button(world: &mut World, b: Button, pressed: bool) {
    let Some(window) = primary_window(world) else { return };
    let ev = MouseButtonInput { button: b.into(), state: state(pressed), window };
    world.write_message(WindowEvent::MouseButtonInput(ev));
    world.write_message(ev);
}

pub fn key(world: &mut World, key_code: KeyCode, pressed: bool) {
    let Some(window) = primary_window(world) else { return };
    // ponytail: logical_key/text are not synthesized; games reading ButtonInput<KeyCode> work, text input does not
    let ev = KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: state(pressed),
        text: None,
        repeat: false,
        window,
    };
    world.write_message(WindowEvent::KeyboardInput(ev.clone()));
    world.write_message(ev);
}

fn state(pressed: bool) -> ButtonState {
    if pressed { ButtonState::Pressed } else { ButtonState::Released }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::message::Messages;

    fn world_with_window(real: Option<Vec2>) -> World {
        let mut world = World::new();
        world.init_resource::<Messages<CursorMoved>>();
        world.init_resource::<Messages<MouseButtonInput>>();
        world.init_resource::<Messages<KeyboardInput>>();
        world.init_resource::<Messages<WindowEvent>>();
        world.init_resource::<Messages<KeyboardFocusLost>>();
        let mut w = Window::default();
        w.set_cursor_position(real);
        world.spawn((w, PrimaryWindow));
        world
    }

    fn cursor(world: &mut World) -> Option<Vec2> {
        world.query::<&Window>().single(world).unwrap().cursor_position()
    }

    #[test]
    fn virtual_cursor_is_seen_then_real_one_restored() {
        let mut world = world_with_window(Some(Vec2::new(5.0, 5.0)));
        set_cursor(&mut world, Vec2::new(100.0, 50.0));
        assert_eq!(cursor(&mut world), Some(Vec2::new(100.0, 50.0)));
        set_cursor(&mut world, Vec2::new(120.0, 60.0));
        restore_real_cursor(&mut world);
        assert_eq!(cursor(&mut world), Some(Vec2::new(5.0, 5.0)));
    }

    #[test]
    fn hold_none_hides_real_cursor_until_restore() {
        let mut world = world_with_window(Some(Vec2::new(5.0, 5.0)));
        hold_cursor(&mut world, None);
        assert_eq!(cursor(&mut world), None);
        restore_real_cursor(&mut world);
        assert_eq!(cursor(&mut world), Some(Vec2::new(5.0, 5.0)));
    }

    #[test]
    fn drop_real_input_clears_os_messages() {
        let mut world = world_with_window(None);
        let window = primary_window(&mut world).unwrap();
        world.write_message(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window });
        drop_real_input(&mut world);
        assert!(world.resource::<Messages<MouseButtonInput>>().is_empty());
    }

    #[test]
    fn injected_input_is_mirrored_as_window_events_for_picking() {
        let mut world = world_with_window(None);
        set_cursor(&mut world, Vec2::new(1.0, 2.0));
        button(&mut world, Button::Left, true);
        key(&mut world, KeyCode::Space, true);
        assert_eq!(world.resource::<Messages<WindowEvent>>().len(), 3);
    }

    #[test]
    fn drop_real_input_filters_input_window_events_but_keeps_others() {
        let mut world = world_with_window(None);
        let window = primary_window(&mut world).unwrap();
        world.write_message(WindowEvent::MouseButtonInput(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
            window,
        }));
        world.write_message(WindowEvent::RequestRedraw(bevy::window::RequestRedraw));
        drop_real_input(&mut world);
        let left: Vec<_> = world.resource_mut::<Messages<WindowEvent>>().drain().collect();
        assert_eq!(left.len(), 1);
        assert!(matches!(left[0], WindowEvent::RequestRedraw(_)));
    }

    #[test]
    fn drop_real_input_clears_focus_lost() {
        let mut world = world_with_window(None);
        world.write_message(KeyboardFocusLost);
        drop_real_input(&mut world);
        assert!(world.resource::<Messages<KeyboardFocusLost>>().is_empty());
    }
}
