//! `malt-elevate` — elevated helper binary for MALT.
//!
//! This binary runs with admin/root privileges and exposes a fixed set of
//! privileged operations over a local IPC channel. Authentication uses a
//! nonce read from a restricted file.
//!
//! Phase 2: minimal skeleton that parses arguments and validates the nonce
//! file. Full socket communication arrives in Phase 3 when the daemon needs
//! to talk to the helper.

use std::path::PathBuf;
use std::process::ExitCode;

use malt_elevate::auth::NonceAuth;
use malt_elevate::error::ElevateError;

/// Command-line arguments for the elevated helper.
struct Args {
    nonce_file: PathBuf,
    socket_path: PathBuf,
}

fn parse_args() -> Result<Args, ElevateError> {
    let mut args = std::env::args_os().skip(1);
    let mut nonce_file: Option<PathBuf> = None;
    let mut socket_path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--nonce-file") => {
                nonce_file = Some(
                    args.next()
                        .ok_or_else(|| ElevateError::InvalidArg("--nonce-file requires a value".into()))?
                        .into(),
                );
            }
            Some("--socket") => {
                socket_path = Some(
                    args.next()
                        .ok_or_else(|| ElevateError::InvalidArg("--socket requires a value".into()))?
                        .into(),
                );
            }
            Some("--help" | "-h") => {
                print_usage();
                std::process::exit(0);
            }
            _ => {
                return Err(ElevateError::InvalidArg(format!(
                    "unknown argument: {}",
                    arg.to_string_lossy()
                )));
            }
        }
    }

    let nonce_file = nonce_file
        .ok_or_else(|| ElevateError::InvalidArg("--nonce-file is required".into()))?;
    let socket_path = socket_path
        .ok_or_else(|| ElevateError::InvalidArg("--socket is required".into()))?;

    Ok(Args { nonce_file, socket_path })
}

fn print_usage() {
    eprintln!(
        "\
malt-elevate — elevated helper binary for MALT

USAGE:
    malt-elevate --nonce-file <PATH> --socket <PATH>

OPTIONS:
    --nonce-file <PATH>    Path to the 8-byte nonce file (required)
    --socket <PATH>        Path to the IPC socket/named pipe (required)
    -h, --help             Print this help message

The helper reads the nonce from <nonce-file>, listens on <socket> for a
single connection from the MALT daemon, validates the ElevateHello nonce,
then dispatches privileged operation requests until shutdown."
    );
}

fn run() -> Result<(), ElevateError> {
    let args = parse_args()?;

    // Validate the nonce file is readable and well-formed.
    let _auth = NonceAuth::from_file(&args.nonce_file)?;

    // Phase 2: print status and exit. Full IPC loop comes in Phase 3.
    eprintln!(
        "malt-elevate: nonce loaded from {:?}, socket {:?}",
        args.nonce_file, args.socket_path
    );
    eprintln!("malt-elevate: Phase 2 skeleton — IPC loop not yet implemented");

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("malt-elevate: error: {e}");
            ExitCode::FAILURE
        }
    }
}
