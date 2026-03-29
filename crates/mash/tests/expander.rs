use mash::env::Env;
use mash::expander::*;

#[test]
fn tilde_expands_to_home() {
    let mut env = Env::empty();
    env.set("HOME", mash::env::Variable::string("/home/user")).unwrap();
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
    env.set("OLDPWD", mash::env::Variable::string("/var")).unwrap();
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
    env.set("NAME", mash::env::Variable::string("world")).unwrap();
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
fn command_sub_returns_error() {
    let mut env = Env::empty();
    let result = expand_word_nosplit("$(echo hello)", &mut env);
    assert!(matches!(result, Err(ExpandError::CommandSubstitution(_))));
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
    assert_eq!(expand_word_nosplit("${X:-fallback}", &mut env).unwrap(), "fallback");
}

#[test]
fn brace_default_set() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("val")).unwrap();
    assert_eq!(expand_word_nosplit("${X:-fallback}", &mut env).unwrap(), "val");
}

#[test]
fn brace_default_empty_with_colon() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("")).unwrap();
    assert_eq!(expand_word_nosplit("${X:-fallback}", &mut env).unwrap(), "fallback");
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
    assert_eq!(expand_word_nosplit("${X:=hello}", &mut env).unwrap(), "hello");
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
fn brace_length() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello")).unwrap();
    assert_eq!(expand_word_nosplit("${#X}", &mut env).unwrap(), "5");
}

#[test]
fn brace_strip_prefix() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("/path/to/file")).unwrap();
    assert_eq!(expand_word_nosplit("${X#*/}", &mut env).unwrap(), "path/to/file");
}

#[test]
fn brace_strip_prefix_greedy() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("/path/to/file")).unwrap();
    assert_eq!(expand_word_nosplit("${X##*/}", &mut env).unwrap(), "file");
}

#[test]
fn brace_strip_suffix() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("file.tar.gz")).unwrap();
    assert_eq!(expand_word_nosplit("${X%.*}", &mut env).unwrap(), "file.tar");
}

#[test]
fn brace_strip_suffix_greedy() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("file.tar.gz")).unwrap();
    assert_eq!(expand_word_nosplit("${X%%.*}", &mut env).unwrap(), "file");
}

#[test]
fn brace_replace_first() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X/hello/bye}", &mut env).unwrap(), "bye world hello");
}

#[test]
fn brace_replace_all() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world hello")).unwrap();
    assert_eq!(expand_word_nosplit("${X//hello/bye}", &mut env).unwrap(), "bye world bye");
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
    env.set("X", mash::env::Variable::string("hello world")).unwrap();
    assert_eq!(expand_word_nosplit("${X:6}", &mut env).unwrap(), "world");
}

#[test]
fn brace_substring_with_length() {
    let mut env = Env::empty();
    env.set("X", mash::env::Variable::string("hello world")).unwrap();
    assert_eq!(expand_word_nosplit("${X:0:5}", &mut env).unwrap(), "hello");
}

#[test]
fn brace_nested_default() {
    let mut env = Env::empty();
    env.set("FALLBACK", mash::env::Variable::string("default")).unwrap();
    assert_eq!(expand_word_nosplit("${X:-$FALLBACK}", &mut env).unwrap(), "default");
}
