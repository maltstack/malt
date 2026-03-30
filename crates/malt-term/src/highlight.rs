//! Syntax highlighting trait and supporting types.
//!
//! Defines the [`Highlighter`] trait and the [`Span`]/[`Style`]/[`Color`] types
//! that concrete highlighters (e.g. in `mash`) produce. No default implementation
//! is provided here — shell-specific highlighting belongs in the shell crate.

/// A text style applied to a highlighted span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    /// Foreground color, or `None` for the terminal default.
    pub fg: Option<Color>,
    /// Whether the text is bold.
    pub bold: bool,
}

impl Style {
    /// Plain style — default foreground, not bold.
    pub fn plain() -> Self {
        Self {
            fg: None,
            bold: false,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::plain()
    }
}

/// Named terminal colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Color {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Default,
}

/// A styled region within a line, identified by byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Start byte offset (inclusive).
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
    /// The style to apply.
    pub style: Style,
}

/// Trait for syntax highlighters.
///
/// Implementations analyse a line of input and return a list of [`Span`]s
/// describing how it should be styled. The spans must be non-overlapping and
/// sorted by `start`.
pub trait Highlighter: Send {
    /// Highlight `line` and return styled spans.
    fn highlight(&self, line: &str) -> Vec<Span>;
}
