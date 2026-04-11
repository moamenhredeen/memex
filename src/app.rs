use freya::prelude::*;

use crate::command_bar::CommandBar;
use crate::editor::Editor;
use crate::state::AppState;
use crate::statusbar::StatusBar;

pub fn app() -> impl IntoElement {
    let app_state = use_state(AppState::new);
    let command_bar_visible = use_state(|| false);

    let bg_color = app_state.read().config.bg_color;

    // Global Ctrl+P handler
    let mut command_bar_visible_toggle = command_bar_visible;
    let on_global_key = move |e: Event<KeyboardEventData>| {
        if e.modifiers.contains(Modifiers::CONTROL)
            && e.key == Key::Character("p".to_string())
        {
            let current = *command_bar_visible_toggle.read();
            command_bar_visible_toggle.set(!current);
            e.stop_propagation();
        }
    };

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .background(bg_color)
        .direction(Direction::Vertical)
        .content(Content::Flex)
        .on_global_key_down(on_global_key)
        .child(Editor { app_state })
        .child(StatusBar { app_state })
        .child(CommandBar {
            app_state,
            visible: command_bar_visible,
        })
}
