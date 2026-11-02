use bevy::app::AppExit;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use serde_json::json;

use crate::scheduler::Action;
use crate::{DevState, capture, dump, input};

/// Max frames to wait for pending screenshots before exiting anyway.
const QUIT_GRACE_FRAMES: u32 = 120;

pub fn run(world: &mut World) {
    let frame = world.resource::<FrameCount>().0;
    let batch = world.resource::<DevState>().batch;
    if batch {
        input::drop_real_input(world);
    } else {
        release_on_real_move(world);
    }
    check_screenshots(world, frame);

    if let Some(since) = world.resource::<DevState>().quitting {
        let pending = world.resource::<DevState>().pending_shots.len();
        if pending == 0 || frame - since > QUIT_GRACE_FRAMES {
            if pending > 0 {
                fail(world, frame, format!("{pending} screenshot(s) not written before exit"));
            }
            finish(world);
        }
        return;
    }

    if batch && frame > world.resource::<DevState>().max_frames {
        fail(world, frame, "BCDT_MAX_FRAMES exceeded".into());
        return;
    }

    let tick = world.resource_mut::<DevState>().scheduler.tick(frame);
    for c in &tick.started {
        world.resource::<DevState>().log(json!({ "frame": frame, "cmd": c }));
    }
    for action in tick.actions {
        apply(world, frame, action);
    }

    // Batch: the game sees only the virtual cursor (None until the first injection).
    // Interactive: the virtual cursor holds until the real mouse moves.
    let cursor = world.resource::<DevState>().cursor;
    if batch || cursor.is_some() {
        input::hold_cursor(world, cursor);
    }
}

/// Exits with the recorded code. Batch calls `process::exit` so the code reaches the shell
/// even when the game's `main` discards `App::run`'s `AppExit`.
fn finish(world: &mut World) {
    let st = world.resource::<DevState>();
    let (code, batch) = (st.exit_code, st.batch);
    // unit tests exercise this path in-process; e2e covers the real exit
    if batch && !cfg!(test) {
        std::process::exit(code as i32);
    }
    world.write_message(AppExit::from_code(code));
}

/// Interactive: a real mouse move hands the cursor back to the user.
fn release_on_real_move(world: &mut World) {
    let real = input::primary_window(world)
        .and_then(|e| world.get::<Window>(e))
        .and_then(|w| w.cursor_position());
    let mut st = world.resource_mut::<DevState>();
    if st.cursor.is_some() && real != st.last_real {
        st.cursor = None;
    }
    st.last_real = real;
}

