//! Completion framework with pluggable sources.
//!
//! Provides a trait-based system where multiple [`CompletionSource`]s contribute
//! candidates, coordinated by [`CompletionCoordinator`]. Includes a built-in
//! [`PathCompletionSource`] for filesystem path completion.

use std::path::Path;

/// A single completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The string to insert into the line.
    pub value: String,
    /// Display label (may differ from value, e.g. trailing `/` for dirs).
    pub display: String,
    /// What kind of thing this candidate represents.
    pub kind: CompletionKind,
}

/// The kind of completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// An executable command.
    Command,
    /// An environment variable.
    Variable,
    /// A history entry.
    History,
    /// An extension-defined kind.
    Custom(String),
}

/// A source of completion candidates.
///
/// Implement this trait to plug custom completion logic into the coordinator.
pub trait CompletionSource: Send {
    /// Produce candidates for the current line at the given cursor position.
    fn complete(&self, line: &str, cursor: usize) -> Vec<Candidate>;
}

/// Coordinates multiple completion sources, merging their results.
pub struct CompletionCoordinator {
    sources: Vec<Box<dyn CompletionSource>>,
}

impl CompletionCoordinator {
    /// Create an empty coordinator.
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// Add a completion source.
    pub fn add_source(&mut self, source: Box<dyn CompletionSource>) {
        self.sources.push(source);
    }

    /// Run all sources and return the merged candidate list.
    pub fn complete(&self, line: &str, cursor: usize) -> Vec<Candidate> {
        let mut all = Vec::new();
        for source in &self.sources {
            all.extend(source.complete(line, cursor));
        }
        all
    }
}

impl Default for CompletionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

/// Find the longest common prefix among a list of candidates.
///
/// Returns the empty string if the list is empty or there is no shared prefix.
pub fn common_prefix(candidates: &[Candidate]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let first = &candidates[0].value;
    let mut prefix_len = first.chars().count();
    for c in &candidates[1..] {
        prefix_len = first
            .chars()
            .zip(c.value.chars())
            .take_while(|(a, b)| a == b)
            .count()
            .min(prefix_len);
        if prefix_len == 0 {
            break;
        }
    }
    first.chars().take(prefix_len).collect()
}

/// Built-in filesystem path completion source.
///
/// Extracts the word being typed (from cursor backward to whitespace),
/// treats it as a partial path, and globs for matches.
pub struct PathCompletionSource;

impl CompletionSource for PathCompletionSource {
    fn complete(&self, line: &str, cursor: usize) -> Vec<Candidate> {
        let before = &line[..cursor.min(line.len())];
        // Find the word being typed by scanning backward to whitespace.
        let word_start = before
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);
        let partial = &before[word_start..];
        if partial.is_empty() {
            return Vec::new();
        }

        complete_path(partial)
    }
}

/// Complete a partial path against the filesystem.
fn complete_path(partial: &str) -> Vec<Candidate> {
    let path = Path::new(partial);
    let (dir, stem) = if partial.ends_with('/') || partial.ends_with('\\') {
        (path, "")
    } else {
        let parent = path.parent().unwrap_or(Path::new("."));
        let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        (parent, stem)
    };

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut candidates: Vec<Candidate> = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(stem))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let full = if dir == Path::new(".") {
                name.clone()
            } else {
                format!("{}/{}", dir.display(), name)
            };
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                Candidate {
                    display: format!("{}/", full),
                    value: full,
                    kind: CompletionKind::Directory,
                }
            } else {
                Candidate {
                    display: full.clone(),
                    value: full,
                    kind: CompletionKind::File,
                }
            }
        })
        .collect();

    candidates.sort_by(|a, b| a.value.cmp(&b.value));
    candidates
}
