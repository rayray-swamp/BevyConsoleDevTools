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
