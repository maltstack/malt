use clap::{Parser, Subcommand, ValueEnum};

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
        /// Isolation tier for processes started in this session
        #[arg(long, value_enum)]
        isolation: Option<IsolationTierArg>,
    },
    /// Attach to a session
    Attach { session_id: Option<u32> },
    /// Kill a session
    Kill { session_id: u32 },
    /// Execute a command in a session
    Exec { session_id: u32, command: String },
    /// Send raw input to a session
    Send { session_id: u32, input: String },
    /// Print a session's current output as plain text
    Output { session_id: u32 },
    /// List a session's command execution history, oldest first
    History { session_id: u32 },
}

/// Canonical command-line spellings for the session isolation tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum IsolationTierArg {
    Bare,
    Restricted,
    Capped,
    Contained,
}

impl IsolationTierArg {
    /// Returns the lower-case value accepted by the session-creation service.
    pub const fn request_value(self) -> &'static str {
        match self {
            Self::Bare => "bare",
            Self::Restricted => "restricted",
            Self::Capped => "capped",
            Self::Contained => "contained",
        }
    }

    /// Returns the canonical title-case label used in session summaries.
    pub const fn display_value(self) -> &'static str {
        match self {
            Self::Bare => "Bare",
            Self::Restricted => "Restricted",
            Self::Capped => "Capped",
            Self::Contained => "Contained",
        }
    }
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
            Some(Command::New { name, isolation }) => {
                assert_eq!(name, Some("foo".to_string()));
                assert_eq!(isolation, None);
            }
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn parse_new_with_each_canonical_isolation_tier() {
        for (value, expected) in [
            ("bare", IsolationTierArg::Bare),
            ("restricted", IsolationTierArg::Restricted),
            ("capped", IsolationTierArg::Capped),
            ("contained", IsolationTierArg::Contained),
        ] {
            let cli = Cli::try_parse_from(["malt", "new", "--isolation", value]).unwrap();
            match cli.command {
                Some(Command::New { name, isolation }) => {
                    assert_eq!(name, None);
                    assert_eq!(isolation, Some(expected));
                }
                other => panic!("expected New, got {other:?}"),
            }
        }
    }

    #[test]
    fn new_rejects_missing_or_invalid_isolation_value() {
        assert!(Cli::try_parse_from(["malt", "new", "--isolation"]).is_err());
        assert!(Cli::try_parse_from(["malt", "new", "--isolation", "isolated"]).is_err());
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
