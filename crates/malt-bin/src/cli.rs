use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "malt", about = "MALT terminal platform", version)]
pub struct Cli {
    #[arg(long, env = "MALT_API_ADDR", default_value = "http://127.0.0.1:7700")]
    pub api_addr: String,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the MALT daemon
    Start,
    /// Stop the MALT daemon
    Stop,
    /// Show daemon status
    Status,
    /// List all sessions
    List,
    /// Create a new session
    New {
        #[arg(long)]
        name: Option<String>,
    },
    /// Attach to a session
    Attach {
        session_id: Option<u32>,
    },
    /// Kill a session
    Kill {
        session_id: u32,
    },
    /// Execute a command in a session
    Exec {
        session_id: u32,
        command: String,
    },
    /// Send raw input to a session
    Send {
        session_id: u32,
        input: String,
    },
}
