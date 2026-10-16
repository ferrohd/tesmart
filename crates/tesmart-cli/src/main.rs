//! `tesmart`: control TESmart KVM switches from the command line.
//!
//! The binary is a thin layer over the [`tesmart`] library: it parses arguments, resolves a
//! target, calls the operations and renders what comes back. What it adds is honesty at the
//! edges — a command that was sent but cannot be confirmed reads differently from one that was,
//! both in the terminal and in `--output json`, and the exit code says which.
//!
//! Everything protocol-shaped comes through the facade, down to the transports themselves, so
//! this crate is also the first consumer to notice if the library is missing something. The one
//! thing it keeps for itself is [`target`]: deciding whether `/dev/ttyUSB0` means a serial port
//! only makes sense where there is a user to have typed it.

use std::process::ExitCode;

use clap::Parser as _;
use tracing_subscriber::EnvFilter;

mod cli;
mod error;
mod generate;
mod handle;
mod render;
mod report;
mod run;
mod simulate;
mod target;

use crate::cli::{Cli, ColorChoice, Global};
use crate::error::CliError;
use crate::render::Printer;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(&cli.global);

    let printer = Printer::new(cli.global.output, cli.global.color, cli.global.quiet);
    let (context, result) = run::execute(&cli, &printer).await;

    match result {
        Ok(report) => {
            // `--require-verified` turns "sent, but nobody can say what happened" into a
            // failure, for scripts that need better than that.
            if cli.global.require_verified && report.confirmed() == Some(false) {
                let reason = report.unverified_reason().unwrap_or("not verified");
                let error = CliError::Unverified(format!(
                    "--require-verified was given, and the result could not be confirmed: {reason}"
                ));
                printer.failure(&context, &error);
                return error.exit_code();
            }

            printer.success(&context, &report);
            ExitCode::SUCCESS
        }
        Err(error) => {
            printer.failure(&context, &error);
            error.exit_code()
        }
    }
}

/// Logs go to standard error so that `--output json` can be piped at any verbosity.
///
/// Timestamps are deliberately omitted: they make snapshot tests unstable and a log consumer
/// adds better ones.
fn init_tracing(global: &Global) {
    let filter = match std::env::var("TESMART_LOG") {
        Ok(directive) if !directive.trim().is_empty() => EnvFilter::new(directive),
        _ => EnvFilter::new(match global.verbose {
            0 => "warn",
            1 => "info",
            // -vv turns on the wire hexdumps, which is the whole point of having them.
            2 => "debug",
            _ => "trace",
        }),
    };

    let ansi = match global.color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            anstream::AutoStream::choice(&std::io::stderr()) != anstream::ColorChoice::Never
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(ansi)
        .without_time()
        .init();
}
