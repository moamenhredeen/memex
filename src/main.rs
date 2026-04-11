mod app;
mod editor;
mod fs;
mod markdown;
mod state;
mod theme;
mod vault;

use freya::prelude::*;

fn main() {
    launch(LaunchConfig::new().with_window(WindowConfig::new(app::app).with_title("Memex")));
}
