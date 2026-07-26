mod cli;
mod client;
mod daemon;
mod events;
mod output;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command, ElevateCommand, IsolationCommand, IsolationPolicyArg, IsolationTierArg};
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
        Some(Command::Isolation {
            command: IsolationCommand::Capabilities,
        }) => handle_isolation_capabilities(&client),
        Some(Command::Elevate { command }) => handle_elevate(command),
        Some(Command::New {
            name,
            isolation,
            isolation_policy,
        }) => handle_new(&client, name.as_deref(), isolation, isolation_policy),
        Some(Command::Attach { session_id }) => handle_attach(&cli.api_addr, session_id),
        Some(Command::Kill { session_id }) => handle_kill(&client, session_id),
        Some(Command::Exec {
            session_id,
            command,
        }) => handle_exec(&client, session_id, &command),
        Some(Command::Send { session_id, input }) => handle_send(&client, session_id, &input),
        Some(Command::Eof { session_id }) => {
            client.end_input(session_id)?;
            println!("end-of-input sent to session {session_id}");
            Ok(())
        }
        Some(Command::Output { session_id }) => handle_output(&client, session_id),
        Some(Command::History { session_id }) => handle_history(&client, session_id),
        Some(Command::Watch {
            session_id,
            resume_from,
            output,
        }) => {
            if output {
                handle_watch_output(&client, session_id, resume_from)
            } else {
                handle_watch(&client, session_id, resume_from)
            }
        }
    }
}

fn handle_elevate(command: ElevateCommand) -> Result<()> {
    match command {
        ElevateCommand::Status => {
            match malt_daemon::elevate_client::status()? {
                malt_daemon::elevate_client::HelperState::NotInstalled => {
                    println!("helper:   not installed");
                    println!("effect:   contained isolation is unavailable; required requests are refused");
                    println!(
                        "resolve:  run `malt elevate install` and accept the Windows UAC prompt"
                    );
                }
                malt_daemon::elevate_client::HelperState::InstalledStopped => {
                    println!("helper:   installed, not running");
                    println!(
                        "effect:   contained isolation is unavailable until the helper is running"
                    );
                    println!(
                        "resolve:  start the {} service",
                        malt_daemon::elevate_client::HELPER_SERVICE_NAME
                    );
                }
                malt_daemon::elevate_client::HelperState::InstalledUnreachable => {
                    println!("helper:   installed, but did not answer its authenticated VNP probe");
                    println!("effect:   contained isolation is unavailable; service bookkeeping is not reachability");
                    println!(
                        "resolve:  inspect the {} service and its event log",
                        malt_daemon::elevate_client::HELPER_SERVICE_NAME
                    );
                }
                malt_daemon::elevate_client::HelperState::Reachable { protocol_version } => {
                    println!("helper:   reachable");
                    println!("protocol: {protocol_version}");
                    println!("verified: authenticated VNP hello/ack round trip completed");
                }
                malt_daemon::elevate_client::HelperState::VersionMismatch { expected, actual } => {
                    println!("helper:   version mismatch");
                    println!("protocol: helper {actual}, daemon expects {expected}");
                    println!("effect:   no privileged operation will be attempted");
                    println!("resolve:  reinstall the helper from this MALT build in an elevated PowerShell");
                }
            }
            Ok(())
        }
        ElevateCommand::Install => {
            if relaunch_elevated_if_needed("install")? {
                return Ok(());
            }
            let helper = helper_executable()?;
            malt_daemon::elevate_client::install(&helper).map_err(|error| {
                anyhow::anyhow!(
                    "helper installation did not complete (run this explicit command from an elevated PowerShell): {error}"
                )
            })?;
            println!(
                "installed and started {}",
                malt_daemon::elevate_client::HELPER_SERVICE_NAME
            );
            handle_elevate(ElevateCommand::Status)
        }
        ElevateCommand::Uninstall => {
            if relaunch_elevated_if_needed("uninstall")? {
                return Ok(());
            }
            malt_daemon::elevate_client::uninstall().map_err(|error| {
                anyhow::anyhow!(
                    "helper removal did not complete (run this explicit command from an elevated PowerShell): {error}"
                )
            })?;
            println!(
                "removed {}",
                malt_daemon::elevate_client::HELPER_SERVICE_NAME
            );
            Ok(())
        }
    }
}

