use malt_protocol::common::ResolvedStyle;

const DEFAULT_FG: (u8, u8, u8) = (204, 204, 204);
const DEFAULT_BG: (u8, u8, u8) = (0, 0, 0);

/// Resolves theme tokens to concrete RGB values.
/// Currently a stub returning default colors.
#[derive(Debug, Clone)]
pub struct ThemeResolver;

impl ThemeResolver {
    pub fn new() -> Self {
        Self
    }

    pub fn default_style(&self) -> ResolvedStyle {
        ResolvedStyle {
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
            italic: false,
            underline: false,
            dim: false,
            strikethrough: false,
            reverse: false,
            blink: false,
            token_name: None,
            _unknown: Vec::new(),
        }
    }

    pub fn resolve(&self, style: &ResolvedStyle) -> ResolvedStyle {
        style.clone()
    }
}

impl Default for ThemeResolver {
    fn default() -> Self {
        Self::new()
    }
}
