mod cli;
mod client;
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
        Some(Command::Start) => handle_start(),
        Some(Command::Stop) => handle_stop(),
        Some(Command::List) => handle_list(&client),
        Some(Command::New { name }) => handle_new(&client, name.as_deref()),
        Some(Command::Attach { session_id }) => handle_attach(session_id),
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
    println!("start: not yet implemented");
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

fn handle_attach(session_id: Option<u32>) -> Result<()> {
    match session_id {
        Some(id) => println!("attach {id}: not yet implemented"),
        None => println!("attach: not yet implemented"),
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
