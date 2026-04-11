mod app;
mod command_bar;
mod editor;
mod fs;
mod markdown;
mod state;
mod statusbar;
mod theme;
mod vault;

use freya::prelude::*;

fn main() {
    launch(LaunchConfig::new().with_window(WindowConfig::new(app::app).with_title("Memex")));
}
