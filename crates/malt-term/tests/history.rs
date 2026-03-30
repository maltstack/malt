use malt_term::history::{History, HistoryError};

#[test]
fn add_and_retrieve() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("ls -la".into());
    assert_eq!(h.len(), 2);
    assert_eq!(h.get(0).unwrap().line, "echo hello");
    assert_eq!(h.get(1).unwrap().line, "ls -la");
}

#[test]
fn dedup_consecutive() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("echo hello".into());
    assert_eq!(h.len(), 1);
}

#[test]
fn ring_buffer_eviction() {
    let mut h = History::new(3);
    for i in 0..5 {
        h.add(format!("cmd {i}"));
    }
    assert_eq!(h.len(), 3);
    // Oldest surviving is "cmd 2"
    assert_eq!(h.get(0).unwrap().line, "cmd 2");
    assert_eq!(h.get(1).unwrap().line, "cmd 3");
    assert_eq!(h.get(2).unwrap().line, "cmd 4");
}

#[test]
fn search_prefix_finds_match() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("ls -la".into());
    h.add("git commit -m fix".into());
    h.add("git push".into());

    let result = h.search_prefix("git", h.len() - 1);
    assert!(result.is_some());
    let (idx, line) = result.unwrap();
    assert_eq!(idx, 3);
    assert_eq!(line, "git push");

    // Search from before the last match
    let result = h.search_prefix("git", 2);
    assert!(result.is_some());
    let (idx, line) = result.unwrap();
    assert_eq!(idx, 2);
    assert_eq!(line, "git commit -m fix");
}

#[test]
fn search_backward_substring() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("git commit -m fix".into());
    h.add("ls -la".into());
    h.add("git push".into());

    let result = h.search_backward("commit", h.len() - 1);
    assert!(result.is_some());
    let (idx, line) = result.unwrap();
    assert_eq!(idx, 1);
    assert_eq!(line, "git commit -m fix");
}

#[test]
fn expand_bang_bang() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("ls -la".into());

    let expanded = h.expand_bangs("sudo !!").unwrap();
    assert_eq!(expanded, "sudo ls -la");
}

#[test]
fn expand_bang_n() {
    let mut h = History::new(100);
    h.add("echo hello".into()); // 1
    h.add("ls -la".into());     // 2
    h.add("pwd".into());        // 3

    // !1 → first entry (1-based)
    let expanded = h.expand_bangs("!1").unwrap();
    assert_eq!(expanded, "echo hello");

    // !2 → second entry
    let expanded = h.expand_bangs("!2").unwrap();
    assert_eq!(expanded, "ls -la");
}

#[test]
fn expand_bang_prefix() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("git commit -m fix".into());
    h.add("ls -la".into());
    h.add("git push".into());

    // !git → most recent starting with "git"
    let expanded = h.expand_bangs("!git").unwrap();
    assert_eq!(expanded, "git push");
}

#[test]
fn expand_caret_substitution() {
    let mut h = History::new(100);
    h.add("echo hello".into());

    let expanded = h.expand_bangs("^hello^world").unwrap();
    assert_eq!(expanded, "echo world");
}

#[test]
fn expand_bang_negative_index() {
    let mut h = History::new(100);
    h.add("echo hello".into());
    h.add("ls -la".into());
    h.add("pwd".into());

    // !-1 → last command
    let expanded = h.expand_bangs("!-1").unwrap();
    assert_eq!(expanded, "pwd");

    // !-2 → second to last
    let expanded = h.expand_bangs("!-2").unwrap();
    assert_eq!(expanded, "ls -la");
}

#[test]
fn expand_bang_empty_history_errors() {
    let h = History::new(100);
    let err = h.expand_bangs("!!").unwrap_err();
    assert!(matches!(err, HistoryError::Empty));
}

#[test]
fn load_save_roundtrip() {
    let dir = std::env::temp_dir().join("malt_term_history_test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("history.txt");

    // Save
    let mut h1 = History::new(100);
    h1.add("echo hello".into());
    h1.add("ls -la".into());
    h1.add("pwd".into());
    h1.save(&path).unwrap();

    // Load into fresh history
    let mut h2 = History::new(100);
    h2.load(&path).unwrap();

    assert_eq!(h2.len(), 3);
    assert_eq!(h2.get(0).unwrap().line, "echo hello");
    assert_eq!(h2.get(1).unwrap().line, "ls -la");
    assert_eq!(h2.get(2).unwrap().line, "pwd");

    let _ = std::fs::remove_dir_all(&dir);
}
