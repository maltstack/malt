//! History ring buffer, search, bang expansion, and file persistence.
//!
//! Provides an in-memory ring buffer of command history with consecutive
//! dedup, prefix/substring search, bash-compatible bang expansion, and
//! simple one-line-per-entry file persistence.

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::Path;

/// A single history entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The command line text.
    pub line: String,
    /// Unix timestamp (seconds) when the entry was recorded.
    pub timestamp: u64,
}

impl HistoryEntry {
    /// Create an entry with timestamp 0.
    pub fn new(line: impl Into<String>) -> Self {
        Self {
            line: line.into(),
            timestamp: 0,
        }
    }
}

/// Bounded ring-buffer history with search and bang expansion.
pub struct History {
    entries: VecDeque<HistoryEntry>,
    max_entries: usize,
}

impl History {
    /// Create a new history with the given capacity (default recommendation: 10 000).
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries,
        }
    }

    /// Add a line to history.
    ///
    /// - Skips blank/whitespace-only lines.
    /// - Skips if identical to the most recent entry (consecutive dedup).
    /// - Evicts the oldest entry when at capacity.
    pub fn add(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        // Consecutive dedup
        if self.entries.back().map(|e| e.line.as_str()) == Some(line.as_str()) {
            return;
        }
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(HistoryEntry {
            line,
            timestamp: 0,
        });
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get an entry by index (0 = oldest).
    pub fn get(&self, index: usize) -> Option<&HistoryEntry> {
        self.entries.get(index)
    }

    /// Search backward from `from` (inclusive) for an entry whose line starts
    /// with `prefix`. Returns `(index, &line)`.
    pub fn search_prefix(&self, prefix: &str, from: usize) -> Option<(usize, &str)> {
        if self.entries.is_empty() {
            return None;
        }
        let start = from.min(self.entries.len() - 1);
        (0..=start)
            .rev()
            .find(|&i| self.entries[i].line.starts_with(prefix))
            .map(|i| (i, self.entries[i].line.as_str()))
    }

    /// Search backward from `from` (inclusive) for an entry whose line
    /// contains `substring`. Returns `(index, &line)`.
    pub fn search_backward(&self, substring: &str, from: usize) -> Option<(usize, &str)> {
        if self.entries.is_empty() {
            return None;
        }
        let start = from.min(self.entries.len() - 1);
        (0..=start)
            .rev()
            .find(|&i| self.entries[i].line.contains(substring))
            .map(|i| (i, self.entries[i].line.as_str()))
    }

    /// Expand bang expressions in `input`.
    ///
    /// Supported forms:
    /// - `!!` — last command
    /// - `!N` — Nth entry (1-indexed from oldest)
    /// - `!-N` — Nth from end
    /// - `!str` — most recent entry starting with `str`
    /// - `^old^new` — substitute first occurrence of `old` with `new` in last command
    pub fn expand_bangs(&self, input: &str) -> Result<String, HistoryError> {
        // Handle ^old^new as a standalone form
        if let Some(rest) = input.strip_prefix('^') {
            return self.expand_caret_subst(rest);
        }

        let mut result = String::with_capacity(input.len());
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '!' && i + 1 < chars.len() {
                // Skip bangs inside single quotes
                if is_in_single_quotes(input, i) {
                    result.push(chars[i]);
                    i += 1;
                    continue;
                }

                let next = chars[i + 1];

                if next == '!' {
                    // !! — last command
                    let last = self
                        .entries
                        .back()
                        .ok_or_else(|| HistoryError::Empty)?;
                    result.push_str(&last.line);
                    i += 2;
                } else if next == '-' || next.is_ascii_digit() {
                    // !N or !-N
                    let (line, consumed) = self.resolve_numeric(&chars, i + 1)?;
                    result.push_str(line);
                    i += 1 + consumed;
                } else if next == ' ' || next == '\t' || next == '=' {
                    // Not a bang expansion
                    result.push(chars[i]);
                    i += 1;
                } else {
                    // !str — prefix search
                    let (line, consumed) = self.resolve_prefix(&chars, i + 1)?;
                    result.push_str(line);
                    i += 1 + consumed;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        Ok(result)
    }

    /// Load history from a file (one command per line).
    pub fn load(&mut self, path: &Path) -> Result<(), std::io::Error> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            if !line.is_empty() {
                self.add(line);
            }
        }
        Ok(())
    }

    /// Save history to a file (one command per line).
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut file = std::fs::File::create(path)?;
        for entry in &self.entries {
            writeln!(file, "{}", entry.line)?;
        }
        file.sync_all()?;
        Ok(())
    }

    // --- private helpers ---

    fn expand_caret_subst(&self, rest: &str) -> Result<String, HistoryError> {
        let last = self
            .entries
            .back()
            .ok_or(HistoryError::Empty)?;

        let parts: Vec<&str> = rest.splitn(2, '^').collect();
        if parts.is_empty() {
            return Err(HistoryError::NotFound("^: missing substitution".into()));
        }

        let old = parts[0];
        let new = if parts.len() > 1 { parts[1] } else { "" };

        if !last.line.contains(old) {
            return Err(HistoryError::NotFound(format!(
                "^{old}^{new}: substitution failed"
            )));
        }

        Ok(last.line.replacen(old, new, 1))
    }

    fn resolve_numeric<'a>(
        &'a self,
        chars: &[char],
        start: usize,
    ) -> Result<(&'a str, usize), HistoryError> {
        let mut end = start;
        if end < chars.len() && chars[end] == '-' {
            end += 1;
        }
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }

        let num_str: String = chars[start..end].iter().collect();
        let consumed = end - start;

        let n: i64 = num_str
            .parse()
            .map_err(|_| HistoryError::NotFound(format!("!{num_str}")))?;

        let idx = if n < 0 {
            let abs = n.unsigned_abs() as usize;
            if abs > self.entries.len() {
                return Err(HistoryError::NotFound(format!("!{n}")));
            }
            self.entries.len() - abs
        } else {
            // 1-based index
            let idx = (n as usize).saturating_sub(1);
            if idx >= self.entries.len() {
                return Err(HistoryError::NotFound(format!("!{n}")));
            }
            idx
        };

        Ok((self.entries[idx].line.as_str(), consumed))
    }

    fn resolve_prefix<'a>(
        &'a self,
        chars: &[char],
        start: usize,
    ) -> Result<(&'a str, usize), HistoryError> {
        let mut end = start;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        let prefix: String = chars[start..end].iter().collect();
        let consumed = end - start;

        // Search backward for most recent match
        for entry in self.entries.iter().rev() {
            if entry.line.starts_with(&prefix) {
                return Ok((entry.line.as_str(), consumed));
            }
        }

        Err(HistoryError::NotFound(format!("!{prefix}")))
    }
}

/// Check if byte position `pos` in `input` is inside single quotes.
fn is_in_single_quotes(input: &str, pos: usize) -> bool {
    let mut in_single = false;
    for (i, ch) in input.char_indices() {
        if i == pos {
            return in_single;
        }
        if ch == '\'' {
            in_single = !in_single;
        }
    }
    false
}

/// Errors from history operations.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// A bang expansion referenced a nonexistent entry.
    #[error("no such history entry: {0}")]
    NotFound(String),
    /// History is empty (e.g. `!!` with no entries).
    #[error("history is empty")]
    Empty,
}
