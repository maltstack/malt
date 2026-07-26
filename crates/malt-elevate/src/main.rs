//! Privileged helper service entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use malt_elevate::auth::NonceAuth;
use malt_elevate::error::ElevateError;
use malt_elevate::server::{serve, ServerConfig};

struct Args {
    nonce_file: PathBuf,
    pipe_name: String,
    authorized_pid: u32,
}

fn parse_args() -> Result<Args, ElevateError> {
    let mut args = std::env::args_os().skip(1);
    let mut nonce_file = None;
    let mut pipe_name = None;
    let mut authorized_pid = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--nonce-file") => {
                nonce_file = Some(next_value(&mut args, "--nonce-file")?.into())
            }
            Some("--pipe") => {
                pipe_name = Some(
                    next_value(&mut args, "--pipe")?
                        .to_string_lossy()
                        .into_owned(),
                )
            }
            Some("--authorized-pid") => {
                let value = next_value(&mut args, "--authorized-pid")?;
                authorized_pid = Some(value.to_string_lossy().parse().map_err(|_| {
                    ElevateError::InvalidArg("--authorized-pid must be a process id".into())
                })?);
            }
            Some("--help" | "-h") => {
                print_usage();
                std::process::exit(0);
            }
            _ => {
                return Err(ElevateError::InvalidArg(format!(
                    "unknown argument: {}",
                    arg.to_string_lossy()
                )))
            }
        }
    }
    Ok(Args {
        nonce_file: nonce_file
            .ok_or_else(|| ElevateError::InvalidArg("--nonce-file is required".into()))?,
        pipe_name: pipe_name
            .ok_or_else(|| ElevateError::InvalidArg("--pipe is required".into()))?,
        authorized_pid: authorized_pid
            .ok_or_else(|| ElevateError::InvalidArg("--authorized-pid is required".into()))?,
    })
}

fn next_value(
    args: &mut impl Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<std::ffi::OsString, ElevateError> {
    args.next()
        .ok_or_else(|| ElevateError::InvalidArg(format!("{flag} requires a value")))
}

fn print_usage() {
    eprintln!("malt-elevate --nonce-file <PATH> --pipe <NAME> --authorized-pid <PID>");
}

fn run() -> Result<(), ElevateError> {
    let args = parse_args()?;
    // The shared secret remains a defence-in-depth installation check; pipe
    // peer identity is the primary authentication decision in `serve`.
    let _nonce_auth = NonceAuth::from_file(&args.nonce_file)?;
    serve(&ServerConfig {
        pipe_name: args.pipe_name,
        authorized_process_id: args.authorized_pid,
        replay_capacity: 4096,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("malt-elevate: error: {error}");
            ExitCode::FAILURE
        }
    }
}
