mod app;
mod editor;
mod markdown;
mod theme;

use freya::prelude::*;

fn main() {
    launch(LaunchConfig::new().with_window(WindowConfig::new(app::app).with_title("Memex")));
}
