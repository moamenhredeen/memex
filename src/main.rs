#![recursion_limit = "1024"]

mod app;
mod config;
mod editor;
mod fs;
mod markdown;
mod state;
mod vault;

use gpui::AppContext;

fn main() {
    gpui::Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);

            cx.bind_keys([
                gpui::KeyBinding::new("tab", editor::TabAction, Some("Editor")),
            ]);

            cx.open_window(
                gpui::WindowOptions {
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Memex".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| app::Memex::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                },
            )
            .expect("Failed to open window");
        });
}
