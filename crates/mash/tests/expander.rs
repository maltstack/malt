use mash::env::Env;
use mash::expander::*;
use std::sync::Mutex;

static CWD_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn tilde_expands_to_home() {
    let mut env = Env::empty();
    env.set("HOME", mash::env::Variable::string("/home/user"))
        .unwrap();
    let result = expand_word_nosplit("~/bin", &mut env).unwrap();
    assert_eq!(result, "/home/user/bin");
}

#[test]
fn tilde_plus_expands_to_pwd() {
    let mut env = Env::empty();
    env.set("PWD", mash::env::Variable::string("/tmp")).unwrap();
    let result = expand_word_nosplit("~+/foo", &mut env).unwrap();
    assert_eq!(result, "/tmp/foo");
}

#[test]
fn tilde_minus_expands_to_oldpwd() {
    let mut env = Env::empty();
    env.set("OLDPWD", mash::env::Variable::string("/var"))
        .unwrap();
    let result = expand_word_nosplit("~-", &mut env).unwrap();
    assert_eq!(result, "/var");
}

#[test]
fn simple_var_expansion() {
    let mut env = Env::empty();
    env.set("FOO", mash::env::Variable::string("bar")).unwrap();
    let result = expand_word_nosplit("$FOO", &mut env).unwrap();
    assert_eq!(result, "bar");
}

#[test]
fn simple_var_in_text() {
    let mut env = Env::empty();
    env.set("NAME", mash::env::Variable::string("world"))
        .unwrap();
    let result = expand_word_nosplit("hello $NAME!", &mut env).unwrap();
    assert_eq!(result, "hello world!");
}

#[test]
fn unset_var_expands_to_empty() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$NONEXISTENT", &mut env).unwrap();
    assert_eq!(result, "");
}

#[test]
fn nounset_errors_on_unset() {
    let mut env = Env::empty();
    env.options_mut().nounset = true;
    let result = expand_word_nosplit("$NONEXISTENT", &mut env);
    assert!(result.is_err());
}

#[test]
fn nounset_errors_on_length_of_unset_parameter() {
    let mut env = Env::empty();
    env.options_mut().nounset = true;
    let result = expand_word_nosplit("${#NONEXISTENT}", &mut env);
    assert!(result.is_err());
}

#[test]
fn nounset_errors_on_plain_identifier_in_arithmetic() {
    let mut env = Env::empty();
    env.options_mut().nounset = true;
    let result = expand_word_nosplit("$((NONEXISTENT + 1))", &mut env);
    assert!(result.is_err());
}

#[test]
fn special_var_question_mark() {
    let mut env = Env::empty();
    env.set_exit_code(42);
    let result = expand_word_nosplit("$?", &mut env).unwrap();
    assert_eq!(result, "42");
}

#[test]
fn special_var_dollar() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$$", &mut env).unwrap();
    assert!(!result.is_empty()); // PID
}

#[test]
fn single_quotes_prevent_expansion() {
    let mut env = Env::empty();
    env.set("FOO", mash::env::Variable::string("bar")).unwrap();
    let result = expand_word_nosplit("'$FOO'", &mut env).unwrap();
    assert_eq!(result, "$FOO");
}

#[test]
fn double_quotes_allow_expansion() {
    let mut env = Env::empty();
    env.set("FOO", mash::env::Variable::string("bar")).unwrap();
    let result = expand_word_nosplit("\"$FOO\"", &mut env).unwrap();
    assert_eq!(result, "bar");
}

#[test]
fn backslash_escape() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("hello\\$world", &mut env).unwrap();
    assert_eq!(result, "hello$world");
}

#[test]
fn double_quoted_backslash_newline_is_removed_in_words() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("\"alpha \\\nomega\"", &mut env).unwrap();
    assert_eq!(result, "alpha omega");
}

#[test]
fn double_quoted_backslash_newline_is_removed_in_assignments() {
    let mut env = Env::empty();
    let result = expand_assignment_word_nosplit("alpha \\\nomega", &mut env).unwrap();
    assert_eq!(result, "alpha omega");
}

#[test]
fn case_pattern_expansion_preserves_backslash_newline_match() {
    let mut env = Env::empty();
    let word = expand_word_nosplit("'foo\\\nbar'", &mut env).unwrap();
    let pattern = expand_word_for_case_pattern("foo\\\\\"\n\"bar", &mut env).unwrap();
    assert!(
        shell_pattern_match(&word, &pattern),
        "word={word:?} pattern={pattern:?}"
    );
}

