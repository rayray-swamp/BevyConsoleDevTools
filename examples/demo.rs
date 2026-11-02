//! Minimal game for exercising the devtools: drag the square with the mouse.
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowResolution};

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Square {
    pos: Vec2,
}

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            resolution: WindowResolution::new(800, 600),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(bevy_console_devtools::ConsoleDevToolsPlugin)
    .register_type::<Square>()
    .add_systems(Startup, setup)
    .add_systems(Update, drag_square);
    app.run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands.spawn((
        Sprite::from_color(Color::srgb(0.9, 0.3, 0.2), Vec2::splat(80.0)),
        Transform::default(),
        Square::default(),
    ));
}

/// Reads `window.cursor_position()` directly, like most real games.
fn drag_square(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    square: Single<(&mut Transform, &mut Square)>,
    mut grabbing: Local<bool>,
) {
    let (mut tf, mut sq) = square.into_inner();
    if let Some(cursor) = window.cursor_position()
        && let Ok(p) = camera.0.viewport_to_world_2d(camera.1, cursor)
    {
        if buttons.just_pressed(MouseButton::Left)
            && (p - tf.translation.truncate()).abs().max_element() < 40.0
        {
            *grabbing = true;
        }
        if *grabbing {
            tf.translation = p.extend(0.0);
        }
    }
    if buttons.just_released(MouseButton::Left) {
        *grabbing = false;
    }
    sq.pos = tf.translation.truncate();
}
