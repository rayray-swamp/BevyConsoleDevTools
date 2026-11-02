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