#[test]
fn heredoc_expansion_removes_backslash_newline_line_continuation() {
    let mut env = Env::empty();
    let body = "\"line\".\\${PATH}.\\'three\\'\\\\x\\\nline four\n";
    let expanded = expand_heredoc_body(body, &mut env).unwrap();
    assert_eq!(expanded, "\"line\".${PATH}.\\'three\\'\\xline four\n");
}

#[test]
fn command_sub_captures_output() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$(echo hello)", &mut env).unwrap();
    assert_eq!(result, "hello");
}

#[test]
fn bare_text_unchanged() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("hello world", &mut env).unwrap();
    assert_eq!(result, "hello world");
}

// ── Parameter expansion tests ──

#[test]
fn brace_simple() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_default_unset() {
    let mut env = Env::empty();
    assert_eq!(
        expand_word_nosplit("${X:-fallback}", &mut env).unwrap(),
        "fallback"
    );
}

#[test]
fn brace_default_set() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    assert_eq!(
        expand_word_nosplit("${X:-fallback}", &mut env).unwrap(),
        "val"
    );
}

#[test]
fn brace_default_empty_with_colon() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("")).unwrap();
    assert_eq!(
        expand_word_nosplit("${X:-fallback}", &mut env).unwrap(),
        "fallback"
    );
}

#[test]
fn brace_default_empty_without_colon() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("")).unwrap();
    assert_eq!(expand_word_nosplit("${X-fallback}", &mut env).unwrap(), "");
}

#[test]
fn brace_assign() {
    let mut env = Env::empty();
    assert_eq!(
        expand_word_nosplit("${X:=hello}", &mut env).unwrap(),
        "hello"
    );
    assert_eq!(env.get_str("X"), "hello");
}

#[test]
fn brace_error_unset() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("${X:?variable X is required}", &mut env);
    assert!(matches!(result, Err(ExpandError::UnsetVarError { .. })));
}

#[test]
fn brace_alt_set() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    assert_eq!(expand_word_nosplit("${X:+alt}", &mut env).unwrap(), "alt");
}

#[test]
fn brace_alt_unset() {
    let mut env = Env::empty();
    assert_eq!(expand_word_nosplit("${X:+alt}", &mut env).unwrap(), "");
}

#[test]
fn quoted_star_uses_first_ifs_character() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["1".into(), "2".into(), "3".into()]);
    env.set("IFS", mash::env::Variable::string(", ")).unwrap();
    assert_eq!(expand_word_nosplit("\"$*\"", &mut env).unwrap(), "1,2,3");
}

#[test]
fn quoted_star_with_empty_ifs_concatenates_positional_args() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["a".into(), "b  e   e".into(), "c".into()]);
    env.set("IFS", mash::env::Variable::string("")).unwrap();
    assert_eq!(
        expand_word_nosplit("\"$*\"", &mut env).unwrap(),
        "ab  e   ec"
    );
}

#[test]
fn unquoted_star_with_empty_ifs_preserves_argument_boundaries() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["a".into(), "b  e   e".into(), "c".into()]);
    env.set("IFS", mash::env::Variable::string("")).unwrap();
    assert_eq!(
        expand_word("$*", &mut env).unwrap(),
        vec!["a".to_string(), "b  e   e".to_string(), "c".to_string()]
    );
}

#[test]
fn quoted_assignment_uses_quoted_star_semantics() {
    let mut env = Env::empty();
    env.set_positional_params(
        "mash",
        &[
            "a".into(),
            "s p  aces".into(),
            "b".into(),
            "c".into(),
            "and\ttabs\n and newlines".into(),
        ],
    );
    env.set("IFS", mash::env::Variable::string(": ")).unwrap();
    assert_eq!(
        expand_word_nosplit("\"${var=$*}\"", &mut env).unwrap(),
        "a:s p  aces:b:c:and\ttabs\n and newlines"
    );
}

#[test]
fn brace_length() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${#X}", &mut env).unwrap(), "5");
}

#[test]
fn brace_strip_prefix() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("/path/to/file"))
        .unwrap();
    assert_eq!(
        expand_word_nosplit("${X#*/}", &mut env).unwrap(),
        "path/to/file"
    );
}

#[test]
fn brace_strip_prefix_greedy() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("/path/to/file"))
        .unwrap();
    assert_eq!(expand_word_nosplit("${X##*/}", &mut env).unwrap(), "file");
}

#[test]
fn brace_strip_suffix() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("file.tar.gz"))
        .unwrap();
    assert_eq!(
        expand_word_nosplit("${X%.*}", &mut env).unwrap(),
        "file.tar"
    );
}

