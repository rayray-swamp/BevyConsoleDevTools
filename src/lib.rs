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

use bevy::ecs::message::MessageUpdateSystems;
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
            .add_systems(First, runner::run.after(MessageUpdateSystems))
            .add_systems(PostUpdate, input::restore_real_cursor);
    }
}
