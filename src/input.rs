//! Injects input into the World as if it came from the OS.
//!
//! The cursor is virtual: the OS cursor must never move. bevy_winit's
//! `changed_windows` (in `Last`) warps the OS cursor whenever `Window`'s cursor
//! position differs from its cache and *anything* changed the window that frame.
//! So the injected position is written with `bypass_change_detection()` and
//! the real position is restored by `restore_real_cursor` before `Last`.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput, NativeKey};
use bevy::input::mouse::MouseButtonInput;
use bevy::math::DVec2;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};

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
    let last = write_cursor(world, window, pos);
    world.write_message(CursorMoved {
        window,
        position: pos,
        delta: last.map(|l| pos - l),
    });
}

/// Re-asserts the cursor position without emitting events (undoes real-mouse hover).
pub fn hold_cursor(world: &mut World, pos: Vec2) {
    if let Some(window) = primary_window(world) {
        write_cursor(world, window, pos);
    }
}

/// The OS-reported cursor, saved while a virtual one is in place.
#[derive(Resource)]
pub struct RealCursor(pub Option<DVec2>);

fn write_cursor(world: &mut World, window: Entity, pos: Vec2) -> Option<Vec2> {
    let mut q = world.query::<&mut Window>();
    let Ok(mut w) = q.get_mut(world, window) else { return None };
    let last = w.cursor_position();
    let real = w.physical_cursor_position().map(|v| v.as_dvec2());
    let scale = w.resolution.scale_factor() as f64;
    w.bypass_change_detection()
        .set_physical_cursor_position(Some(pos.as_dvec2() * scale));
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
    world.write_message(MouseButtonInput {
        button: b.into(),
        state: state(pressed),
        window,
    });
}

pub fn key(world: &mut World, key_code: KeyCode, pressed: bool) {
    let Some(window) = primary_window(world) else { return };
    // ponytail: logical_key/text are not synthesized; games reading ButtonInput<KeyCode> work, text input does not
    world.write_message(KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: state(pressed),
        text: None,
        repeat: false,
        window,
    });
}

fn state(pressed: bool) -> ButtonState {
    if pressed { ButtonState::Pressed } else { ButtonState::Released }
}
