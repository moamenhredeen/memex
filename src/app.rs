use freya::prelude::*;

use crate::editor::Editor;
use crate::state::AppState;
use crate::statusbar::StatusBar;
use crate::theme;

pub fn app() -> impl IntoElement {
    let app_state = use_state(AppState::new);

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::BG_COLOR)
        .direction(Direction::Vertical)
        .child(
            // Editor takes all remaining space
            rect()
                .width(Size::fill())
                .height(Size::flex(1.))
                .child(Editor { app_state }),
        )
        .child(StatusBar { app_state })
}
