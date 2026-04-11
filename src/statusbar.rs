use freya::prelude::*;

use crate::state::AppState;

const STATUSBAR_HEIGHT: f32 = 32.0;
const STATUSBAR_BG: (u8, u8, u8) = (235, 235, 240);
const STATUSBAR_TEXT: (u8, u8, u8) = (80, 80, 90);
const STATUSBAR_ACCENT: (u8, u8, u8) = (40, 100, 200);
const DROPDOWN_BG: (u8, u8, u8) = (245, 245, 250);

/// Status bar at the bottom of the window.
#[derive(PartialEq)]
pub struct StatusBar {
    pub app_state: State<AppState>,
}

impl Component for StatusBar {
    fn render(&self) -> impl IntoElement {
        let app_state = self.app_state;
        let vault_name = app_state.read().vault_name();
        let note_title = app_state.read().current_title();
        let dirty = app_state.read().dirty;

        let dirty_indicator = if dirty { " ●" } else { "" };

        rect()
            .width(Size::fill())
            .height(Size::px(STATUSBAR_HEIGHT))
            .background(STATUSBAR_BG)
            .direction(Direction::Horizontal)
            .cross_align(Alignment::Center)
            .padding((0., 12., 0., 12.))
            .child(
                VaultSwitcher {
                    app_state: self.app_state,
                    vault_name: vault_name.clone(),
                },
            )
            .child(
                // Spacer
                rect().width(Size::px(16.)).height(Size::px(1.)),
            )
            .child(
                // Note title + dirty indicator
                label()
                    .text(format!("{}{}", note_title, dirty_indicator))
                    .font_size(13.)
                    .color(STATUSBAR_TEXT),
            )
    }
}

/// Vault name button that shows a dropdown of available vaults.
#[derive(PartialEq)]
struct VaultSwitcher {
    app_state: State<AppState>,
    vault_name: String,
}

impl Component for VaultSwitcher {
    fn render(&self) -> impl IntoElement {
        let mut app_state = self.app_state;
        let show_dropdown = use_state(|| false);
        let vault_name = self.vault_name.clone();

        let toggle = {
            let mut show_dropdown = show_dropdown;
            move |_: Event<PointerEventData>| {
                let current = *show_dropdown.read();
                show_dropdown.set(!current);
            }
        };

        let is_open = *show_dropdown.read();
        let registry = app_state.read().registry.clone();
        let vault_paths = registry.vault_paths();

        // Build dropdown items (always built, shown/hidden via size)
        let mut dropdown_inner = rect().width(Size::fill());

        for vault_path in &vault_paths {
            let name = vault_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("vault")
                .to_string();

            let path = vault_path.clone();
            let mut show_dropdown = show_dropdown;
            let on_click = move |_: Event<PointerEventData>| {
                let path = path.clone();
                if let Err(e) = app_state.write().open_vault(path) {
                    eprintln!("failed to open vault: {}", e);
                }
                show_dropdown.set(false);
            };

            dropdown_inner = dropdown_inner.child(
                rect()
                    .width(Size::fill())
                    .padding((6., 8., 6., 8.))
                    .corner_radius(4.)
                    .on_pointer_press(on_click)
                    .child(label().text(name).font_size(13.).color(STATUSBAR_TEXT)),
            );
        }

        // "Open vault..." option
        let mut show_dd = show_dropdown;
        let on_open_vault = move |_: Event<PointerEventData>| {
            eprintln!("TODO: open folder picker dialog");
            show_dd.set(false);
        };

        dropdown_inner = dropdown_inner.child(
            rect()
                .width(Size::fill())
                .padding((6., 8., 6., 8.))
                .corner_radius(4.)
                .on_pointer_press(on_open_vault)
                .child(
                    label()
                        .text("+ Open vault...")
                        .font_size(13.)
                        .color(STATUSBAR_ACCENT),
                ),
        );

        // Dropdown container: always in tree, visibility controlled by size
        let dropdown = if is_open {
            rect()
                .width(Size::px(250.))
                .background(DROPDOWN_BG)
                .corner_radius(6.)
                .padding(4.)
                .position(Position::new_global().bottom(STATUSBAR_HEIGHT + 4.).left(12.))
                .child(dropdown_inner)
        } else {
            rect()
                .width(Size::px(0.))
                .height(Size::px(0.))
                .overflow(Overflow::Clip)
                .position(Position::new_global())
                .child(dropdown_inner)
        };

        rect()
            .child(
                // Vault name button
                rect()
                    .padding((4., 8., 4., 8.))
                    .corner_radius(4.)
                    .background(DROPDOWN_BG)
                    .on_pointer_press(toggle)
                    .child(
                        label()
                            .text(format!("⌂ {}", vault_name))
                            .font_size(13.)
                            .color(STATUSBAR_ACCENT),
                    ),
            )
            .child(dropdown)
    }
}
