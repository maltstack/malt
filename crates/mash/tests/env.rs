use mash::env::*;

// ── Task 1: Constructor tests ──

#[test]
fn empty_env_has_defaults() {
    let env = Env::empty();
    assert_eq!(env.exit_code(), 0);
    assert!(!env.is_interactive());
    assert!(!env.options().errexit);
}

#[test]
fn empty_env_has_pid() {
    let env = Env::empty();
    let pid = env.get_str("$");
    assert!(!pid.is_empty());
    assert!(pid.parse::<u32>().is_ok());
}

#[test]
fn from_os_has_path() {
    let env = Env::from_os();
    // PATH should exist on all platforms
    assert!(env.get("PATH").is_some() || env.get("Path").is_some());
}

#[test]
fn from_os_vars_are_exported() {
    let env = Env::from_os();
    if let Some(var) = env.get("PATH").or(env.get("Path")) {
        assert!(var.exported);
    }
}

#[test]
#[cfg(windows)]
fn from_os_normalizes_pwd_separators() {
    let env = Env::from_os();
    let pwd = env.get_str("PWD");
    assert!(!pwd.contains('\\'), "PWD should use forward slashes: {pwd}");
}

#[test]
fn from_os_does_not_import_ifs() {
    let original = std::env::var_os("IFS");
    std::env::set_var("IFS", "123");
    let env = Env::from_os();
    assert_eq!(env.get_str("IFS"), " \t\n");
    assert!(env.is_set("IFS"));
    assert!(!env.get("IFS").expect("IFS should exist").exported);
    match original {
        Some(value) => std::env::set_var("IFS", value),
        None => std::env::remove_var("IFS"),
    }
}

#[test]
fn empty_env_sets_default_ifs() {
    let env = Env::empty();
    assert_eq!(env.get_str("IFS"), " \t\n");
    assert!(env.is_set("IFS"));
    assert!(!env.get("IFS").expect("IFS should exist").exported);
}

// ── Task 2: Variable access + scope stack ──

#[test]
fn set_and_get_variable() {
    let mut env = Env::empty();
    env.set("FOO", Variable::string("bar")).unwrap();
    assert_eq!(env.get_str("FOO"), "bar");
}

#[test]
fn get_unset_returns_empty() {
    let env = Env::empty();
    assert_eq!(env.get_str("NONEXISTENT"), "");
    assert!(!env.is_set("NONEXISTENT"));
}

#[test]
fn scope_isolation() {
    let mut env = Env::empty();
    env.set("X", Variable::string("global")).unwrap();
    env.push_scope();
    env.set("X", Variable::string("local")).unwrap();
    assert_eq!(env.get_str("X"), "local");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "global");
}

#[test]
fn scope_child_sees_parent() {
    let mut env = Env::empty();
    env.set("X", Variable::string("parent")).unwrap();
    env.push_scope();
    assert_eq!(env.get_str("X"), "parent");
    env.pop_scope().unwrap();
}

#[test]
fn unset_masks_parent() {
    let mut env = Env::empty();
    env.set("X", Variable::string("parent")).unwrap();
    env.push_scope();
    env.unset("X").unwrap();
    assert!(!env.is_set("X"));
    assert_eq!(env.get_str("X"), "");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "parent"); // Parent unaffected
}

#[test]
fn readonly_prevents_set() {
    let mut env = Env::empty();
    env.set("X", Variable::string("val")).unwrap();
    env.mark_readonly("X");
    assert!(env.set("X", Variable::string("new")).is_err());
}

#[test]
fn readonly_prevents_unset() {
    let mut env = Env::empty();
    env.set("X", Variable::string("val")).unwrap();
    env.mark_readonly("X");
    assert!(env.unset("X").is_err());
}

#[test]
fn pop_global_scope_fails() {
    let mut env = Env::empty();
    assert!(env.pop_scope().is_err());
}

#[test]
fn nested_scopes_three_deep() {
    let mut env = Env::empty();
    env.set("X", Variable::string("0")).unwrap();
    env.push_scope();
    env.set("X", Variable::string("1")).unwrap();
    env.push_scope();
    env.set("X", Variable::string("2")).unwrap();
    assert_eq!(env.get_str("X"), "2");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "1");
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "0");
}

