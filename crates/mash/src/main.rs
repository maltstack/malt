//! Standalone MASH binary for conformance testing
//!
//! This provides a POSIX-compatible shell interface for running
//! Smoosh and Modernish conformance tests.

use std::io::{self, BufRead, Write};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 2 && (args[1] == "--version" || args[1] == "-v") {
        println!("MASH (MALT Shell) 0.1.0-phase-c");
        println!("POSIX-compliant shell for MALT");
        exit(0);
    }

    let mut interactive = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                interactive = true;
                i += 1;
            }
            "-c" => {
                let Some(command) = args.get(i + 1) else {
                    eprintln!("mash: -c: option requires an argument");
                    exit(2);
                };
                let arg0 = args.get(i + 2).cloned().unwrap_or_else(|| "mash".to_string());
                let positional = if i + 3 <= args.len() {
                    args[i + 3..].to_vec()
                } else {
                    Vec::new()
                };
                run_command(command, interactive, &arg0, &positional);
                return;
            }
            arg if !arg.starts_with('-') || arg == "-" => {
                let positional = if i + 1 <= args.len() {
                    args[i + 1..].to_vec()
                } else {
                    Vec::new()
                };
                run_script_file(arg, interactive, &positional);
                return;
            }
            unknown => {
                eprintln!("mash: unsupported option: {unknown}");
                exit(2);
            }
        }
    }

    if interactive && !malt_platform::io::is_tty(0) {
        run_stdin(interactive, true);
        return;
    }

    run_interactive();
}

fn run_command(command: &str, interactive: bool, arg0: &str, positional: &[String]) {
    use mash::env::{Env, Variable};
    use mash::executor::execute_list;

    let mut env = Env::from_os();
    env.set_interactive(interactive);
    env.set_positional_params(arg0, positional);
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = env.set(
            "MASH_SELF_EXE",
            Variable::exported_string(self_exe.to_string_lossy().into_owned()),
        );
    }

    match mash::parser::parse(command) {
        Ok(commands) => {
            let result = execute_list(&commands, command, &mut env);

            // Print stdout
            if !result.stdout.is_empty() {
                let _ = io::stdout().write_all(&result.stdout);
            }

            // Print stderr
            if !result.stderr.is_empty() {
                let _ = io::stderr().write_all(&result.stderr);
            }

            exit(result.exit_code);
        }
        Err(e) => {
            eprintln!("mash: parse error: {}", e);
            exit(1);
        }
    }
}

fn run_script_file(path: &str, interactive: bool, positional: &[String]) {
    use mash::env::{Env, Variable};
    use mash::executor::execute_list;

    let path = resolve_script_path(path);
    let path_buf = std::path::PathBuf::from(&path);
    if !malt_platform::fs::is_readable(&path_buf) {
        eprintln!("mash: {}: Permission denied", path);
        exit(126);
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mash: {}: {}", path, e);
            exit(126);
        }
    };

    let mut env = Env::from_os();
    env.set_interactive(interactive);
    env.set_positional_params(&path, positional);
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = env.set(
            "MASH_SELF_EXE",
            Variable::exported_string(self_exe.to_string_lossy().into_owned()),
        );
    }

    match mash::parser::parse(&contents) {
        Ok(commands) => {
            let result = execute_list(&commands, &contents, &mut env);

            if !result.stdout.is_empty() {
                let _ = io::stdout().write_all(&result.stdout);
            }

            if !result.stderr.is_empty() {
                let _ = io::stderr().write_all(&result.stderr);
            }

            exit(result.exit_code);
        }
        Err(e) => {
            eprintln!("mash: {}: parse error: {}", path, e);
            exit(1);
        }
    }
}

fn resolve_script_path(path: &str) -> String {
    #[cfg(windows)]
    {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }

        for ext in [".sh", ".msh", ".bash", ".cmd", ".bat"] {
            let with_ext = format!("{path}{ext}");
            if std::path::Path::new(&with_ext).exists() {
                return with_ext;
            }
        }
    }

    path.to_string()
}

fn run_interactive() {
    run_stdin(true, true);
}

fn run_stdin(interactive: bool, prompt: bool) {
    use mash::env::{Env, Variable};
    use mash::executor::execute;

    let mut env = Env::from_os();
    env.set_interactive(interactive);
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = env.set(
            "MASH_SELF_EXE",
            Variable::exported_string(self_exe.to_string_lossy().into_owned()),
        );
    }

    let stdin = io::stdin();

    loop {
        if prompt {
            let ps1 = env.get_str("PS1");
            if ps1.is_empty() {
                eprint!("$ ");
            } else {
                eprint!("{ps1}");
            }
            let _ = io::stderr().flush();
        }

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if line == "exit" {
                    break;
                }

                match mash::parser::parse(line) {
                    Ok(commands) => {
                        for cmd in commands {
                            let result = execute(&cmd, line, &mut env);

                            if !result.stdout.is_empty() {
                                let _ = io::stdout().write_all(&result.stdout);
                            }

                            if !result.stderr.is_empty() {
                                let _ = io::stderr().write_all(&result.stderr);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("mash: parse error: {}", e);
                        env.set_exit_code(1);
                        if !prompt {
                            exit(1);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("mash: read error: {}", e);
                break;
            }
        }
    }

    exit(env.exit_code());
}
