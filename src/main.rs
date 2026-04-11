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

            // Embed FiraCode Nerd Font so we don't depend on system fonts
            let font_data: Vec<std::borrow::Cow<'static, [u8]>> = vec![
                std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/FiraCodeNerdFont-Regular.ttf")),
                std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/FiraCodeNerdFont-Bold.ttf")),
                std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/FiraCodeNerdFontMono-Regular.ttf")),
                std::borrow::Cow::Borrowed(include_bytes!("../assets/fonts/FiraCodeNerdFontMono-Bold.ttf")),
            ];
            cx.text_system().add_fonts(font_data).expect("Failed to load embedded fonts");

            cx.bind_keys([
                gpui::KeyBinding::new("tab", editor::TabAction, Some("Editor")),
                gpui::KeyBinding::new("shift-tab", editor::ShiftTabAction, Some("Editor")),
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