#[test]
fn exported_vars_only_exported() {
    let mut env = Env::empty();
    env.set("A", Variable::string("local")).unwrap();
    env.set("B", Variable::exported_string("exported")).unwrap();
    let exported = env.exported_vars();
    assert!(!exported.contains_key("A"));
    assert_eq!(exported.get("B").unwrap(), "exported");
}

#[test]
fn set_global_bypasses_scope() {
    let mut env = Env::empty();
    env.push_scope();
    env.set_global("X", Variable::string("global")).unwrap();
    env.pop_scope().unwrap();
    assert_eq!(env.get_str("X"), "global");
}

// ── Task 3: Special parameters + options ──

#[test]
fn positional_params() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["a".into(), "b".into(), "c".into()]);
    assert_eq!(env.get_str("0"), "mash");
    assert_eq!(env.get_str("1"), "a");
    assert_eq!(env.get_str("2"), "b");
    assert_eq!(env.get_str("3"), "c");
    assert_eq!(env.get_str("#"), "3");
}

#[test]
fn replace_positional_preserves_zero() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["old".into()]);
    env.replace_positional_args(&["new1".into(), "new2".into()]);
    assert_eq!(env.get_str("0"), "mash"); // preserved
    assert_eq!(env.get_str("1"), "new1");
    assert_eq!(env.get_str("2"), "new2");
    assert_eq!(env.get_str("#"), "2");
}

#[test]
fn save_restore_positional() {
    let mut env = Env::empty();
    env.set_positional_params("mash", &["a".into(), "b".into()]);
    let saved = env.save_positional();
    env.replace_positional_args(&["x".into()]);
    assert_eq!(env.get_str("1"), "x");
    env.restore_positional(saved);
    assert_eq!(env.get_str("1"), "a");
    assert_eq!(env.get_str("#"), "2");
}

#[test]
fn exit_code_tracking() {
    let mut env = Env::empty();
    assert_eq!(env.exit_code(), 0);
    env.set_exit_code(42);
    assert_eq!(env.exit_code(), 42);
    assert_eq!(env.get_str("?"), "42");
}

#[test]
fn bg_pid_tracking() {
    let mut env = Env::empty();
    env.set_last_bg_pid(12345);
    assert_eq!(env.get_str("!"), "12345");
}

#[test]
fn options_flags_string() {
    let mut env = Env::empty();
    env.options_mut().errexit = true;
    env.options_mut().xtrace = true;
    let flags = env.options().flags_string();
    assert!(flags.contains('e'));
    assert!(flags.contains('x'));
    assert!(!flags.contains('u'));
}

#[test]
fn loop_control_default() {
    let env = Env::empty();
    assert_eq!(*env.loop_control(), LoopControl::None);
}

#[test]
fn dir_stack_push_pop() {
    let mut env = Env::empty();
    env.push_dir("/home".to_string());
    env.push_dir("/tmp".to_string());
    assert_eq!(env.dir_stack().len(), 2);
    assert_eq!(env.pop_dir(), Some("/tmp".to_string()));
    assert_eq!(env.dir_stack().len(), 1);
}

#[test]
fn call_depth_tracking() {
    let mut env = Env::empty();
    assert_eq!(env.call_depth(), 0);
    env.push_call(CallFrame {
        name: "f".into(),
        file: "test".into(),
        line: 1,
    });
    assert_eq!(env.call_depth(), 1);
    env.pop_call();
    assert_eq!(env.call_depth(), 0);
}

// ── Task 4: Functions, aliases, traps ──

#[test]
fn alias_set_get_unset() {
    let mut env = Env::empty();
    env.set_alias("ll".into(), "ls -la".into());
    assert_eq!(env.get_alias("ll"), Some("ls -la"));
    assert!(env.unset_alias("ll"));
    assert_eq!(env.get_alias("ll"), None);
    assert!(!env.unset_alias("ll")); // already removed
}