/// A screenshot entity is despawned once its file was written (or failed to be).
fn check_screenshots(world: &mut World, frame: u32) {
    let pending = std::mem::take(&mut world.resource_mut::<DevState>().pending_shots);
    let mut still = Vec::new();
    for (e, path) in pending {
        if world.get_entity(e).is_ok() {
            still.push((e, path));
        } else if path.exists() {
            world.resource::<DevState>().log(json!({ "frame": frame, "result": path }));
        } else {
            fail(world, frame, format!("screenshot not written: {}", path.display()));
        }
    }
    world.resource_mut::<DevState>().pending_shots.extend(still);
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
            let e = capture::screenshot(world, path.clone());
            world.resource_mut::<DevState>().pending_shots.push((e, path));
        }
        Action::Dump { resources, components } => match dump::dump_json(world, &resources, &components) {
            Ok(v) => {
                let path = dump_path(&world.resource::<DevState>().out, frame);
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

/// `dump/<frame>.json`, then `<frame>_2.json`, … for further dumps in the same frame.
fn dump_path(out: &std::path::Path, frame: u32) -> std::path::PathBuf {
    let dir = out.join("dump");
    let first = dir.join(format!("{frame:06}.json"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{frame:06}_{n}.json"))).find(|p| !p.exists()).unwrap()
}

/// Logs an error; in batch mode the run ends with exit code 1.
fn fail(world: &mut World, frame: u32, error: String) {
    eprintln!("bcdt: frame {frame}: {error}");
    let mut st = world.resource_mut::<DevState>();
    st.log(json!({ "frame": frame, "error": error }));
    if st.batch {
        st.exit_code = 1;
        st.quitting.get_or_insert(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::DevCommand;
    use crate::scheduler::Scheduler;
    use bevy::ecs::message::Messages;
    use bevy::window::PrimaryWindow;
    use std::path::{Path, PathBuf};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bcdt_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        crate::init_out(&d);
        d
    }

    fn world(cmds: Vec<DevCommand>, batch: bool, out: &Path) -> World {
        let mut w = World::new();
        w.insert_resource(FrameCount(5));
        w.insert_resource(Time::<Virtual>::default());
        w.init_resource::<Messages<AppExit>>();
        w.insert_resource(DevState::new(Scheduler::new(cmds, batch), out.to_path_buf(), batch, 36_000));
        w
    }

    #[test]
    fn interactive_mode_ignores_max_frames() {
        let out = tmp("maxf");
        let mut w = world(vec![], false, &out);
        w.resource_mut::<DevState>().max_frames = 0;
        run(&mut w);
        assert!(w.resource::<DevState>().quitting.is_none());
    }

    #[test]
    fn interactive_real_mouse_move_releases_virtual_cursor() {
        let out = tmp("release");
        let mut w = world(vec![], false, &out);
        let mut win = Window::default();
        win.set_cursor_position(Some(Vec2::new(5.0, 5.0)));
        let e = w.spawn((win, PrimaryWindow)).id();
        {
            let mut st = w.resource_mut::<DevState>();
            st.cursor = Some(Vec2::new(100.0, 100.0));
            st.last_real = Some(Vec2::new(5.0, 5.0));
        }
        run(&mut w);
        input::restore_real_cursor(&mut w);
        assert!(w.resource::<DevState>().cursor.is_some(), "still virtual while the real mouse is idle");
        w.get_mut::<Window>(e).unwrap().set_cursor_position(Some(Vec2::new(6.0, 6.0)));
        run(&mut w);
        assert!(w.resource::<DevState>().cursor.is_none(), "real mouse moved: virtual cursor released");
    }

    #[test]
    fn missing_screenshot_file_fails_batch() {
        let out = tmp("shotfail");
        let mut w = world(vec![DevCommand::Wait { frames: 9 }], true, &out);
        let e = w.spawn_empty().id();
        w.despawn(e);
        w.resource_mut::<DevState>().pending_shots.push((e, out.join("frames/missing.png")));
        run(&mut w);
        let st = w.resource::<DevState>();
        assert_eq!(st.exit_code, 1);
        assert!(st.pending_shots.is_empty());
    }

    #[test]
    fn written_screenshot_is_logged_once_done() {
        let out = tmp("shotok");
        let mut w = world(vec![DevCommand::Wait { frames: 9 }], true, &out);
        let path = out.join("frames/ok.png");
        std::fs::write(&path, b"png").unwrap();
        let e = w.spawn_empty().id();
        w.despawn(e);
        w.resource_mut::<DevState>().pending_shots.push((e, path));
        run(&mut w);
        assert_eq!(w.resource::<DevState>().exit_code, 0);
        let log = std::fs::read_to_string(out.join("events.jsonl")).unwrap();
        assert!(log.contains("ok.png"), "{log}");
    }

    #[test]
    fn two_dumps_in_one_frame_do_not_overwrite() {
        let out = tmp("dump2");
        let d = || DevCommand::Dump { resources: vec![], components: vec![] };
        let mut w = world(vec![d(), d(), DevCommand::Wait { frames: 9 }], false, &out);
        run(&mut w);
        assert_eq!(std::fs::read_dir(out.join("dump")).unwrap().count(), 2);
    }
}
