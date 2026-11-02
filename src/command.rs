use bevy::input::keyboard::KeyCode;
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
