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
            Busy::Wait(r) => {
                self.busy = Some(Busy::Wait(r - 1));
                true
            }
            Busy::Drag { from, to, frames, button, k } => {
                out.push(Action::Cursor(from.lerp(to, k as f32 / frames as f32)));
                if k == frames {
                    out.push(Action::Button(button, false));
                } else {
                    self.busy = Some(Busy::Drag { from, to, frames, button, k: k + 1 });
                }
                true
            }
            Busy::ReleaseButton(b) => {
                out.push(Action::Button(b, false));
                false
            }
            Busy::ReleaseKey(k) => {
                out.push(Action::Key(k, false));
                false
            }
            Busy::Step(0) => {
                out.push(Action::SetPaused(true));
                false
            }
            Busy::Step(r) => {
                self.busy = Some(Busy::Step(r - 1));
                true
            }
            Busy::RecordFor(0) => {
                self.record_every = None;
                false
            }
            Busy::RecordFor(r) => {
                self.busy = Some(Busy::RecordFor(r - 1));
                true
            }
        }
    }

    /// Starts a command. Returns true if it occupies this tick.
    fn start(&mut self, cmd: DevCommand, frame: u32, out: &mut Vec<Action>) -> bool {
        match cmd {
            DevCommand::Move { x, y } => {
                out.push(Action::Cursor(Vec2::new(x, y)));
                false
            }
            DevCommand::Down { button } => {
                out.push(Action::Button(button, true));
                false
            }
            DevCommand::Up { button } => {
                out.push(Action::Button(button, false));
                false
            }
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
                KeyAction::Press => {
                    out.push(Action::Key(key, true));
                    false
                }
                KeyAction::Release => {
                    out.push(Action::Key(key, false));
                    false
                }
                KeyAction::Tap => {
                    out.push(Action::Key(key, true));
                    self.busy = Some(Busy::ReleaseKey(key));
                    true
                }
            },
            DevCommand::Wait { frames: 0 } => false,
            DevCommand::Wait { frames } => {
                self.busy = Some(Busy::Wait(frames - 1));
                true
            }
            DevCommand::Pause => {
                out.push(Action::SetPaused(true));
                false
            }
            DevCommand::Resume => {
                out.push(Action::SetPaused(false));
                false
            }
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
            DevCommand::Dump { resources, components } => {
                out.push(Action::Dump { resources, components });
                false
            }
            DevCommand::Mark { label } => {
                out.push(Action::Mark(label));
                false
            }
            DevCommand::Quit => {
                self.queue.clear();
                self.quit_sent = true;
                out.push(Action::Quit);
                false
            }
        }
    }
}

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
