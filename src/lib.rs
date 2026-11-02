pub mod capture;
pub mod command;
pub mod dump;
pub mod input;
pub mod remote;
mod runner;
pub mod scheduler;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
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
    /// OS cursor seen last frame (interactive: a real move releases the virtual cursor).
    pub last_real: Option<Vec2>,
    /// Screenshots requested but not yet confirmed on disk.
    pub pending_shots: Vec<(Entity, PathBuf)>,
    pub remote_seq: u32,
}

impl DevState {
    pub fn new(scheduler: Scheduler, out: PathBuf, batch: bool, max_frames: u32) -> Self {
        Self {
            scheduler,
            out,
            batch,
            max_frames,
            exit_code: 0,
            quitting: None,
            cursor: None,
            last_real: None,
            pending_shots: Vec::new(),
            remote_seq: 0,
        }
    }

    /// Appends one JSON line to `<out>/events.jsonl`.
    pub fn log(&self, v: serde_json::Value) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.out.join("events.jsonl")) {
            let _ = writeln!(f, "{v}");
        }
    }
}

/// Creates `frames/` and `dump/` and starts a fresh `events.jsonl` (no leftovers from earlier runs).
pub(crate) fn init_out(out: &Path) {
    for d in ["frames", "dump"] {
        fs::create_dir_all(out.join(d)).expect("bcdt: cannot create output dir");
    }
    fs::write(out.join("events.jsonl"), "").expect("bcdt: cannot write events.jsonl");
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
        init_out(&out);

        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .insert_resource(WinitSettings { focused_mode: UpdateMode::Continuous, unfocused_mode: UpdateMode::Continuous })
            .insert_resource(DevState::new(Scheduler::new(cmds, batch), out, batch, max_frames))
            .add_systems(First, runner::run.after(MessageUpdateSystems))
            .add_systems(PostUpdate, input::restore_real_cursor);

        if !batch {
            // ponytail: panics if the game already adds RemotePlugin; merge methods then
            app.add_plugins((remote::plugin(), bevy::remote::http::RemoteHttpPlugin::default()));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn init_out_truncates_previous_events() {
        let d = std::env::temp_dir().join(format!("bcdt_test_initout_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("events.jsonl"), "{\"error\":\"old run\"}\n").unwrap();
        super::init_out(&d);
        assert_eq!(std::fs::read_to_string(d.join("events.jsonl")).unwrap(), "");
        assert!(d.join("frames").is_dir() && d.join("dump").is_dir());
    }
}
