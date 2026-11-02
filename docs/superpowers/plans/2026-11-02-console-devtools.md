# BevyConsoleDevTools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bevy 0.20 plugin crate that lets an AI drive a game from the shell: scripted/remote input injection, frame capture to PNG, and reflected state dumps.

**Architecture:** A pure `Scheduler` turns `DevCommand`s into per-frame `Action`s (unit-testable, no Bevy app). An exclusive system in `First` executes the actions against the `World` (input messages, cursor, screenshots, dumps, clock). Batch mode feeds the scheduler from a JSONL script; interactive mode feeds the same scheduler from BRP methods `devtools.<cmd>`.

**Tech Stack:** Rust 2024, bevy 0.20.1 (`bevy_remote`, `serialize` features), serde, serde_json.

**Spec:** `docs/superpowers/specs/2026-11-02-console-devtools-design.md`

## Global Constraints

- Bevy `0.20.1`, edition `2024`. Crate name `bevy_console_devtools`, plugin `ConsoleDevToolsPlugin`.
- Env vars: `BCDT_SCRIPT` (script path → batch mode), `BCDT_OUT` (output dir, default `bcdt_out`), `BCDT_MAX_FRAMES` (default `36000`).
- BRP HTTP port: Bevy default `15702`; methods named `devtools.<cmd>`.
- Exit codes: `0` success, `1` runtime error / max frames, `2` script parse error (before any frame).
- Coordinates: window logical pixels, origin top-left.
- Time: fixed `1/60` s per frame (`TimeUpdateStrategy::ManualDuration`).
- Outputs: `frames/<frame:06>.png` or `frames/<name>.png`, `events.jsonl`, `dump/<frame:06>.json`.
- **The OS cursor must never move.** Cursor injection writes `Window` with `bypass_change_detection()`; a normal mutable write makes `bevy_winit::changed_windows` call `winit_window.set_cursor_position` (verified in bevy_winit-0.20.1 `src/system.rs:426-432`).

## Review Focus

1. **Real mouse over the window during batch** — winit `CursorMoved` overwrites the injected cursor. Expected: injected position wins. → Task 4 re-asserts the last injected position every frame (`DevState.cursor`).
2. **`quit` right after `screenshot`/`record`** — capture is async; exiting immediately loses the last PNG. Expected: last frame's PNG exists. → Task 4 delays exit until no `Screenshot` entities remain (cap 120 frames); e2e asserts the final PNG.
3. **Script without `quit`** — Expected: auto-exit 0 when the queue drains. → Task 3 test `auto_quit_when_idle`.
4. **Misspelled type in `dump`** — non-strict BRP query silently returns nothing. Expected: error, exit 1. → Task 4 uses `"strict": true`; test `dump_unknown_component_is_error`.
5. **Malformed script line** — Expected: exit 2 with the 1-based line number, no window opened. → Task 2 parse tests + Task 4 e2e `bad_script_exits_2`.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | crate manifest |
| `src/lib.rs` | `ConsoleDevToolsPlugin`, `DevState` resource, env config |
| `src/command.rs` | `DevCommand` (+serde), `Button`, `KeyAction`, `parse_script`, `validate` |
| `src/scheduler.rs` | pure command → per-frame `Action` scheduler |
| `src/input.rs` | cursor / mouse button / key injection into the `World` |
| `src/capture.rs` | screenshot to disk |
| `src/dump.rs` | reflected state dump via BRP builtin handlers |
| `src/runner.rs` | exclusive `First` system: tick scheduler, apply actions, log, exit |
| `src/remote.rs` | BRP methods `devtools.<cmd>` |
| `examples/demo.rs` | 800×600 window, draggable square (reads `window.cursor_position()` like real games) |
| `scenarios/demo_drag.jsonl` | sample batch script |
| `tests/demo_e2e.rs` | `#[ignore]` end-to-end tests (needs a window; macOS local) |
| `README.md` | usage for AI agents and integrators |

---

### Task 1: Crate scaffold, input injection, and cursor spike (GATE)

Proves the riskiest assumption first: the injected cursor moves the game's view of the cursor **without moving the OS cursor**. If the spike fails, STOP and report — the integration design changes.

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/lib.rs`, `src/command.rs` (only `Button`), `src/input.rs`, `examples/demo.rs`

**Interfaces:**
- Produces: `command::Button { Left, Right, Middle }` (serde lowercase, `Default = Left`, `impl From<Button> for MouseButton`);
  `input::primary_window(&mut World) -> Option<Entity>`;
  `input::set_cursor(&mut World, Vec2)` (moves + writes `CursorMoved`);
  `input::hold_cursor(&mut World, Vec2)` (re-asserts position only, no message);
  `input::button(&mut World, Button, bool)`; `input::key(&mut World, KeyCode, bool)`.

- [ ] **Step 1: Write the manifest and module skeleton**

`Cargo.toml`:
```toml
[package]
name = "bevy_console_devtools"
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
description = "AI-friendly scripted input, frame capture and state dumps for Bevy"