#[test]
fn brace_strip_suffix_greedy() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("file.tar.gz"))
        .unwrap();
    assert_eq!(expand_word_nosplit("${X%%.*}", &mut env).unwrap(), "file");
}

#[test]
fn brace_replace_first() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world hello"))
        .unwrap();
    assert_eq!(
        expand_word_nosplit("${X/hello/bye}", &mut env).unwrap(),
        "bye world hello"
    );
}

#[test]
fn brace_replace_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world hello"))
        .unwrap();
    assert_eq!(
        expand_word_nosplit("${X//hello/bye}", &mut env).unwrap(),
        "bye world bye"
    );
}

#[test]
fn brace_uppercase() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X^}", &mut env).unwrap(), "Hello");
}

#[test]
fn brace_uppercase_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X^^}", &mut env).unwrap(), "HELLO");
}

#[test]
fn brace_lowercase_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("HELLO")).unwrap();
    assert_eq!(expand_word_nosplit("${X,,}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_substring() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world"))
        .unwrap();
    assert_eq!(expand_word_nosplit("${X:6}", &mut env).unwrap(), "world");
}

#[test]
fn brace_substring_with_length() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world"))
        .unwrap();
    assert_eq!(expand_word_nosplit("${X:0:5}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_nested_default() {
    let mut env = Env::empty();
    env.set("FALLBACK", mash::env::Variable::string("default"))
        .unwrap();
    assert_eq!(
        expand_word_nosplit("${X:-$FALLBACK}", &mut env).unwrap(),
        "default"
    );
}

// ── Arithmetic evaluation tests ──

#[test]
fn arith_basic_add() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("1+2", &mut env).unwrap(), 3);
}

#[test]
fn arith_precedence() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("2+3*4", &mut env).unwrap(), 14);
}

#[test]
fn arith_parens() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("(2+3)*4", &mut env).unwrap(), 20);
}

#[test]
fn arith_variable() {
    let mut env = Env::empty();
    env.set("x", mash::env::Variable::string("5")).unwrap();
    assert_eq!(eval_arithmetic("x+1", &mut env).unwrap(), 6);
}

#[test]
fn arith_assignment() {
    let mut env = Env::empty();
    let val = eval_arithmetic("x=10", &mut env).unwrap();
    assert_eq!(val, 10);
    assert_eq!(env.get_str("x"), "10");
}

#[test]
fn arith_hex() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("0xFF", &mut env).unwrap(), 255);
}

#[test]
fn arith_octal() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("010", &mut env).unwrap(), 8);
}

#[test]
fn arith_power() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("2**8", &mut env).unwrap(), 256);
}

#[test]
fn arith_ternary() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("1>0 ? 42 : -1", &mut env).unwrap(), 42);
}

#[test]
fn arith_division_by_zero() {
    let mut env = Env::empty();
    let result = eval_arithmetic("1/0", &mut env);
    assert!(matches!(result, Err(ExpandError::Arithmetic { .. })));
}

#[test]
fn arith_compound_assign() {
    let mut env = Env::empty();
    env.set("x", mash::env::Variable::string("10")).unwrap();
    assert_eq!(eval_arithmetic("x+=5", &mut env).unwrap(), 15);
    assert_eq!(env.get_str("x"), "15");
}

#[test]
fn arith_pre_increment() {
    let mut env = Env::empty();
    env.set("x", mash::env::Variable::string("5")).unwrap();
    assert_eq!(eval_arithmetic("++x", &mut env).unwrap(), 6);
    assert_eq!(env.get_str("x"), "6");
}

#[test]
fn arith_in_expansion() {
    let mut env = Env::empty();
    assert_eq!(expand_word_nosplit("$((2+3))", &mut env).unwrap(), "5");
}

#[test]
fn arith_bitwise() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("0xFF & 0x0F", &mut env).unwrap(), 15);
}

#[test]
fn arith_logical() {
    let mut env = Env::empty();
    assert_eq!(eval_arithmetic("1&&0", &mut env).unwrap(), 0);
    assert_eq!(eval_arithmetic("1||0", &mut env).unwrap(), 1);
}

// ── Word splitting tests ──

#[test]
fn word_split_default_ifs() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("a  b  c"))
        .unwrap();
    let result = expand_word("$X", &mut env).unwrap();
    assert_eq!(result, vec!["a", "b", "c"]);
}