/// Relaunch exactly one explicit elevate subcommand through UAC when the
/// current process is unelevated. Returns true in the parent after the child
/// completed, so only the elevated child reaches SCM mutation.
fn relaunch_elevated_if_needed(operation: &str) -> Result<bool> {
    if malt_daemon::elevate_client::is_current_process_elevated()? {
        return Ok(false);
    }
    let executable = std::env::current_exe()?;
    let exit_code = malt_daemon::elevate_client::run_elevated(&executable, &["elevate", operation])
        .map_err(|error| {
            anyhow::anyhow!("Windows elevation was not granted; no helper change ran: {error}")
        })?;
    if exit_code != 0 {
        anyhow::bail!("elevated `malt elevate {operation}` exited with code {exit_code}");
    }
    println!("elevated `malt elevate {operation}` completed successfully");
    Ok(true)
}

fn helper_executable() -> Result<std::path::PathBuf> {
    let mut helper = std::env::current_exe()?;
    helper.set_file_name(if cfg!(windows) {
        "malt-elevate.exe"
    } else {
        "malt-elevate"
    });
    if !helper.is_file() {
        anyhow::bail!(
            "cannot install the privileged helper because {} is absent; build and deploy malt-elevate beside malt first",
            helper.display()
        );
    }
    Ok(helper)
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
        let session = client.create_session(None, None, None)?;
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

fn handle_isolation_capabilities(client: &MaltClient) -> Result<()> {
    for capability in client.isolation_capabilities()? {
        let mechanism = capability.mechanism.unwrap_or_else(|| "-".to_string());
        let detail = capability.detail.unwrap_or_default();
        println!(
            "{}: available={} basis={} mechanism={} {}",
            capability.tier, capability.available, capability.basis, mechanism, detail
        );
    }
    Ok(())
}

fn handle_new(
    client: &MaltClient,
    name: Option<&str>,
    isolation: Option<IsolationTierArg>,
    isolation_policy: Option<IsolationPolicyArg>,
) -> Result<()> {
    let expected_tier = expected_isolation_tier(isolation);
    let session = client.create_session(
        name,
        isolation.map(IsolationTierArg::request_value),
        isolation_policy.map(IsolationPolicyArg::request_value),
    )?;
    validate_created_session(&session, expected_tier)?;
    println!("{}", creation_message(&session));
    if !session
        .isolation
        .effective
        .eq_ignore_ascii_case(&session.isolation.requested)
    {
        eprintln!(
            "WARNING: requested {} isolation, received {} (basis: {}). {}",
            session.isolation.requested,
            session.isolation.effective,
            session.isolation.basis,
            session
                .isolation
                .detail
                .as_deref()
                .unwrap_or("no further detail"),
        );
    }
    Ok(())
}

fn expected_isolation_tier(isolation: Option<IsolationTierArg>) -> IsolationTierArg {
    isolation.unwrap_or(IsolationTierArg::Bare)
}

fn validate_created_session(session: &SessionData, expected_tier: IsolationTierArg) -> Result<()> {
    if session
        .isolation
        .effective
        .eq_ignore_ascii_case(expected_tier.request_value())
    {
        return Ok(());
    }

    if session
        .isolation
        .requested
        .eq_ignore_ascii_case(expected_tier.request_value())
        && session.isolation.detail.is_some()
    {
        return Ok(());
    }

    anyhow::bail!(
        "session {} was created with isolation {}, not requested {}",
        session.id,
        session.isolation.effective,
        expected_tier.display_value()
    );
}

fn creation_message(session: &SessionData) -> String {
    let mechanism = session.isolation.mechanism.as_deref().unwrap_or("none");
    format!(
        "created session {} ({}) [{}; basis: {}; mechanism: {}]",
        session.id,
        session.name.as_deref().unwrap_or("-"),
        session.isolation.effective,
        session.isolation.basis,
        mechanism,
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
    if result.truncated {
        eprintln!(
            "malt: output truncated ({} bytes omitted); use `malt watch {session_id} --output` for the full stream",
            result.omitted_bytes
        );
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

fn handle_history(client: &MaltClient, session_id: u32) -> Result<()> {
    let entries = client.get_command_history(session_id)?;
    if entries.is_empty() {
        println!("session {session_id} has no command history");
        return Ok(());
    }
    for entry in &entries {
        println!(
            "{:>5}  {}  {:>10}  {:>4}  {}",
            entry.command_id,
            format_epoch_ms(entry.started_at),
            format_duration(entry.started_at, entry.finished_at),
            entry
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string()),
            entry.cmd,
        );
    }
    Ok(())
}

/// Render an epoch-ms timestamp as local wall-clock `HH:MM:SS`.
fn format_epoch_ms(ms: u64) -> String {
    let secs_of_day = (ms / 1000) % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Elapsed time between start and finish, or a marker when the command never
/// reported completion. Deliberately does not guess: an unfinished record is
/// shown as such rather than as a duration up to "now".
fn format_duration(started_at: u64, finished_at: Option<u64>) -> String {
    match finished_at {
        Some(end) => {
            let ms = end.saturating_sub(started_at);
            if ms < 1000 {
                format!("{ms}ms")
            } else {
                format!("{:.1}s", ms as f64 / 1000.0)
            }
        }
        None => "incomplete".to_string(),
    }
}

fn handle_watch(client: &MaltClient, session_id: u32, resume_from: Option<u64>) -> Result<()> {
    println!("watching session {session_id} (ctrl-c to stop)");
    events::watch_events(client, session_id, resume_from, |event| {
        match event.kind.as_str() {
            "command_started" => println!(
                "{:>5}  started   {:>5}  {}",
                event.sequence,
                event
                    .payload
                    .command_id
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                event.payload.cmd.as_deref().unwrap_or(""),
            ),
            "command_finished" => println!(
                "{:>5}  finished  {:>5}  exit {}  {}",
                event.sequence,
                event
                    .payload
                    .command_id
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                event
                    .payload
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                format_us(event.payload.duration_us),
            ),
            // A gap means this client's view is incomplete. Say so loudly
            // rather than letting it scroll past as one more line.
            "gap" => println!(
                "\n  !! GAP: missed events {}..={} ({}) -- this view is incomplete\n",
                event.payload.missed_from.unwrap_or(0),
                event.payload.missed_through.unwrap_or(0),
                event.payload.reason.as_deref().unwrap_or("unknown reason"),
            ),
            other => println!("{:>5}  {other}", event.sequence),
        }
        events::ControlFlow::Continue
    })
}

/// As `handle_watch`, but for the output-chunk stream (`--output`).
///
/// Output bytes go to stdout exactly as received -- decoded from base64,
/// never through a text formatter, since the command's output may not be
/// valid UTF-8 (research R6). Status text (the startup banner, gap notices)
/// goes to stderr so a caller piping stdout gets only the command's real
/// output.
fn handle_watch_output(
    client: &MaltClient,
    session_id: u32,
    resume_from: Option<u64>,
) -> Result<()> {
    use base64::Engine;
    use std::io::Write;

    eprintln!("watching session {session_id} output (ctrl-c to stop)");
    let mut stdout = std::io::stdout();
    events::watch_output(client, session_id, resume_from, |event| {
        match event.kind.as_str() {
            "output" => {
                if let Some(encoded) = &event.payload.data {
                    match base64::engine::general_purpose::STANDARD.decode(encoded) {
                        Ok(bytes) => {
                            let _ = stdout.write_all(&bytes);
                            let _ = stdout.flush();
                        }
                        Err(error) => {
                            eprintln!("malt: watch --output: undecodable chunk: {error}");
                        }
                    }
                }
            }
            "gap" => {
                eprintln!(
                    "\n  !! GAP: missed output {}..={} ({}) -- this view is incomplete\n",
                    event.payload.from.unwrap_or(0),
                    event.payload.to.unwrap_or(0),
                    event.payload.reason.as_deref().unwrap_or("unknown reason"),
                );
            }
            _ => {}
        }
        events::ControlFlow::Continue
    })
}

/// Render a microsecond duration compactly, or a dash when absent.
fn format_us(duration_us: Option<u64>) -> String {
    match duration_us {
        Some(us) if us < 1_000 => format!("{us}us"),
        Some(us) if us < 1_000_000 => format!("{}ms", us / 1_000),
        Some(us) => format!("{:.1}s", us as f64 / 1_000_000.0),
        None => "-".to_string(),
    }
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
            isolation: crate::client::IsolationData {
                effective: isolation.to_string(),
                requested: isolation.to_string(),
                basis: "none".to_string(),
                mechanism: None,
                detail: None,
            },
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
                format!(
                    "created session 42 (build) [{}; basis: none; mechanism: none]",
                    tier.display_value()
                )
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