#[test]
fn function_define_get_unset() {
    let mut env = Env::empty();
    let body = mash::parser::parse("echo hello").unwrap().remove(0);
    env.define_function("greet".into(), "echo hello".into(), body);
    assert!(env.get_function("greet").is_some());
    assert_eq!(env.get_function("greet").unwrap().source, "echo hello");
    env.unset_function("greet");
    assert!(env.get_function("greet").is_none());
}

#[test]
fn trap_set_get_clear() {
    let mut env = Env::empty();
    env.set_trap(
        "INT".into(),
        TrapAction {
            action: "echo caught".into(),
            inherited: false,
        },
    );
    assert!(env.get_trap("INT").is_some());
    assert_eq!(env.get_trap("INT").unwrap().action, "echo caught");
    env.clear_trap("INT");
    assert!(env.get_trap("INT").is_none());
}

// ── Task 5: Persistence (EnvSnapshot) ──

#[test]
fn snapshot_roundtrip_variables() {
    let mut env = Env::empty();
    env.set("FOO", Variable::exported_string("bar")).unwrap();
    env.set("BAZ", Variable::string("qux")).unwrap();
    env.options_mut().errexit = true;
    env.set_alias("ll".into(), "ls -la".into());

    let snapshot = env.to_snapshot();

    let mut restored = Env::empty();
    restored.apply_snapshot(&snapshot);

    assert_eq!(restored.get_str("FOO"), "bar");
    assert_eq!(restored.get_str("BAZ"), "qux");
    assert!(restored.get("FOO").unwrap().exported);
    assert!(restored.options().errexit);
    assert_eq!(restored.get_alias("ll"), Some("ls -la"));
}

#[test]
fn snapshot_roundtrip_functions() {
    // Use a real, full function-definition statement (as
    // executor.rs::execute_inner's Command::FunctionDef handling actually
    // stores in `FunctionDef.source` -- the whole "name() { body }" text,
    // not just the inner body) and verify the restored function actually
    // *runs* correctly, not just that an entry with the right name exists.
    // A prior version of this test used unrealistic source text ("echo
    // hello" instead of "greet() { echo hello; }") and only checked
    // `get_function(..).is_some()`, which passed even though
    // `apply_snapshot` was storing the wrong `body` (the whole reparsed
    // `FunctionDef` node instead of its inner body) -- a real bug, only
    // caught once something actually executed the restored function.
    let mut env = Env::from_os();
    let cmds = mash::parser::parse("greet() { echo hello; }").unwrap();
    mash::executor::execute_list(&cmds, "greet() { echo hello; }", &mut env);

    let snapshot = env.to_snapshot();
    assert_eq!(
        snapshot.functions.get("greet").unwrap(),
        "greet() { echo hello; }"
    );

    let mut restored = Env::from_os();
    restored.apply_snapshot(&snapshot);
    assert!(restored.get_function("greet").is_some());

    let call_cmds = mash::parser::parse("greet").unwrap();
    let result = mash::executor::execute_list(&call_cmds, "greet", &mut restored);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "hello\n",
        "the restored function must actually execute its real body, not a \
         mis-stored FunctionDef node"
    );
}

#[test]
fn snapshot_only_global_scope() {
    let mut env = Env::empty();
    env.set("GLOBAL", Variable::string("yes")).unwrap();
    env.push_scope();
    env.set("LOCAL", Variable::string("no")).unwrap();

    let snapshot = env.to_snapshot();

    // Only global scope variables in snapshot
    assert!(snapshot.variables.contains_key("GLOBAL"));
    assert!(!snapshot.variables.contains_key("LOCAL"));
}

#[test]
fn snapshot_traps_roundtrip() {
    let mut env = Env::empty();
    env.set_trap(
        "EXIT".into(),
        TrapAction {
            action: "echo bye".into(),
            inherited: false,
        },
    );

    let snapshot = env.to_snapshot();
    let mut restored = Env::empty();
    restored.apply_snapshot(&snapshot);

    assert_eq!(restored.get_trap("EXIT").unwrap().action, "echo bye");
}
