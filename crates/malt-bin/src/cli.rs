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
    /// Run the daemon (foreground)
    Daemon {
        /// Port to listen on
        #[arg(long, default_value = "7700")]
        port: u16,
    },
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
    Attach { session_id: Option<u32> },
    /// Kill a session
    Kill { session_id: u32 },
    /// Execute a command in a session
    Exec { session_id: u32, command: String },
    /// Send raw input to a session
    Send { session_id: u32, input: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_no_args() {
        let cli = Cli::try_parse_from(["malt"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parse_list() {
        let cli = Cli::try_parse_from(["malt", "list"]).unwrap();
        assert!(matches!(cli.command, Some(Command::List)));
    }

    #[test]
    fn parse_new_with_name() {
        let cli = Cli::try_parse_from(["malt", "new", "--name", "foo"]).unwrap();
        match cli.command {
            Some(Command::New { name }) => assert_eq!(name, Some("foo".to_string())),
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_command() {
        let cli = Cli::try_parse_from(["malt", "kill", "42"]).unwrap();
        match cli.command {
            Some(Command::Kill { session_id }) => assert_eq!(session_id, 42),
            other => panic!("expected Kill, got {other:?}"),
        }
    }
}
