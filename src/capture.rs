use std::path::PathBuf;

use bevy::prelude::World;
use bevy::render::view::window::screenshot::{Screenshot, save_to_disk};

/// Captures the primary window. The file is written a few frames later (async GPU readback).
pub fn screenshot(world: &mut World, path: PathBuf) {
    world.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
}
