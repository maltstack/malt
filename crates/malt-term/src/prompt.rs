//! Prompt template renderer.
//!
//! Expands bash-style backslash escape sequences in a prompt template string.
//! The renderer is a pure function over the template and a small set of
//! context values — it performs no I/O.

/// Renders prompt strings from a template with bash-style escape sequences.
///
/// Supported escapes:
/// - `\w` — last component of `cwd`
/// - `\W` — full `cwd` path
/// - `\u` — username
/// - `\h` — hostname
/// - `\$` — `$` for normal users, `#` if `user == "root"`
/// - `\?` — last exit code (only rendered if non-zero)
/// - `\\` — literal backslash
///
/// Default template: `"\u@\h:\w\$ "`
pub struct PromptRenderer {
    template: String,
}

impl PromptRenderer {
    /// Create a renderer with the given template.
    pub fn new(template: &str) -> Self {
        Self {
            template: template.to_string(),
        }
    }

    /// Render the prompt, substituting escape sequences.
    pub fn render(&self, cwd: &str, user: &str, hostname: &str, exit_code: i32) -> String {
        render_template(&self.template, cwd, user, hostname, exit_code)
    }
}

impl Default for PromptRenderer {
    fn default() -> Self {
        Self::new(r"\u@\h:\w\$ ")
    }
}

/// Pure render function — expand escape sequences in `template`.
fn render_template(
    template: &str,
    cwd: &str,
    user: &str,
    hostname: &str,
    exit_code: i32,
) -> String {
    let mut result = String::with_capacity(template.len() * 2);
    let chars: Vec<char> = template.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '\\' && i + 1 < len {
            match chars[i + 1] {
                'w' => {
                    // Last component of cwd
                    let base = cwd
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(cwd);
                    result.push_str(base);
                    i += 2;
                }
                'W' => {
                    // Full cwd
                    result.push_str(cwd);
                    i += 2;
                }
                'u' => {
                    result.push_str(user);
                    i += 2;
                }
                'h' => {
                    result.push_str(hostname);
                    i += 2;
                }
                '$' => {
                    result.push(if user == "root" { '#' } else { '$' });
                    i += 2;
                }
                '?' => {
                    if exit_code != 0 {
                        result.push_str(&exit_code.to_string());
                    }
                    i += 2;
                }
                '\\' => {
                    result.push('\\');
                    i += 2;
                }
                other => {
                    // Unknown escape — pass through literally
                    result.push('\\');
                    result.push(other);
                    i += 2;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}
