/// Colors used across the application shell and document views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub background: u32,
    pub surface: u32,
    pub selection: u32,
    pub border: u32,
    pub text: u32,
    pub text_strong: u32,
    pub text_muted: u32,
    pub accent: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
    pub violet: u32,
    pub cyan: u32,
    pub code_background: u32,
    pub pdf_background: u32,
}

pub const SOLARIZED_LIGHT: Theme = Theme {
    id: "solarized-light",
    name: "Solarized Light",
    description: "Warm, low-contrast light theme",
    background: 0xFDF6E3,
    surface: 0xEEE8D5,
    selection: 0xE4DCC5,
    border: 0xD3CBB8,
    text: 0x657B83,
    text_strong: 0x073642,
    text_muted: 0x93A1A1,
    accent: 0x268BD2,
    success: 0x859900,
    warning: 0xCB4B16,
    danger: 0xDC322F,
    violet: 0x6C71C4,
    cyan: 0x2AA198,
    code_background: 0xEEE8D5,
    pdf_background: 0xE8E4DA,
};

pub const SOLARIZED_DARK: Theme = Theme {
    id: "solarized-dark",
    name: "Solarized Dark",
    description: "Low-contrast dark companion to Solarized Light",
    background: 0x002B36,
    surface: 0x073642,
    selection: 0x164955,
    border: 0x1B4D57,
    text: 0x839496,
    text_strong: 0xFDF6E3,
    text_muted: 0x586E75,
    accent: 0x268BD2,
    success: 0x859900,
    warning: 0xCB4B16,
    danger: 0xDC322F,
    violet: 0x6C71C4,
    cyan: 0x2AA198,
    code_background: 0x073642,
    pdf_background: 0x001F27,
};

pub const GRUVBOX_DARK: Theme = Theme {
    id: "gruvbox-dark",
    name: "Gruvbox Dark",
    description: "Warm retro dark theme",
    background: 0x282828,
    surface: 0x3C3836,
    selection: 0x504945,
    border: 0x665C54,
    text: 0xEBDBB2,
    text_strong: 0xFBF1C7,
    text_muted: 0xA89984,
    accent: 0x83A598,
    success: 0xB8BB26,
    warning: 0xFE8019,
    danger: 0xFB4934,
    violet: 0xD3869B,
    cyan: 0x8EC07C,
    code_background: 0x3C3836,
    pdf_background: 0x1D2021,
};

pub const NORD: Theme = Theme {
    id: "nord",
    name: "Nord",
    description: "Cool arctic dark theme",
    background: 0x2E3440,
    surface: 0x3B4252,
    selection: 0x434C5E,
    border: 0x4C566A,
    text: 0xD8DEE9,
    text_strong: 0xECEFF4,
    text_muted: 0x7B88A1,
    accent: 0x88C0D0,
    success: 0xA3BE8C,
    warning: 0xD08770,
    danger: 0xBF616A,
    violet: 0xB48EAD,
    cyan: 0x8FBCBB,
    code_background: 0x3B4252,
    pdf_background: 0x242933,
};

pub const THEMES: &[Theme] = &[SOLARIZED_LIGHT, SOLARIZED_DARK, GRUVBOX_DARK, NORD];

pub fn by_id(id: &str) -> Option<Theme> {
    THEMES.iter().copied().find(|theme| theme.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_are_unique_and_resolvable() {
        for (index, theme) in THEMES.iter().enumerate() {
            assert_eq!(by_id(theme.id), Some(*theme));
            assert!(!THEMES[..index].iter().any(|other| other.id == theme.id));
        }
    }
}
