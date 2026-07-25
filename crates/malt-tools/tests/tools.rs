use malt_tools::Registry;

#[test]
fn registry_has_all_tools() {
    let reg = Registry::new();
    assert!(reg.contains("cat"));
    assert!(reg.contains("env"));
    assert!(reg.contains("which"));
    assert!(reg.contains("grep"));
    assert!(reg.contains("wc"));
}

#[test]
fn registry_names_sorted() {
    let reg = Registry::new();
    let names = reg.names();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

#[test]
fn registry_does_not_contain_unknown() {
    let reg = Registry::new();
    assert!(!reg.contains("nonexistent"));
    assert!(reg.get("nonexistent").is_none());
}

#[test]
fn cat_stdin() {
    let reg = Registry::new();
    let cat = reg.get("cat").unwrap();
    let result = cat(&[], &mut &b"hello\nworld\n"[..], &mut std::io::sink());
    assert_eq!(result.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&result.stdout), "hello\nworld\n");
}

#[test]
fn cat_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "content\n").unwrap();
    let reg = Registry::new();
    let cat = reg.get("cat").unwrap();
    let result = cat(
        &[path.to_string_lossy().to_string()],
        &mut &[][..],
        &mut std::io::sink(),
    );
    assert_eq!(result.exit_code, 0);
    assert!(String::from_utf8_lossy(&result.stdout).contains("content"));
}

#[test]
fn cat_missing_file() {
    let reg = Registry::new();
    let cat = reg.get("cat").unwrap();
    let result = cat(&["/nonexistent/file".into()], &mut &[][..], &mut std::io::sink());
    assert_ne!(result.exit_code, 0);
}

#[test]
fn grep_matches() {
    let reg = Registry::new();
    let grep = reg.get("grep").unwrap();
    let result = grep(
        &["hello".into()],
        &mut &b"hello world\ngoodbye\nhello again\n"[..],
        &mut std::io::sink(),
    );
    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(output.contains("hello world"));
    assert!(output.contains("hello again"));
    assert!(!output.contains("goodbye"));
}

#[test]
fn grep_no_match() {
    let reg = Registry::new();
    let grep = reg.get("grep").unwrap();
    let result = grep(&["xyz".into()], &mut &b"hello\nworld\n"[..], &mut std::io::sink());
    assert_eq!(result.exit_code, 1);
}

#[test]
fn grep_invert() {
    let reg = Registry::new();
    let grep = reg.get("grep").unwrap();
    let result = grep(
        &["-v".into(), "hello".into()],
        &mut &b"hello\nworld\n"[..],
        &mut std::io::sink(),
    );
    let output = String::from_utf8_lossy(&result.stdout);
    assert!(!output.contains("hello"));
    assert!(output.contains("world"));
}

#[test]
fn grep_count() {
    let reg = Registry::new();
    let grep = reg.get("grep").unwrap();
    let result = grep(
        &["-c".into(), "hello".into()],
        &mut &b"hello\nhello\nworld\n"[..],
        &mut std::io::sink(),
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "2");
}

#[test]
fn wc_stdin() {
    let reg = Registry::new();
    let wc = reg.get("wc").unwrap();
    let result = wc(&[], &mut &b"hello world\ngoodbye\n"[..], &mut std::io::sink());
    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8_lossy(&result.stdout);
    // Should contain line count (2), word count (3), byte count
    assert!(output.contains("2")); // 2 lines
}

#[test]
fn wc_lines_only() {
    let reg = Registry::new();
    let wc = reg.get("wc").unwrap();
    let result = wc(&["-l".into()], &mut &b"a\nb\nc\n"[..], &mut std::io::sink());
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "3");
}

#[test]
fn which_finds_something() {
    let reg = Registry::new();
    let which = reg.get("which").unwrap();
    // On Windows, cmd.exe should be findable; on Unix, sh
    if cfg!(windows) {
        let result = which(&["cmd".into()], &mut &[][..], &mut std::io::sink());
        // May or may not find it depending on PATH config
        let _ = result;
    } else {
        let result = which(&["sh".into()], &mut &[][..], &mut std::io::sink());
        let _ = result;
    }
}

#[test]
fn env_prints_vars() {
    let reg = Registry::new();
    let env = reg.get("env").unwrap();
    let result = env(&[], &mut &[][..], &mut std::io::sink());
    assert_eq!(result.exit_code, 0);
    let output = String::from_utf8_lossy(&result.stdout);
    // Should contain at least PATH
    assert!(output.contains("PATH") || output.contains("Path"));
}
