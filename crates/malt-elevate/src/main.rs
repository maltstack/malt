//! Privileged helper service entry point.

use std::process::ExitCode;

use malt_elevate::error::ElevateError;
use malt_elevate::server::{serve, ServerConfig};
use malt_platform::service::run_service;

pub const SERVICE_NAME: &str = "MALT-Elevate";

struct Args {
    service: bool,
    pipe_name: String,
    authorized_principal: String,
}

fn parse_args() -> Result<Args, ElevateError> {
    let mut args = std::env::args_os().skip(1);
    let mut service = false;
    let mut pipe_name = None;
    let mut authorized_principal = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--service") => service = true,
            Some("--pipe") => {
                pipe_name = Some(
                    next_value(&mut args, "--pipe")?
                        .to_string_lossy()
                        .into_owned(),
                )
            }
            Some("--authorized-principal") => {
                authorized_principal = Some(
                    next_value(&mut args, "--authorized-principal")?
                        .to_string_lossy()
                        .into_owned(),
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
                )))
            }
        }
    }
    if !service {
        return Err(ElevateError::InvalidArg(
            "malt-elevate is a service host and must be launched with --service".into(),
        ));
    }
    Ok(Args {
        service,
        pipe_name: pipe_name
            .ok_or_else(|| ElevateError::InvalidArg("--pipe is required".into()))?,
        authorized_principal: authorized_principal
            .ok_or_else(|| ElevateError::InvalidArg("--authorized-principal is required".into()))?,
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
    eprintln!("malt-elevate --service --pipe <NAME> --authorized-principal <SID>");
}

fn run() -> Result<(), ElevateError> {
    let args = parse_args()?;
    if !args.service {
        return Err(ElevateError::InvalidArg("--service is required".into()));
    }
    let config = ServerConfig {
        pipe_name: args.pipe_name,
        authorized_principal: args.authorized_principal,
        replay_capacity: 4096,
    };
    let pipe_name = config.pipe_name.clone();
    run_service(SERVICE_NAME, Some(&pipe_name), move |stop| {
        serve(&config, stop).map_err(std::io::Error::other)
    })
    .map_err(ElevateError::Connection)
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
