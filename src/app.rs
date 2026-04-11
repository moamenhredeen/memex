use freya::prelude::*;

use crate::editor::Editor;
use crate::state::AppState;
use crate::theme;

pub fn app() -> impl IntoElement {
    let app_state = use_state(AppState::new);

    rect()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::BG_COLOR)
        .child(Editor { app_state })
}
