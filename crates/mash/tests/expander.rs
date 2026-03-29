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
