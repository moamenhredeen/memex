use freya::prelude::*;

use crate::editor::Editor;
use crate::theme;

pub fn app() -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::fill())
        .background(theme::BG_COLOR)
        .child(Editor)
}
