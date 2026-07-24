mod cli;
mod client;
mod daemon;
mod output;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command, IsolationTierArg};
use client::{MaltClient, SessionData};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = MaltClient::new(&cli.api_addr);

    match cli.command {
        None => handle_default(&cli.api_addr, &client),
        Some(Command::Status) => handle_status(&client),
        Some(Command::Daemon { port }) => daemon::run_daemon(port),
        Some(Command::Start) => handle_start(),
        Some(Command::Stop) => handle_stop(&client),
        Some(Command::List) => handle_list(&client),
        Some(Command::New { name, isolation }) => handle_new(&client, name.as_deref(), isolation),
        Some(Command::Attach { session_id }) => handle_attach(&cli.api_addr, session_id),
        Some(Command::Kill { session_id }) => handle_kill(&client, session_id),
        Some(Command::Exec {
            session_id,
            command,
        }) => handle_exec(&client, session_id, &command),
        Some(Command::Send { session_id, input }) => handle_send(&client, session_id, &input),
        Some(Command::Output { session_id }) => handle_output(&client, session_id),
    }
}

fn handle_default(api_addr: &str, client: &MaltClient) -> Result<()> {
    // 1. Check if daemon is running
    if client.health().is_err() {
        // Start daemon in background
        eprintln!("starting daemon...");
        handle_start()?;

        // Wait for daemon to be ready (up to 5 seconds)
        let mut ready = false;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if client.health().is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            anyhow::bail!("daemon failed to start within 5 seconds");
        }
    }

    // 2. Check for existing sessions
    let sessions = client.list_sessions()?;
    let session_id = if sessions.is_empty() {
        // Create a new session
        eprintln!("creating session...");
        let session = client.create_session(None, None)?;
        // Give shell a moment to start
        std::thread::sleep(std::time::Duration::from_millis(500));
        session.id
    } else {
        // Use the first (most recent) session
        sessions[0].id
    };

    // 3. Attach to the session
    handle_attach(api_addr, Some(session_id))
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
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    println!("daemon started (pid: {})", child.id());
    Ok(())
}

fn handle_stop(client: &MaltClient) -> Result<()> {
    match client.shutdown() {
        Ok(_) => println!("daemon stopped"),
        Err(_) => println!("daemon not running"),
    }
    Ok(())
}

fn handle_list(client: &MaltClient) -> Result<()> {
    let sessions = client.list_sessions()?;
    output::print_sessions(&sessions);
    Ok(())
}

fn handle_new(
    client: &MaltClient,
    name: Option<&str>,
    isolation: Option<IsolationTierArg>,
) -> Result<()> {
    let expected_tier = expected_isolation_tier(isolation);
    let session = client.create_session(name, isolation.map(IsolationTierArg::request_value))?;
    validate_created_session(&session, expected_tier)?;
    println!("{}", creation_message(&session));
    Ok(())
}

fn expected_isolation_tier(isolation: Option<IsolationTierArg>) -> IsolationTierArg {
    isolation.unwrap_or(IsolationTierArg::Bare)
}

fn validate_created_session(session: &SessionData, expected_tier: IsolationTierArg) -> Result<()> {
    if session
        .isolation
        .eq_ignore_ascii_case(expected_tier.request_value())
    {
        return Ok(());
    }

    anyhow::bail!(
        "session {} was created with isolation {}, not requested {}",
        session.id,
        session.isolation,
        expected_tier.display_value()
    );
}

fn creation_message(session: &SessionData) -> String {
    format!(
        "created session {} ({}) [{}]",
        session.id,
        session.name.as_deref().unwrap_or("-"),
        session.isolation
    )
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
    let tui_exe = exe_dir.join(if cfg!(windows) {
        "malt-tui.exe"
    } else {
        "malt-tui"
    });

    if !tui_exe.exists() {
        anyhow::bail!(
            "malt-tui not found at {}. Build it with: cargo build -p malt-tui",
            tui_exe.display()
        );
    }

    // Derive VNP port from API address (HTTP port + 1)
    let vnp_addr = {
        // Parse port from api_addr (e.g. "http://127.0.0.1:7700" -> 7701)
        let port = api_addr
            .rsplit(':')
            .next()
            .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
            .unwrap_or(7700);
        format!("127.0.0.1:{}", port + 1)
    };

    // Launch malt-tui as a child process (replaces our terminal)
    let status = std::process::Command::new(&tui_exe)
        .args([
            "--session",
            &id.to_string(),
            "--api-addr",
            api_addr,
            "--vnp",
            &vnp_addr,
        ])
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
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }
    if let Some(code) = result.exit_code {
        if code != 0 {
            std::process::exit(code);
        }
    }
    Ok(())
}

fn handle_output(client: &MaltClient, session_id: u32) -> Result<()> {
    let result = client.get_output_text(session_id)?;
    print!("{}", result.text);
    Ok(())
}

fn handle_send(client: &MaltClient, session_id: u32, input: &str) -> Result<()> {
    client.send_input(session_id, input)?;
    println!("sent to session {session_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::IsolationTierArg;

    fn session_with_isolation(isolation: &str) -> SessionData {
        SessionData {
            id: 42,
            name: Some("build".to_string()),
            pane_count: 1,
            isolation: isolation.to_string(),
            state: "Active".to_string(),
        }
    }

    #[test]
    fn matching_selected_tiers_validate_and_format_successfully() {
        for tier in [
            IsolationTierArg::Bare,
            IsolationTierArg::Restricted,
            IsolationTierArg::Capped,
            IsolationTierArg::Contained,
        ] {
            let session = session_with_isolation(tier.display_value());
            validate_created_session(&session, tier).unwrap();
            assert_eq!(
                creation_message(&session),
                format!("created session 42 (build) [{}]", tier.display_value())
            );
        }
    }

    #[test]
    fn omitted_isolation_defaults_to_bare() {
        assert_eq!(expected_isolation_tier(None), IsolationTierArg::Bare);
        assert_eq!(
            expected_isolation_tier(Some(IsolationTierArg::Restricted)),
            IsolationTierArg::Restricted
        );
    }

    #[test]
    fn mismatched_reported_tier_is_an_actionable_error() {
        let session = session_with_isolation("Bare");
        let error = validate_created_session(&session, IsolationTierArg::Restricted).unwrap_err();
        assert_eq!(
            error.to_string(),
            "session 42 was created with isolation Bare, not requested Restricted"
        );
        assert!(!error.to_string().contains("created session"));
    }
}
