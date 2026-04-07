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
                run_command(command, interactive);
                return;
            }
            arg if !arg.starts_with('-') || arg == "-" => {
                run_script_file(arg, interactive);
                return;
            }
            unknown => {
                eprintln!("mash: unsupported option: {unknown}");
                exit(2);
            }
        }
    }

    run_interactive();
}

fn run_command(command: &str, interactive: bool) {
    use mash::env::Env;
    use mash::executor::execute_list;

    let mut env = Env::from_os();
    env.set_interactive(interactive);

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

fn run_script_file(path: &str, interactive: bool) {
    use mash::env::Env;
    use mash::executor::execute_list;

    let path = resolve_script_path(path);
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mash: {}: {}", path, e);
            exit(126);
        }
    };

    let mut env = Env::from_os();
    env.set_interactive(interactive);

    // Set $0 to script name
    let _ = env.set("0", mash::env::Variable::string(&path));

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
    use mash::env::Env;
    use mash::executor::execute;

    let mut env = Env::from_os();
    env.set_interactive(true);

    let stdin = io::stdin();

    println!("MASH 0.1.0 -- POSIX Shell for MALT");
    println!("Type 'exit' to quit");
    println!();

    loop {
        print!("$ ");
        let _ = io::stdout().flush();

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
                    }
                }
            }
            Err(e) => {
                eprintln!("mash: read error: {}", e);
                break;
            }
        }
    }
}