#[test]
fn word_split_custom_ifs() {
    let mut env = Env::empty();
    env.set("IFS", mash::env::Variable::string(":")).unwrap();
    env.set("X", mash::env::Variable::string("a:b:c")).unwrap();
    let result = expand_word("$X", &mut env).unwrap();
    assert_eq!(result, vec!["a", "b", "c"]);
}

#[test]
fn word_split_empty_ifs_no_split() {
    let mut env = Env::empty();
    env.set("IFS", mash::env::Variable::string("")).unwrap();
    env.set("X", mash::env::Variable::string("a b c")).unwrap();
    let result = expand_word("$X", &mut env).unwrap();
    assert_eq!(result, vec!["a b c"]);
}

#[test]
fn word_split_quoted_no_split() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("a b c")).unwrap();
    let result = expand_word("\"$X\"", &mut env).unwrap();
    assert_eq!(result, vec!["a b c"]);
}

#[test]
fn word_split_leading_trailing_ws_trimmed() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("  a b  "))
        .unwrap();
    let result = expand_word("$X", &mut env).unwrap();
    assert_eq!(result, vec!["a", "b"]);
}

#[test]
fn word_split_nonws_ifs_preserves_empty() {
    let mut env = Env::empty();
    env.set("IFS", mash::env::Variable::string(":")).unwrap();
    env.set("X", mash::env::Variable::string("a::b")).unwrap();
    let result = expand_word("$X", &mut env).unwrap();
    assert_eq!(result, vec!["a", "", "b"]);
}

// ── Glob tests ──

#[test]
fn glob_no_metachar_unchanged() {
    let mut env = Env::empty();
    let result = expand_word("hello", &mut env).unwrap();
    assert_eq!(result, vec!["hello"]);
}

#[test]
fn glob_noglob_skips() {
    let mut env = Env::empty();
    env.options_mut().noglob = true;
    // Even with a metachar, noglob prevents expansion.
    let result = expand_word("*.rs", &mut env).unwrap();
    assert_eq!(result, vec!["*.rs"]);
}

#[test]
fn glob_no_match_returns_pattern() {
    let mut env = Env::empty();
    let result = expand_word("*.nonexistent_xyz_42", &mut env).unwrap();
    assert_eq!(result, vec!["*.nonexistent_xyz_42"]);
}

#[test]
fn glob_quoted_star_literal() {
    let mut env = Env::empty();
    // Single-quoted: no glob expansion.
    let result = expand_word("'*.rs'", &mut env).unwrap();
    assert_eq!(result, vec!["*.rs"]);
}

#[test]
fn shell_pattern_bracket_expression_supports_literal_hyphen_and_right_bracket() {
    assert!(shell_pattern_match("file-", "file[-123]"));
    assert!(shell_pattern_match("file-", "file[123-]"));
    assert!(shell_pattern_match("file]", "file[]123]"));
}

#[test]
fn shell_pattern_bracket_expression_supports_posix_class_and_collating_literals() {
    assert!(shell_pattern_match("filea", "file[[:alpha:]]"));
    assert!(shell_pattern_match("file-", "file[[.-.]]"));
    assert!(shell_pattern_match("file]", "file[[.].]]"));
    assert!(shell_pattern_match("file-", "file[[=-=]]"));
    assert!(shell_pattern_match("file]", "file[[=]=]]"));
}

#[test]
fn assignment_tilde_expands_at_start_and_after_colon() {
    let mut env = Env::empty();
    env.set("HOME", mash::env::Variable::string("/home/alice"))
        .unwrap();

    assert_eq!(
        expand_assignment_word_nosplit("~", &mut env).unwrap(),
        "/home/alice"
    );
    assert_eq!(
        expand_assignment_word_nosplit("~/bin", &mut env).unwrap(),
        "/home/alice/bin"
    );
    assert_eq!(
        expand_assignment_word_nosplit(":~", &mut env).unwrap(),
        ":/home/alice"
    );
    assert_eq!(
        expand_assignment_word_nosplit("foo:~:bar", &mut env).unwrap(),
        "foo:/home/alice:bar"
    );
}

#[test]
fn non_assignment_tilde_before_colon_stays_literal() {
    let mut env = Env::empty();
    env.set("HOME", mash::env::Variable::string("/home/alice"))
        .unwrap();

    assert_eq!(expand_word("~:", &mut env).unwrap(), vec!["~:".to_string()]);
}

#[test]
fn non_assignment_tilde_with_quoted_suffix_stays_literal_and_drops_quotes() {
    let mut env = Env::empty();
    env.set("HOME", mash::env::Variable::string("/home/alice"))
        .unwrap();

    assert_eq!(
        expand_word("~\"alice\"", &mut env).unwrap(),
        vec!["~alice".to_string()]
    );
}

