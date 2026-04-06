//! Standalone MASH binary for conformance testing
//!
//! This provides a POSIX-compatible shell interface for running
//! Smoosh and Modernish conformance tests.

use std::io::{self, BufRead, Write};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle -c flag (execute command string)
    if args.len() >= 3 && args[1] == "-c" {
        let command = &args[2];
        run_command(command);
        return;
    }

    // Handle --version
    if args.len() == 2 && (args[1] == "--version" || args[1] == "-v") {
        println!("MASH (MALT Shell) 0.1.0-phase-c");
        println!("POSIX-compliant shell for MALT");
        exit(0);
    }

    // Handle script file
    if args.len() == 2 {
        let script_path = &args[1];
        // On Windows, try common extensions if file doesn't exist
        #[cfg(windows)]
        let script_path = if !std::path::Path::new(script_path).exists() {
            let mut found = script_path.clone();
            for ext in [".sh", ".msh", ".bash", ".cmd", ".bat"] {
                let with_ext = format!("{}{}", script_path, ext);
                if std::path::Path::new(&with_ext).exists() {
                    found = with_ext;
                    break;
                }
            }
            found
        } else {
            script_path.clone()
        };
        #[cfg(not(windows))]
        let script_path = script_path.clone();
        run_script_file(&script_path);
        return;
    }

    // Interactive mode
    run_interactive();
}

fn run_command(command: &str) {
    use mash::env::Env;
    use mash::executor::execute_list;

    let mut env = Env::from_os();
    env.set_interactive(false);

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

fn run_script_file(path: &str) {
    use mash::env::Env;
    use mash::executor::execute_list;

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mash: {}: {}", path, e);
            exit(126);
        }
    };

    let mut env = Env::from_os();
    env.set_interactive(false);

    // Set $0 to script name
    let _ = env.set("0", mash::env::Variable::string(path));

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