[dependencies]
# ponytail: default bevy features (heavy); trim to the needed feature set if compile time matters
bevy = { version = "0.20.1", features = ["bevy_remote", "serialize"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`.gitignore`:
```
/target
/bcdt_out
```

`src/lib.rs`:
```rust
pub mod command;
pub mod input;
```

`src/command.rs`:
```rust
use bevy::input::mouse::MouseButton;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

impl From<Button> for MouseButton {
    fn from(b: Button) -> Self {
        match b {
            Button::Left => MouseButton::Left,
            Button::Right => MouseButton::Right,
            Button::Middle => MouseButton::Middle,
        }
    }
}
```

- [ ] **Step 2: Write `src/input.rs`**

```rust
//! Injects input into the World as if it came from the OS.
//!
//! The cursor is written with `bypass_change_detection()`: a normal write marks
//! `Window` changed and bevy_winit's `changed_windows` then warps the real OS
//! cursor via `winit_window.set_cursor_position`.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput, NativeKey};
use bevy::input::mouse::MouseButtonInput;
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

fn write_cursor(world: &mut World, window: Entity, pos: Vec2) -> Option<Vec2> {
    let mut q = world.query::<&mut Window>();
    let Ok(mut w) = q.get_mut(world, window) else { return None };
    let last = w.cursor_position();
    let scale = w.resolution.scale_factor() as f64;
    w.bypass_change_detection()
        .set_physical_cursor_position(Some(pos.as_dvec2() * scale));
    last
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
```

- [ ] **Step 3: Write `examples/demo.rs` with a temporary spike system**

```rust
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
    .add_systems(Startup, setup)
    .add_systems(Update, drag_square);

    // SPIKE (removed in Task 4): inject a drag directly via input::*
    if std::env::var("BCDT_SPIKE").is_ok() {
        app.add_systems(First, spike_drag);
    }
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

fn spike_drag(world: &mut World, mut f: Local<u32>) {
    use bevy_console_devtools::{command::Button, input};
    *f += 1;
    match *f {
        30 => { input::set_cursor(world, Vec2::new(400.0, 300.0)); input::button(world, Button::Left, true); }
        31..=90 => input::set_cursor(world, Vec2::new(400.0, 300.0).lerp(Vec2::new(600.0, 150.0), (*f - 30) as f32 / 60.0)),
        91 => input::button(world, Button::Left, false),
        100 => {
            let pos = world.query::<&Square>().single(world).unwrap().pos;
            println!("SPIKE square.pos = {pos:?}");
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}
```

- [ ] **Step 4: Build**

Run: `cargo build --example demo`
Expected: compiles. If an import path differs in 0.20.1, fix it by grepping `~/.cargo/registry/src/*/bevy_*-0.20.1/src` for the type name.

- [ ] **Step 5: Run the spike and check the OS cursor did not move**

```bash
cat > /tmp/cursor.swift <<'EOF'
import CoreGraphics
print(CGEvent(source: nil)!.location)
EOF
swift /tmp/cursor.swift > /tmp/c_before.txt
BCDT_SPIKE=1 cargo run --example demo 2>&1 | grep SPIKE
swift /tmp/cursor.swift > /tmp/c_after.txt
diff /tmp/c_before.txt /tmp/c_after.txt && echo "OS cursor unchanged"
```
Do not touch the mouse while it runs.
Expected: `SPIKE square.pos = Vec2(200.0, 150.0)` (±1; window (600,150) → world (200,150)) AND `OS cursor unchanged`.
**GATE:** if the square did not move or the OS cursor moved, STOP and report the outcome; do not continue to Task 2.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml .gitignore src examples
git commit -m "feat: crate scaffold and input injection without moving the OS cursor"
```

---

### Task 2: Commands and script parsing

**Files:**
- Modify: `src/command.rs`

**Interfaces:**
- Consumes: `Button` (Task 1).
- Produces: `KeyAction { Press, Release, Tap }`; `DevCommand` (serde `tag = "cmd"`, lowercase); `DevCommand::validate(&self) -> Result<(), String>`; `ScriptError { line: usize, message: String }` (impl `Display`); `parse_script(&str) -> Result<Vec<DevCommand>, ScriptError>`.

- [ ] **Step 1: Write the failing tests** (append to `src/command.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::keyboard::KeyCode;

    #[test]
    fn parses_spec_example() {
        let src = r#"
# comment
{"cmd":"wait","frames":30}
{"cmd":"record","every":2,"frames":0}
{"cmd":"drag","from":[300,200],"to":[520,260],"frames":90}
{"cmd":"dump","components":["ccc::EdgeComponent"]}
{"cmd":"quit"}
"#;
        let cmds = parse_script(src).unwrap();
        assert_eq!(cmds.len(), 5);
        assert_eq!(cmds[0], DevCommand::Wait { frames: 30 });
        assert_eq!(
            cmds[2],
            DevCommand::Drag { from: [300.0, 200.0], to: [520.0, 260.0], frames: 90, button: Button::Left }
        );
        assert_eq!(
            cmds[3],
            DevCommand::Dump { resources: vec![], components: vec!["ccc::EdgeComponent".into()] }
        );
        assert_eq!(cmds[4], DevCommand::Quit);
    }

    #[test]
    fn key_and_button_fields() {
        let cmds = parse_script(
            "{\"cmd\":\"key\",\"key\":\"Space\",\"action\":\"tap\"}\n{\"cmd\":\"down\",\"button\":\"right\"}",
        )
        .unwrap();
        assert_eq!(cmds[0], DevCommand::Key { key: KeyCode::Space, action: KeyAction::Tap });
        assert_eq!(cmds[1], DevCommand::Down { button: Button::Right });
    }

    #[test]
    fn error_reports_one_based_line_including_comments() {
        let err = parse_script("# c\n\n{\"cmd\":\"nope\"}").unwrap_err();
        assert_eq!(err.line, 3);
    }

    #[test]
    fn validate_rejects_zero_frame_drag() {
        let err = parse_script("{\"cmd\":\"drag\",\"from\":[0,0],\"to\":[1,1],\"frames\":0}").unwrap_err();
        assert_eq!(err.line, 1);
        assert!(err.message.contains("drag.frames"), "{}", err.message);
    }

    #[test]
    fn validate_rejects_zero_record_every() {
        assert!(parse_script("{\"cmd\":\"record\",\"every\":0,\"frames\":0}").is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib command`
Expected: FAIL to compile (`parse_script`, `DevCommand` undefined).

- [ ] **Step 3: Implement** (add above the tests in `src/command.rs`)

```rust
use bevy::input::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyAction {
    Press,
    Release,
    Tap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase", deny_unknown_fields)]
pub enum DevCommand {
    Move { x: f32, y: f32 },
    Down { #[serde(default)] button: Button },
    Up { #[serde(default)] button: Button },
    Click { x: f32, y: f32, #[serde(default)] button: Button },
    Drag { from: [f32; 2], to: [f32; 2], frames: u32, #[serde(default)] button: Button },
    Key { key: KeyCode, action: KeyAction },
    Wait { frames: u32 },
    Pause,
    Resume,
    Step { frames: u32 },
    Screenshot { #[serde(default)] name: Option<String> },
    Record { every: u32, frames: u32 },
    Dump { #[serde(default)] resources: Vec<String>, #[serde(default)] components: Vec<String> },
    Mark { label: String },
    Quit,
}

impl DevCommand {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            DevCommand::Drag { frames: 0, .. } => Err("drag.frames must be >= 1".into()),
            DevCommand::Record { every: 0, .. } => Err("record.every must be >= 1".into()),
            DevCommand::Step { frames: 0 } => Err("step.frames must be >= 1".into()),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ScriptError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "script line {}: {}", self.line, self.message)
    }
}

/// One command per line; blank lines and lines starting with `#` are ignored.
pub fn parse_script(src: &str) -> Result<Vec<DevCommand>, ScriptError> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |message: String| ScriptError { line: i + 1, message };
        let cmd: DevCommand = serde_json::from_str(line).map_err(|e| err(e.to_string()))?;
        cmd.validate().map_err(err)?;
        out.push(cmd);
    }
    Ok(out)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib command`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src/command.rs
git commit -m "feat: DevCommand and JSONL script parsing"
```

---

### Task 3: Scheduler (pure frame logic)

**Files:**
- Create: `src/scheduler.rs`
- Modify: `src/lib.rs` (add `pub mod scheduler;`)

**Interfaces:**
- Consumes: `DevCommand`, `Button`, `KeyAction` (Task 2).
- Produces:
  `enum Action { Cursor(Vec2), Button(Button, bool), Key(KeyCode, bool), Screenshot(String), Dump { resources: Vec<String>, components: Vec<String> }, Mark(String), SetPaused(bool), Quit }`;
  `struct Tick { pub started: Vec<DevCommand>, pub actions: Vec<Action> }`;
  `Scheduler::new(Vec<DevCommand>, exit_when_empty: bool)`, `push(DevCommand)`, `tick(frame: u32) -> Tick`.

Frame rules (document in code): a command with cost N occupies the current tick and N-1 more; the next command starts on the tick after. `drag` with `frames = n` emits cursor positions `k/n` for `k = 0..=n` (n+1 ticks), button down at k=0, up at k=n. `click` and `key tap` release on the next tick *without* occupying it. Screenshots auto-named `{frame:06}`.

- [ ] **Step 1: Write the failing tests** (in `src/scheduler.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Button, DevCommand as C, KeyAction};
    use bevy::input::keyboard::KeyCode;

    fn run(cmds: Vec<C>, frames: u32) -> Vec<Vec<Action>> {
        let mut s = Scheduler::new(cmds, true);
        (0..frames).map(|f| s.tick(f).actions).collect()
    }

    #[test]
    fn instant_commands_share_a_tick() {
        let t = run(vec![C::Move { x: 1.0, y: 2.0 }, C::Down { button: Button::Left }, C::Wait { frames: 5 }], 1);
        assert_eq!(t[0], vec![Action::Cursor(Vec2::new(1.0, 2.0)), Action::Button(Button::Left, true)]);
    }

    #[test]
    fn wait_occupies_n_ticks() {
        let t = run(vec![C::Wait { frames: 3 }, C::Mark { label: "a".into() }, C::Wait { frames: 9 }], 5);
        assert!(t[0].is_empty() && t[1].is_empty() && t[2].is_empty());
        assert_eq!(t[3], vec![Action::Mark("a".into())]);
    }

    #[test]
    fn drag_interpolates_and_releases() {
        let t = run(vec![C::Drag { from: [0.0, 0.0], to: [10.0, 0.0], frames: 2, button: Button::Left }, C::Wait { frames: 9 }], 3);
        assert_eq!(t[0], vec![Action::Cursor(Vec2::ZERO), Action::Button(Button::Left, true)]);
        assert_eq!(t[1], vec![Action::Cursor(Vec2::new(5.0, 0.0))]);
        assert_eq!(t[2], vec![Action::Cursor(Vec2::new(10.0, 0.0)), Action::Button(Button::Left, false)]);
    }

    #[test]
    fn click_releases_next_tick_without_blocking() {
        let t = run(vec![C::Click { x: 3.0, y: 4.0, button: Button::Left }, C::Mark { label: "m".into() }, C::Wait { frames: 9 }], 2);
        assert_eq!(t[0], vec![Action::Cursor(Vec2::new(3.0, 4.0)), Action::Button(Button::Left, true)]);
        assert_eq!(t[1], vec![Action::Button(Button::Left, false), Action::Mark("m".into())]);
    }

    #[test]
    fn key_tap_presses_then_releases() {
        let t = run(vec![C::Key { key: KeyCode::Space, action: KeyAction::Tap }, C::Wait { frames: 9 }], 2);
        assert_eq!(t[0], vec![Action::Key(KeyCode::Space, true)]);
        assert_eq!(t[1][0], Action::Key(KeyCode::Space, false));
    }

    #[test]
    fn record_screenshots_every_n_frames() {
        let t = run(vec![C::Record { every: 2, frames: 0 }, C::Wait { frames: 10 }], 5);
        let shots: Vec<u32> = (0..5).filter(|&f| t[f as usize].contains(&Action::Screenshot(format!("{f:06}")))).collect();
        assert_eq!(shots, vec![0, 2, 4]);
    }

    #[test]
    fn record_with_frames_stops_after() {
        let t = run(vec![C::Record { every: 1, frames: 2 }, C::Wait { frames: 10 }], 5);
        assert!(t[0].contains(&Action::Screenshot("000000".into())));
        assert!(t[1].contains(&Action::Screenshot("000001".into())));
        assert!(!t[3].iter().any(|a| matches!(a, Action::Screenshot(_))));
    }

    #[test]
    fn auto_quit_when_idle() {
        let t = run(vec![C::Mark { label: "x".into() }], 2);
        assert_eq!(t[0], vec![Action::Mark("x".into()), Action::Quit]);
        assert!(t[1].is_empty());
    }

    #[test]
    fn interactive_mode_never_auto_quits() {
        let mut s = Scheduler::new(vec![], false);
        assert!(s.tick(0).actions.is_empty());
    }

    #[test]
    fn quit_drops_remaining_commands() {
        let t = run(vec![C::Quit, C::Mark { label: "never".into() }], 2);
        assert_eq!(t[0], vec![Action::Quit]);
        assert!(t[1].is_empty());
    }

    #[test]
    fn step_unpauses_for_n_frames() {
        let t = run(vec![C::Step { frames: 2 }, C::Wait { frames: 9 }], 3);
        assert_eq!(t[0], vec![Action::SetPaused(false)]);
        assert!(t[1].is_empty());
        assert_eq!(t[2], vec![Action::SetPaused(true)]);
    }

    #[test]
    fn started_lists_commands_begun_this_tick() {
        let mut s = Scheduler::new(vec![C::Mark { label: "a".into() }, C::Wait { frames: 2 }], false);
        assert_eq!(s.tick(0).started.len(), 2);
        assert!(s.tick(1).started.is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib scheduler`
Expected: FAIL to compile (`Scheduler` undefined).

- [ ] **Step 3: Implement** (top of `src/scheduler.rs`; add `pub mod scheduler;` to `src/lib.rs`)

```rust
//! Pure command → per-frame action scheduling. No Bevy app needed.
//!
//! A command with cost N occupies the current tick and N-1 more; the next
//! command starts on the following tick. `drag` (frames = n) emits cursor
//! positions k/n for k = 0..=n. `click` / key `tap` release on the next tick
//! without occupying it.

use std::collections::VecDeque;

use bevy::input::keyboard::KeyCode;
use bevy::math::Vec2;

use crate::command::{Button, DevCommand, KeyAction};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Cursor(Vec2),
    Button(Button, bool),
    Key(KeyCode, bool),
    Screenshot(String),
    Dump { resources: Vec<String>, components: Vec<String> },
    Mark(String),
    SetPaused(bool),
    Quit,
}

#[derive(Debug, Default)]
pub struct Tick {
    pub started: Vec<DevCommand>,
    pub actions: Vec<Action>,
}

#[derive(Debug)]
enum Busy {
    /// Further ticks to occupy.
    Wait(u32),
    Drag { from: Vec2, to: Vec2, frames: u32, button: Button, k: u32 },
    ReleaseButton(Button),
    ReleaseKey(KeyCode),
    Step(u32),
    RecordFor(u32),
}

#[derive(Debug, Default)]
pub struct Scheduler {
    queue: VecDeque<DevCommand>,
    busy: Option<Busy>,
    record_every: Option<u32>,
    exit_when_empty: bool,
    quit_sent: bool,
}

impl Scheduler {
    pub fn new(cmds: Vec<DevCommand>, exit_when_empty: bool) -> Self {
        Self { queue: cmds.into(), exit_when_empty, ..Default::default() }
    }

    pub fn push(&mut self, cmd: DevCommand) {
        self.queue.push_back(cmd);
    }

    pub fn tick(&mut self, frame: u32) -> Tick {
        let mut t = Tick::default();
        if self.quit_sent {
            return t;
        }
        let mut consumed = self.advance(&mut t.actions);
        while !consumed && !self.quit_sent {
            let Some(cmd) = self.queue.pop_front() else { break };
            t.started.push(cmd.clone());
            consumed = self.start(cmd, frame, &mut t.actions);
        }
        if let Some(every) = self.record_every
            && frame % every == 0
        {
            let shot = Action::Screenshot(format!("{frame:06}"));
            if !t.actions.contains(&shot) {
                t.actions.push(shot);
            }
        }
        if !consumed && !self.quit_sent && self.exit_when_empty && self.busy.is_none() && self.queue.is_empty() {
            self.quit_sent = true;
            t.actions.push(Action::Quit);
        }
        t
    }

    /// Advances the busy command. Returns true if it occupies this tick.
    fn advance(&mut self, out: &mut Vec<Action>) -> bool {
        let Some(busy) = self.busy.take() else { return false };
        match busy {
            Busy::Wait(0) => false,
            Busy::Wait(r) => { self.busy = Some(Busy::Wait(r - 1)); true }
            Busy::Drag { from, to, frames, button, k } => {
                out.push(Action::Cursor(from.lerp(to, k as f32 / frames as f32)));
                if k == frames {
                    out.push(Action::Button(button, false));
                } else {
                    self.busy = Some(Busy::Drag { from, to, frames, button, k: k + 1 });
                }
                true
            }
            Busy::ReleaseButton(b) => { out.push(Action::Button(b, false)); false }
            Busy::ReleaseKey(k) => { out.push(Action::Key(k, false)); false }
            Busy::Step(0) => { out.push(Action::SetPaused(true)); false }
            Busy::Step(r) => { self.busy = Some(Busy::Step(r - 1)); true }
            Busy::RecordFor(0) => { self.record_every = None; false }
            Busy::RecordFor(r) => { self.busy = Some(Busy::RecordFor(r - 1)); true }
        }
    }

    /// Starts a command. Returns true if it occupies this tick.
    fn start(&mut self, cmd: DevCommand, frame: u32, out: &mut Vec<Action>) -> bool {
        match cmd {
            DevCommand::Move { x, y } => { out.push(Action::Cursor(Vec2::new(x, y))); false }
            DevCommand::Down { button } => { out.push(Action::Button(button, true)); false }
            DevCommand::Up { button } => { out.push(Action::Button(button, false)); false }
            DevCommand::Click { x, y, button } => {
                out.push(Action::Cursor(Vec2::new(x, y)));
                out.push(Action::Button(button, true));
                self.busy = Some(Busy::ReleaseButton(button));
                true
            }
            DevCommand::Drag { from, to, frames, button } => {
                let from = Vec2::from(from);
                out.push(Action::Cursor(from));
                out.push(Action::Button(button, true));
                self.busy = Some(Busy::Drag { from, to: Vec2::from(to), frames, button, k: 1 });
                true
            }
            DevCommand::Key { key, action } => match action {
                KeyAction::Press => { out.push(Action::Key(key, true)); false }
                KeyAction::Release => { out.push(Action::Key(key, false)); false }
                KeyAction::Tap => {
                    out.push(Action::Key(key, true));
                    self.busy = Some(Busy::ReleaseKey(key));
                    true
                }
            },
            DevCommand::Wait { frames: 0 } => false,
            DevCommand::Wait { frames } => { self.busy = Some(Busy::Wait(frames - 1)); true }
            DevCommand::Pause => { out.push(Action::SetPaused(true)); false }
            DevCommand::Resume => { out.push(Action::SetPaused(false)); false }
            DevCommand::Step { frames } => {
                out.push(Action::SetPaused(false));
                self.busy = Some(Busy::Step(frames - 1));
                true
            }
            DevCommand::Screenshot { name } => {
                out.push(Action::Screenshot(name.unwrap_or_else(|| format!("{frame:06}"))));
                false
            }
            DevCommand::Record { every, frames } => {
                self.record_every = Some(every);
                if frames == 0 {
                    false
                } else {
                    self.busy = Some(Busy::RecordFor(frames - 1));
                    true
                }
            }
            DevCommand::Dump { resources, components } => { out.push(Action::Dump { resources, components }); false }
            DevCommand::Mark { label } => { out.push(Action::Mark(label)); false }
            DevCommand::Quit => {
                self.queue.clear();
                self.quit_sent = true;
                out.push(Action::Quit);
                false
            }
        }
    }
}
```

Note: `click_releases_next_tick_without_blocking` expects tick 1 = `[Button up, Mark]`. `ReleaseButton` returns `false` from `advance`, so the loop then starts `Mark` in the same tick — matches.

- [ ] **Step 4: Run tests**

Run: `cargo test --lib scheduler`
Expected: 12 passed. If `step_unpauses_for_n_frames` fails, re-check: `Step{2}` → tick0 `SetPaused(false)` + `Busy::Step(1)`; tick1 `Step(1)→Step(0)` occupied; tick2 `Step(0)` → `SetPaused(true)`.

- [ ] **Step 5: Commit**

```bash
git add src/scheduler.rs src/lib.rs
git commit -m "feat: pure frame scheduler for dev commands"
```

---

### Task 4: Plugin, runner, capture, dump, clock — batch mode end to end

**Files:**
- Create: `src/capture.rs`, `src/dump.rs`, `src/runner.rs`, `scenarios/demo_drag.jsonl`, `tests/demo_e2e.rs`
- Modify: `src/lib.rs` (plugin + `DevState`), `examples/demo.rs` (remove spike, add plugin)

**Interfaces:**
- Consumes: `Scheduler`, `Action`, `Tick` (Task 3); `parse_script` (Task 2); `input::*` (Task 1).
- Produces: `ConsoleDevToolsPlugin` (unit struct, `Default`);
  `#[derive(Resource)] DevState { pub scheduler: Scheduler, pub out: PathBuf, pub batch: bool, pub max_frames: u32, pub exit_code: u8, pub quitting: Option<u32>, pub cursor: Option<Vec2>, pub remote_seq: u32 }` with `DevState::log(&self, serde_json::Value)`;
  `dump::dump_json(&mut World, &[String], &[String]) -> Result<Value, String>`;
  `capture::screenshot(&mut World, PathBuf)`.

- [ ] **Step 1: Write the failing dump tests** (`src/dump.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use bevy::reflect::TypePath;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct Foo {
        v: i32,
    }

    #[test]
    fn dump_component_values() {
        let mut app = App::new();
        app.register_type::<Foo>();
        app.world_mut().spawn(Foo { v: 7 });
        let v = dump_json(app.world_mut(), &[], &[Foo::type_path().to_string()]).unwrap();
        assert!(v.to_string().contains("\"v\":7"), "{v}");
    }

    #[test]
    fn dump_unknown_component_is_error() {
        let mut app = App::new();
        assert!(dump_json(app.world_mut(), &[], &["nope::Missing".into()]).is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib dump`
Expected: FAIL to compile (`dump_json` undefined).

- [ ] **Step 3: Implement `src/dump.rs`** (above the tests)

```rust
//! State dumps reuse BRP's builtin reflection handlers (same output as `world.query`).

use bevy::ecs::system::In;
use bevy::prelude::World;
use bevy::remote::builtin_methods::{process_remote_get_resources_request, process_remote_query_request};
use serde_json::{Map, Value, json};

pub fn dump_json(world: &mut World, resources: &[String], components: &[String]) -> Result<Value, String> {
    let mut res = Map::new();
    for r in resources {
        let v = process_remote_get_resources_request(In(Some(json!({ "resource": r }))), world)
            .map_err(|e| format!("resource {r}: {}", e.message))?;
        res.insert(r.clone(), v);
    }
    let mut comps = Map::new();
    for c in components {
        // strict: a misspelled type must fail instead of silently returning nothing
        let v = process_remote_query_request(
            In(Some(json!({ "data": { "components": [c] }, "strict": true }))),
            world,
        )
        .map_err(|e| format!("component {c}: {}", e.message))?;
        comps.insert(c.clone(), v);
    }
    Ok(json!({ "resources": res, "components": comps }))
}
```

Run: `cargo test --lib dump` → Expected: 2 passed. (If `dump_unknown_component_is_error` passes but `dump_component_values` fails because the registry is missing, add `app.add_plugins(bevy::MinimalPlugins)` in the test.)

- [ ] **Step 4: Write `src/capture.rs`**

```rust
use std::path::PathBuf;

use bevy::prelude::World;
use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};

/// Captures the primary window. The file is written a few frames later (async GPU readback).
pub fn screenshot(world: &mut World, path: PathBuf) {
    world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
}
```

- [ ] **Step 5: Write `src/lib.rs`**

```rust
pub mod capture;
pub mod command;
pub mod dump;
pub mod input;
mod runner;
pub mod scheduler;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::winit::{UpdateMode, WinitSettings};

use scheduler::Scheduler;

#[derive(Default)]
pub struct ConsoleDevToolsPlugin;

#[derive(Resource)]
pub struct DevState {
    pub scheduler: Scheduler,
    pub out: PathBuf,
    pub batch: bool,
    pub max_frames: u32,
    pub exit_code: u8,
    /// Frame at which quitting began (waiting for pending screenshots).
    pub quitting: Option<u32>,
    /// Last injected cursor, re-asserted every frame so a real mouse cannot override it.
    pub cursor: Option<Vec2>,
    pub remote_seq: u32,
}

impl DevState {
    /// Appends one JSON line to `<out>/events.jsonl`.
    pub fn log(&self, v: serde_json::Value) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.out.join("events.jsonl")) {
            let _ = writeln!(f, "{v}");
        }
    }
}

impl Plugin for ConsoleDevToolsPlugin {
    fn build(&self, app: &mut App) {
        let out = PathBuf::from(std::env::var("BCDT_OUT").unwrap_or_else(|_| "bcdt_out".into()));
        let max_frames = std::env::var("BCDT_MAX_FRAMES").ok().and_then(|s| s.parse().ok()).unwrap_or(36_000);
        let script = std::env::var("BCDT_SCRIPT").ok();
        let batch = script.is_some();

        let cmds = match &script {
            Some(path) => {
                let src = fs::read_to_string(path).unwrap_or_else(|e| {
                    eprintln!("bcdt: cannot read {path}: {e}");
                    std::process::exit(2);
                });
                command::parse_script(&src).unwrap_or_else(|e| {
                    eprintln!("bcdt: {path}: {e}");
                    std::process::exit(2);
                })
            }
            None => Vec::new(),
        };
        for d in ["frames", "dump"] {
            fs::create_dir_all(out.join(d)).expect("bcdt: cannot create output dir");
        }

        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .insert_resource(WinitSettings { focused_mode: UpdateMode::Continuous, unfocused_mode: UpdateMode::Continuous })
            .insert_resource(DevState {
                scheduler: Scheduler::new(cmds, batch),
                out,
                batch,
                max_frames,
                exit_code: 0,
                quitting: None,
                cursor: None,
                remote_seq: 0,
            })
            .add_systems(First, runner::run);
    }
}
```

- [ ] **Step 6: Write `src/runner.rs`**

```rust
use bevy::app::AppExit;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::view::window::screenshot::Screenshot;
use serde_json::json;

use crate::scheduler::Action;
use crate::{DevState, capture, dump, input};

/// Max frames to wait for pending screenshots before exiting anyway.
const QUIT_GRACE_FRAMES: u32 = 120;

pub fn run(world: &mut World) {
    let frame = world.resource::<FrameCount>().0;

    if let Some(since) = world.resource::<DevState>().quitting {
        let pending = world.query::<&Screenshot>().iter(world).count();
        if pending == 0 || frame - since > QUIT_GRACE_FRAMES {
            let code = world.resource::<DevState>().exit_code;
            world.write_message(AppExit::from_code(code));
        }
        return;
    }

    if frame > world.resource::<DevState>().max_frames {
        fail(world, frame, "BCDT_MAX_FRAMES exceeded".into());
        world.resource_mut::<DevState>().quitting = Some(frame);
        return;
    }

    let tick = world.resource_mut::<DevState>().scheduler.tick(frame);
    for c in &tick.started {
        world.resource::<DevState>().log(json!({ "frame": frame, "cmd": c }));
    }
    for action in tick.actions {
        apply(world, frame, action);
    }

    if let Some(p) = world.resource::<DevState>().cursor {
        input::hold_cursor(world, p);
    }
}

fn apply(world: &mut World, frame: u32, action: Action) {
    match action {
        Action::Cursor(p) => {
            world.resource_mut::<DevState>().cursor = Some(p);
            input::set_cursor(world, p);
        }
        Action::Button(b, down) => input::button(world, b, down),
        Action::Key(k, down) => input::key(world, k, down),
        Action::Screenshot(name) => {
            let path = world.resource::<DevState>().out.join("frames").join(format!("{name}.png"));
            world.resource::<DevState>().log(json!({ "frame": frame, "result": path }));
            capture::screenshot(world, path);
        }
        Action::Dump { resources, components } => match dump::dump_json(world, &resources, &components) {
            Ok(v) => {
                let path = world.resource::<DevState>().out.join("dump").join(format!("{frame:06}.json"));
                match std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()) {
                    Ok(()) => world.resource::<DevState>().log(json!({ "frame": frame, "result": path })),
                    Err(e) => fail(world, frame, format!("dump write: {e}")),
                }
            }
            Err(e) => fail(world, frame, e),
        },
        Action::Mark(label) => world.resource::<DevState>().log(json!({ "frame": frame, "mark": label })),
        Action::SetPaused(p) => {
            let mut t = world.resource_mut::<Time<Virtual>>();
            if p { t.pause() } else { t.unpause() }
        }
        Action::Quit => world.resource_mut::<DevState>().quitting = Some(frame),
    }
}

/// Logs an error; in batch mode the run ends with exit code 1.
fn fail(world: &mut World, frame: u32, error: String) {
    eprintln!("bcdt: frame {frame}: {error}");
    let mut st = world.resource_mut::<DevState>();
    st.log(json!({ "frame": frame, "error": error }));
    if st.batch {
        st.exit_code = 1;
        st.quitting = Some(frame);
    }
}
```

- [ ] **Step 7: Wire the demo; remove the spike**

In `examples/demo.rs`: delete the `if std::env::var("BCDT_SPIKE")` block and the whole `spike_drag` function, and add the plugin:

```rust
    .add_plugins(bevy_console_devtools::ConsoleDevToolsPlugin)
    .register_type::<Square>()
    .add_systems(Startup, setup)
```

- [ ] **Step 8: Scenario file `scenarios/demo_drag.jsonl`**

```jsonl
# Drag the square from the window center to (600,150) and record it.
{"cmd":"wait","frames":10}
{"cmd":"record","every":5,"frames":0}
{"cmd":"drag","from":[400,300],"to":[600,150],"frames":60}
{"cmd":"wait","frames":5}
{"cmd":"dump","components":["demo::Square"]}
{"cmd":"screenshot","name":"final"}
{"cmd":"quit"}
```

- [ ] **Step 9: Write the e2e tests `tests/demo_e2e.rs`**

```rust
//! Needs a window (macOS local). Run: cargo test --test demo_e2e -- --ignored
use std::path::Path;
use std::process::Command;

fn run_demo(script: &str, out: &Path) -> std::process::ExitStatus {
    let _ = std::fs::remove_dir_all(out);
    Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--example", "demo"])
        .env("BCDT_SCRIPT", script)
        .env("BCDT_OUT", out)
        .env("BCDT_MAX_FRAMES", "2000")
        .status()
        .expect("cargo run")
}

#[test]
#[ignore]
fn drag_scenario_moves_square_and_records() {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e_drag");
    assert!(run_demo("scenarios/demo_drag.jsonl", &out).success());

    let pngs = std::fs::read_dir(out.join("frames")).unwrap().count();
    assert!(pngs >= 10, "only {pngs} frames");
    assert!(out.join("frames/final.png").exists(), "last screenshot lost on quit");

    let dump = std::fs::read_dir(out.join("dump")).unwrap().next().unwrap().unwrap().path();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
    let pos = &v.to_string();
    // window (600,150) on an 800x600 window → world (200, 150)
    assert!(pos.contains("200") && pos.contains("150"), "{pos}");
}

#[test]
#[ignore]
fn bad_script_exits_2() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let script = dir.join("bad.jsonl");
    std::fs::write(&script, "{\"cmd\":\"wait\",\"frames\":1}\n{\"cmd\":\"bogus\"}\n").unwrap();
    let status = run_demo(script.to_str().unwrap(), &dir.join("e2e_bad"));
    assert_eq!(status.code(), Some(2));
}
```

- [ ] **Step 10: Run all tests**

Run: `cargo test --lib` → Expected: all unit tests pass.
Run: `cargo test --test demo_e2e -- --ignored` (do not touch the mouse) → Expected: 2 passed.
Then look at 2–3 PNGs in `target/tmp/e2e_drag/frames/` with an image viewer and confirm the square moves along the drag.

- [ ] **Step 11: Re-check the OS cursor with the real plugin**

```bash
swift /tmp/cursor.swift > /tmp/c_before.txt
BCDT_SCRIPT=scenarios/demo_drag.jsonl BCDT_OUT=bcdt_out cargo run --quiet --example demo
swift /tmp/cursor.swift > /tmp/c_after.txt
diff /tmp/c_before.txt /tmp/c_after.txt && echo "OS cursor unchanged"
```
Expected: `OS cursor unchanged`.

- [ ] **Step 12: Commit**

```bash
git add src examples scenarios tests
git commit -m "feat: batch mode - script runner, capture, dump, fixed clock"
```

---

### Task 5: Interactive mode over BRP + README

**Files:**
- Create: `src/remote.rs`, `README.md`
- Modify: `src/lib.rs` (add `pub mod remote;`, register BRP when not batch)

**Interfaces:**
- Consumes: `DevState`, `DevCommand`, `Scheduler::push`.
- Produces: `remote::METHODS: &[&str]`; `remote::handle(name: &str, params: Option<Value>, st: &mut DevState, frame: u32) -> BrpResult`; `remote::plugin() -> RemotePlugin`.

- [ ] **Step 1: Write the failing tests** (`src/remote.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::{Action, Scheduler};
    use serde_json::json;
    use std::path::PathBuf;

    fn state() -> DevState {
        DevState {
            scheduler: Scheduler::new(vec![], false),
            out: PathBuf::from("o"),
            batch: false,
            max_frames: 100,
            exit_code: 0,
            quitting: None,
            cursor: None,
            remote_seq: 0,
        }
    }

    #[test]
    fn click_is_queued() {
        let mut st = state();
        let r = handle("click", Some(json!({"x": 1, "y": 2})), &mut st, 7).unwrap();
        assert_eq!(r["frame"], 7);
        assert_eq!(st.scheduler.tick(8).actions[1], Action::Button(crate::command::Button::Left, true));
    }

    #[test]
    fn screenshot_without_name_gets_unique_path() {
        let mut st = state();
        let a = handle("screenshot", None, &mut st, 3).unwrap();
        let b = handle("screenshot", None, &mut st, 3).unwrap();
        assert_ne!(a["path"], b["path"]);
        assert!(a["path"].as_str().unwrap().ends_with(".png"));
    }

    #[test]
    fn bad_params_are_invalid_params_error() {
        let mut st = state();
        let e = handle("drag", Some(json!({"from": [0, 0]})), &mut st, 0).unwrap_err();
        assert_eq!(e.code, bevy::remote::error_codes::INVALID_PARAMS);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib remote` → Expected: FAIL to compile (`handle` undefined).

- [ ] **Step 3: Implement `src/remote.rs`** (above the tests)

```rust
//! Interactive mode: every DevCommand is a BRP method `devtools.<cmd>`.

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::In;
use bevy::prelude::*;
use bevy::remote::{BrpError, BrpResult, RemotePlugin, error_codes};
use serde_json::{Map, Value, json};

use crate::DevState;
use crate::command::DevCommand;

pub const METHODS: &[&str] = &[
    "move", "down", "up", "click", "drag", "key", "wait", "pause", "resume", "step",
    "screenshot", "record", "dump", "mark", "quit",
];

pub fn plugin() -> RemotePlugin {
    METHODS.iter().fold(RemotePlugin::default(), |p, &name| {
        p.with_method_main(
            format!("devtools.{name}"),
            move |In(params): In<Option<Value>>, mut st: ResMut<DevState>, fc: Res<FrameCount>| {
                handle(name, params, &mut st, fc.0)
            },
        )
    })
}

/// Parses `params` as the `name` command and queues it. Returns `{frame, queued, path}`.
pub fn handle(name: &str, params: Option<Value>, st: &mut DevState, frame: u32) -> BrpResult {
    let mut obj = match params {
        Some(Value::Object(m)) => m,
        None => Map::new(),
        Some(_) => return Err(invalid("params must be an object")),
    };
    obj.insert("cmd".into(), Value::String(name.into()));
    let mut cmd: DevCommand = serde_json::from_value(Value::Object(obj)).map_err(invalid)?;
    cmd.validate().map_err(invalid)?;

    let mut path = None;
    if let DevCommand::Screenshot { name } = &mut cmd {
        st.remote_seq += 1;
        let seq = st.remote_seq;
        let n = name.get_or_insert_with(|| format!("remote_{frame:06}_{seq}"));
        path = Some(st.out.join("frames").join(format!("{n}.png")));
    }
    st.scheduler.push(cmd);
    Ok(json!({ "frame": frame, "queued": true, "path": path }))
}

fn invalid(e: impl ToString) -> BrpError {
    BrpError { code: error_codes::INVALID_PARAMS, message: e.to_string(), data: None }
}
```

- [ ] **Step 4: Register BRP in interactive mode** (`src/lib.rs`)

Add `pub mod remote;` and at the end of `build`:
```rust
        if !batch {
            // ponytail: panics if the game already adds RemotePlugin; merge methods then
            app.add_plugins((remote::plugin(), bevy::remote::http::RemoteHttpPlugin::default()));
        }
```

- [ ] **Step 5: Run unit tests**

Run: `cargo test --lib` → Expected: all pass (3 new).

- [ ] **Step 6: Manual interactive check**

```bash
cargo run --quiet --example demo &
sleep 8
rpc() { curl -s localhost:15702 -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}"; echo; }
rpc devtools.drag '{"from":[400,300],"to":[200,450],"frames":30}'
sleep 1
rpc devtools.screenshot '{}'
sleep 1
rpc world.query '{"data":{"components":["demo::Square"]}}'
rpc devtools.quit '{}'
wait
```
Expected: drag → `{"frame":…,"queued":true,"path":null}`; screenshot → a `path` that exists as a PNG; `world.query` shows `pos` ≈ `[-200, -150]`; the process exits 0.

- [ ] **Step 7: Write `README.md`**

````markdown
# BevyConsoleDevTools

Lets an AI agent (or you) drive a Bevy 0.20 game from the shell — no screen-control tools,
no OS permissions: scripted input, PNG frame capture, and reflected state dumps.

## Integrate

```toml
[dependencies]
bevy_console_devtools = { git = "https://github.com/rayray-swamp/BevyConsoleDevTools", optional = true }

[features]
devtools = ["dep:bevy_console_devtools"]
```

```rust
#[cfg(feature = "devtools")]
app.add_plugins(bevy_console_devtools::ConsoleDevToolsPlugin);
```

Types you want to `dump` need `#[derive(Reflect)]` + `#[reflect(Component)]` / `#[reflect(Resource)]`.

## Batch mode

```bash
BCDT_SCRIPT=scenarios/demo_drag.jsonl BCDT_OUT=out cargo run --features devtools
```

One JSON command per line (`#` comments allowed). Outputs: `out/frames/*.png`,
`out/events.jsonl`, `out/dump/<frame>.json`. Exit code: 0 ok, 1 runtime error / `BCDT_MAX_FRAMES`
(default 36000), 2 bad script.

## Interactive mode

Run without `BCDT_SCRIPT`; send the same commands as BRP methods on `localhost:15702`:

```bash
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"devtools.click","params":{"x":300,"y":200}}'
```

BRP builtins (`world.query`, `world.get_resources`, …) work too.

## Commands

Coordinates are window logical pixels, origin top-left. Time advances a fixed 1/60 s per frame.

| cmd | params | frames |
|---|---|---|
| `move` | `x, y` | 0 |
| `down` / `up` | `button?` (`left`/`right`/`middle`) | 0 |
| `click` | `x, y, button?` | 1 |
| `drag` | `from:[x,y], to:[x,y], frames, button?` | frames+1 |
| `key` | `key` (Bevy `KeyCode` name, e.g. `Space`, `KeyA`), `action` (`press`/`release`/`tap`) | 0 (tap 1) |
| `wait` | `frames` | frames |
| `pause` / `resume` | – | 0 |
| `step` | `frames` (runs N frames, then pauses) | frames |
| `screenshot` | `name?` | 0 |
| `record` | `every, frames` (`frames:0` = until quit) | frames |
| `dump` | `resources?:[type path], components?:[type path]` | 0 |
| `mark` | `label` | 0 |
| `quit` | – | – |

## Caveats

- The OS cursor is never moved; the game sees an injected cursor position. Don't hover the
  window during batch runs (the injected position is re-asserted every frame anyway).
- Injected keys carry no text: games reading `ButtonInput<KeyCode>` work, text fields don't.
- Needs a window (macOS/desktop). Offscreen capture is planned.
````

- [ ] **Step 8: Commit**

```bash
git add src README.md
git commit -m "feat: interactive mode over BRP and README"
```
