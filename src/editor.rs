use freya::prelude::*;
use freya::text_edit::*;

use crate::markdown::{self, HeadingLevel};
use crate::state::AppState;
use crate::theme;

const WELCOME_TEXT: &str = "\
# Welcome to Memex

Open or create a vault to get started.
Use Ctrl+P to search and create notes.
";

/// WYSIWYG markdown editor — each line is rendered with its heading style
/// while remaining fully editable.
#[derive(PartialEq)]
pub struct Editor {
    pub app_state: State<AppState>,
}

impl Component for Editor {
    fn render(&self) -> impl IntoElement {
        let mut app_state = self.app_state;
        let initial_content = app_state.read().content.clone();
        let content_for_init = if initial_content.is_empty() {
            WELCOME_TEXT.to_string()
        } else {
            initial_content
        };

        let mut editable = use_editable(move || content_for_init.clone(), EditableConfig::new);

        // Sync editor content back to AppState on every change
        let editor_text = editable.editor().read().to_string();
        if editor_text != app_state.read().content {
            app_state.write().content = editor_text;
            app_state.write().dirty = true;
        }

        let on_global_pointer_press = move |_: Event<PointerEventData>| {
            editable.process_event(EditableEvent::Release);
        };

        let on_global_key_down = move |e: Event<KeyboardEventData>| {
            // Ctrl+S → save
            if e.modifiers.contains(Modifiers::CONTROL) && e.key == Key::Character("s".to_string())
            {
                let mut state = app_state.write();
                if let Err(err) = state.save() {
                    eprintln!("save error: {}", err);
                }
                return;
            }

            editable.process_event(EditableEvent::KeyDown {
                key: &e.key,
                modifiers: e.modifiers,
            });
        };

        let on_global_key_up = move |e: Event<KeyboardEventData>| {
            editable.process_event(EditableEvent::KeyUp { key: &e.key });
        };

        let line_count = editable.editor().read().len_lines();

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .background(theme::EDITOR_BG)
            .corner_radius(8.)
            .padding(24.)
            .on_global_pointer_press(on_global_pointer_press)
            .on_global_key_down(on_global_key_down)
            .on_global_key_up(on_global_key_up)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .children((0..line_count).map(|i| {
                            EditableLine {
                                line_index: i,
                                editable,
                            }
                            .into_element()
                        })),
                ),
            )
    }
}

/// A single editable line rendered with WYSIWYG markdown styling.
#[derive(PartialEq)]
struct EditableLine {
    line_index: usize,
    editable: UseEditable,
}

impl Component for EditableLine {
    fn render(&self) -> impl IntoElement {
        let line_index = self.line_index;
        let mut editable = self.editable;
        let holder = use_state(ParagraphHolder::default);
        let editor = editable.editor().read();
        let line = editor.line(line_index).unwrap();
        let line_text = line.text.to_string();

        let is_active = editor.cursor_row() == line_index;

        let cursor_index = if is_active {
            Some(editor.cursor_col())
        } else {
            None
        };

        let highlights = editable
            .editor()
            .read()
            .get_visible_selection(EditorLine::Paragraph(line_index));

        let on_mouse_down = move |e: Event<MouseEventData>| {
            editable.process_event(EditableEvent::Down {
                location: e.element_location,
                editor_line: EditorLine::Paragraph(line_index),
                holder: &holder.read(),
            });
        };

        let on_mouse_move = move |e: Event<MouseEventData>| {
            editable.process_event(EditableEvent::Move {
                location: e.element_location,
                editor_line: EditorLine::Paragraph(line_index),
                holder: &holder.read(),
            });
        };

        let styled = markdown::parse_line(&line_text);
        let (font_size, font_weight, color) = match styled.level {
            HeadingLevel::H1 => (theme::H1_SIZE, FontWeight::BOLD, theme::HEADING_COLOR),
            HeadingLevel::H2 => (theme::H2_SIZE, FontWeight::BOLD, theme::HEADING_COLOR),
            HeadingLevel::H3 => (theme::H3_SIZE, FontWeight::BOLD, theme::HEADING_COLOR),
            HeadingLevel::Body => (theme::BODY_SIZE, FontWeight::NORMAL, theme::TEXT_COLOR),
        };

        let display_text = if line_text.is_empty() {
            " ".to_string()
        } else {
            line_text
        };

        let mut p = paragraph()
            .holder(holder.read().clone())
            .on_mouse_down(on_mouse_down)
            .on_mouse_move(on_mouse_move)
            .cursor_index(cursor_index)
            .cursor_mode(CursorMode::Expanded)
            .highlights(highlights.map(|h| vec![h]))
            .width(Size::fill());

        // Render marker in a dimmed color, content in the heading color
        if !styled.marker.is_empty() {
            p = p.span(
                Span::new(styled.marker)
                    .font_size(font_size)
                    .font_weight(font_weight)
                    .color(theme::MARKER_COLOR),
            );
            p = p.span(
                Span::new(styled.content.clone())
                    .font_size(font_size)
                    .font_weight(font_weight)
                    .color(color),
            );
        } else {
            p = p.span(
                Span::new(display_text)
                    .font_size(font_size)
                    .font_weight(font_weight)
                    .color(color),
            );
        }

        p
    }
}
