mod cli;
mod client;
mod daemon;
mod output;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};
use client::MaltClient;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = MaltClient::new(&cli.api_addr);

    match cli.command {
        None | Some(Command::Status) => handle_status(&client),
        Some(Command::Daemon { port }) => daemon::run_daemon(port),
        Some(Command::Start) => handle_start(),
        Some(Command::Stop) => handle_stop(),
        Some(Command::List) => handle_list(&client),
        Some(Command::New { name }) => handle_new(&client, name.as_deref()),
        Some(Command::Attach { session_id }) => handle_attach(&cli.api_addr, session_id),
        Some(Command::Kill { session_id }) => handle_kill(&client, session_id),
        Some(Command::Exec {
            session_id,
            command,
        }) => handle_exec(&client, session_id, &command),
        Some(Command::Send { session_id, input }) => handle_send(&client, session_id, &input),
    }
}

fn handle_status(client: &MaltClient) -> Result<()> {
    match client.health() {
        Ok(h) => {
            println!("daemon: {}", h.status);
        }
        Err(_) => {
            println!("daemon: not reachable");
            return Ok(());
        }
    }
    println!();
    let sessions = client.list_sessions()?;
    output::print_sessions(&sessions);
    Ok(())
}

fn handle_start() -> Result<()> {
    let exe = std::env::current_exe()?;
    let child = std::process::Command::new(exe)
        .args(["daemon"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    println!("daemon started (pid: {})", child.id());
    Ok(())
}

fn handle_stop() -> Result<()> {
    println!("stop: not yet implemented");
    Ok(())
}

fn handle_list(client: &MaltClient) -> Result<()> {
    let sessions = client.list_sessions()?;
    output::print_sessions(&sessions);
    Ok(())
}

fn handle_new(client: &MaltClient, name: Option<&str>) -> Result<()> {
    let session = client.create_session(name)?;
    println!(
        "created session {} ({})",
        session.id,
        session.name.as_deref().unwrap_or("-")
    );
    Ok(())
}

fn handle_attach(api_addr: &str, session_id: Option<u32>) -> Result<()> {
    let id = match session_id {
        Some(id) => id,
        None => {
            // Default to session 1 if not specified
            println!("no session ID specified, defaulting to 1");
            1
        }
    };

    // Find malt-tui binary next to our own executable
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));
    let tui_exe = exe_dir.join(if cfg!(windows) { "malt-tui.exe" } else { "malt-tui" });

    if !tui_exe.exists() {
        anyhow::bail!(
            "malt-tui not found at {}. Build it with: cargo build -p malt-tui",
            tui_exe.display()
        );
    }

    // Launch malt-tui as a child process (replaces our terminal)
    let status = std::process::Command::new(&tui_exe)
        .args(["--session", &id.to_string(), "--api-addr", api_addr])
        .status()?;

    if !status.success() {
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
    }
    Ok(())
}

fn handle_kill(client: &MaltClient, session_id: u32) -> Result<()> {
    client.destroy_session(session_id)?;
    println!("killed session {session_id}");
    Ok(())
}

fn handle_exec(client: &MaltClient, session_id: u32, command: &str) -> Result<()> {
    let result = client.exec_command(session_id, command)?;
    if !result.output.is_empty() {
        print!("{}", result.output);
    }
    if let Some(code) = result.exit_code {
        if code != 0 {
            std::process::exit(code);
        }
    }
    Ok(())
}

fn handle_send(client: &MaltClient, session_id: u32, input: &str) -> Result<()> {
    client.send_input(session_id, input)?;
    println!("sent to session {session_id}");
    Ok(())
}
