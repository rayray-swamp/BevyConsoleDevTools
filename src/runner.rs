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
    let batch = world.resource::<DevState>().batch;
    if batch {
        input::drop_real_input(world);
    }

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

    // Batch: the game sees only the virtual cursor (None until the first injection).
    // Interactive: real input stays usable until something is injected.
    let cursor = world.resource::<DevState>().cursor;
    if batch || cursor.is_some() {
        input::hold_cursor(world, cursor);
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
