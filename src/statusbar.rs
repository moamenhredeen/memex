use freya::prelude::*;

use crate::state::AppState;

const STATUSBAR_HEIGHT: f32 = 32.0;
const STATUSBAR_BG: (u8, u8, u8) = (24, 24, 38);
const STATUSBAR_TEXT: (u8, u8, u8) = (180, 180, 200);
const STATUSBAR_ACCENT: (u8, u8, u8) = (137, 180, 250);
const DROPDOWN_BG: (u8, u8, u8) = (35, 35, 55);

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
        let mut show_dropdown = use_state(|| false);
        let vault_name = self.vault_name.clone();

        let toggle = {
            let mut show_dropdown = show_dropdown;
            move |_: Event<PointerEventData>| {
                let current = *show_dropdown.read();
                show_dropdown.set(!current);
            }
        };

        let mut container = rect()
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
            );

        if *show_dropdown.read() {
            let registry = app_state.read().registry.clone();
            let vault_paths = registry.vault_paths();

            container = container.child(
                VaultDropdown {
                    app_state,
                    vault_paths,
                    show_dropdown,
                },
            );
        }

        container
    }
}

/// The dropdown menu listing all registered vaults.
#[derive(PartialEq)]
struct VaultDropdown {
    app_state: State<AppState>,
    vault_paths: Vec<std::path::PathBuf>,
    show_dropdown: State<bool>,
}

impl Component for VaultDropdown {
    fn render(&self) -> impl IntoElement {
        let mut app_state = self.app_state;
        let mut show_dropdown = self.show_dropdown;

        let mut dropdown = rect()
            .width(Size::px(250.))
            .background(DROPDOWN_BG)
            .corner_radius(6.)
            .padding(4.)
            .position(Position::new_absolute().bottom(STATUSBAR_HEIGHT + 4.));

        for vault_path in &self.vault_paths {
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

            dropdown = dropdown.child(
                rect()
                    .width(Size::fill())
                    .padding((6., 8., 6., 8.))
                    .corner_radius(4.)
                    .on_pointer_press(on_click)
                    .child(label().text(name).font_size(13.).color(STATUSBAR_TEXT)),
            );
        }

        // "Open vault..." option
        let on_open_vault = move |_: Event<PointerEventData>| {
            eprintln!("TODO: open folder picker dialog");
            show_dropdown.set(false);
        };

        dropdown = dropdown.child(
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

        dropdown
    }
}
