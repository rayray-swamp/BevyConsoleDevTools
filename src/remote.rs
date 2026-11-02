//! Interactive mode: every DevCommand is a BRP method `devtools.<cmd>`.

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::In;
use bevy::prelude::*;
use bevy::reflect::TypeRegistry;
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
            move |In(params): In<Option<Value>>,
                  mut st: ResMut<DevState>,
                  fc: Res<FrameCount>,
                  registry: Res<AppTypeRegistry>| {
                handle(name, params, &mut st, fc.0, &registry.read())
            },
        )
    })
}

/// Parses `params` as the `name` command and queues it. Returns `{frame, queued, path}`.
/// `dump` type paths are checked against `registry` now, so a typo is a JSON-RPC error.
pub fn handle(name: &str, params: Option<Value>, st: &mut DevState, frame: u32, registry: &TypeRegistry) -> BrpResult {
    let mut obj = match params {
        Some(Value::Object(m)) => m,
        None => Map::new(),
        Some(_) => return Err(invalid("params must be an object")),
    };
    obj.insert("cmd".into(), Value::String(name.into()));
    let mut cmd: DevCommand = serde_json::from_value(Value::Object(obj)).map_err(invalid)?;
    cmd.validate().map_err(invalid)?;
    if let DevCommand::Dump { resources, components } = &cmd
        && let Some(t) = resources.iter().chain(components).find(|t| registry.get_with_type_path(t).is_none())
    {
        return Err(invalid(format!("unknown type `{t}` (needs #[derive(Reflect)] and registration)")));
    }

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
    use bevy::reflect::TypeRegistry;
    use crate::scheduler::{Action, Scheduler};
    use serde_json::json;
    use std::path::PathBuf;

    fn reg() -> TypeRegistry {
        TypeRegistry::default()
    }

    fn state() -> DevState {
        DevState::new(Scheduler::new(vec![], false), PathBuf::from("o"), false, 100)
    }

    #[test]
    fn click_is_queued() {
        let mut st = state();
        let r = handle("click", Some(json!({"x": 1, "y": 2})), &mut st, 7, &reg()).unwrap();
        assert_eq!(r["frame"], 7);
        assert_eq!(st.scheduler.tick(8).actions[1], Action::Button(crate::command::Button::Left, true));
    }

    #[test]
    fn screenshot_without_name_gets_unique_path() {
        let mut st = state();
        let a = handle("screenshot", None, &mut st, 3, &reg()).unwrap();
        let b = handle("screenshot", None, &mut st, 3, &reg()).unwrap();
        assert_ne!(a["path"], b["path"]);
        assert!(a["path"].as_str().unwrap().ends_with(".png"));
    }

    #[test]
    fn bad_params_are_invalid_params_error() {
        let mut st = state();
        let e = handle("drag", Some(json!({"from": [0, 0]})), &mut st, 0, &reg()).unwrap_err();
        assert_eq!(e.code, bevy::remote::error_codes::INVALID_PARAMS);
    }

    #[test]
    fn dump_of_unknown_type_is_rejected_immediately() {
        let mut st = state();
        let e = handle("dump", Some(json!({"components": ["nope::Missing"]})), &mut st, 0, &reg()).unwrap_err();
        assert_eq!(e.code, bevy::remote::error_codes::INVALID_PARAMS);
        assert!(e.message.contains("nope::Missing"), "{}", e.message);
    }
}