#[test]
#[cfg(windows)]
fn windows_shell_path_variable_expands_without_truncation() {
    let mut env = Env::empty();
    env.set(
        "TEST_SHELL",
        mash::env::Variable::string(
            "C:/Users/mamuk/projects/orix/malt/.worktrees/windows-smoosh-baseline/target-wave3/debug/mash.exe",
        ),
    )
    .unwrap();

    assert_eq!(
        expand_word("$TEST_SHELL", &mut env).unwrap(),
        vec![
            "C:/Users/mamuk/projects/orix/malt/.worktrees/windows-smoosh-baseline/target-wave3/debug/mash.exe"
                .to_string()
        ]
    );
}

#[test]
fn pathname_glob_supports_posix_bracket_forms() {
    let _guard = CWD_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    std::fs::write("file-", "").unwrap();
    std::fs::write("file]", "").unwrap();
    std::fs::write("filea", "").unwrap();

    let mut env = Env::empty();
    assert_eq!(expand_word("file[[.-.]]", &mut env).unwrap(), vec!["file-"]);
    assert_eq!(expand_word("file[[=-=]]", &mut env).unwrap(), vec!["file-"]);
    assert_eq!(expand_word("file[[.].]]", &mut env).unwrap(), vec!["file]"]);
    assert_eq!(expand_word("file[[=]=]]", &mut env).unwrap(), vec!["file]"]);
    assert_eq!(
        expand_word("file[[:alpha:]]", &mut env).unwrap(),
        vec!["filea"]
    );

    std::env::set_current_dir(prev).unwrap();
}

#[test]
fn pathname_glob_preserves_explicit_double_slash_segment() {
    let _guard = CWD_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    std::fs::create_dir("foo").unwrap();
    std::fs::write("foo/a", "").unwrap();
    std::fs::write("foo/b", "").unwrap();
    std::fs::write("foo/c", "").unwrap();

    let mut env = Env::empty();
    assert_eq!(
        expand_word("foo//*", &mut env).unwrap(),
        vec!["foo//a", "foo//b", "foo//c"]
    );

    std::env::set_current_dir(prev).unwrap();
}

#[test]
fn pathname_glob_synthesizes_dot_and_dotdot_matches() {
    let _guard = CWD_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    std::fs::create_dir_all("bar/inner").unwrap();
    std::fs::write("bar/foo", "").unwrap();
    std::fs::write("bar/inner/foo", "").unwrap();
    std::env::set_current_dir(dir.path().join("bar/inner")).unwrap();

    let mut env = Env::empty();
    assert_eq!(
        expand_word(".*/foo", &mut env).unwrap(),
        vec!["../foo", "./foo"]
    );

    std::env::set_current_dir(prev).unwrap();
}

// ── Heredoc expansion tests ──

#[test]
fn heredoc_expands_vars() {
    let mut env = Env::empty();
    env.set("NAME", mash::env::Variable::string("world"))
        .unwrap();
    let result = expand_heredoc_body("hello $NAME\n", &mut env).unwrap();
    assert_eq!(result, "hello world\n");
}

#[test]
fn heredoc_quotes_literal() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    let result = expand_heredoc_body("it's \"here\" $X\n", &mut env).unwrap();
    assert_eq!(result, "it's \"here\" val\n");
}

// ── Full pipeline / multi-var tests ──

#[test]
fn full_pipeline_mixed() {
    let mut env = Env::empty();
    env.set("USER", mash::env::Variable::string("alice"))
        .unwrap();
    env.set("HOME", mash::env::Variable::string("/home/alice"))
        .unwrap();
    let result = expand_word_nosplit("Welcome $USER to $HOME", &mut env).unwrap();
    assert_eq!(result, "Welcome alice to /home/alice");
}

#[test]
fn mixed_quoting_and_expansion() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("world")).unwrap();
    let result = expand_word_nosplit("'hello' \"$X\"", &mut env).unwrap();
    assert_eq!(result, "hello world");
}

// ── Nested expansion tests ──

#[test]
fn nested_parameter_default() {
    let mut env = Env::empty();
    env.set("FALLBACK", mash::env::Variable::string("default"))
        .unwrap();
    let result = expand_word_nosplit("${X:-${FALLBACK}}", &mut env).unwrap();
    assert_eq!(result, "default");
}

#[test]
fn arith_in_word() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("count=$((5 + 3))", &mut env).unwrap();
    assert_eq!(result, "count=8");
}
