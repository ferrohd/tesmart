//! `sim serve`: an emulated switch on a socket, for testing anything that speaks the protocol.

use tesmart::proto::model;
use tesmart::sim::Faults;
use tesmart::sim::Simulator;

use crate::cli::SimServeArgs;
use crate::error::CliError;
use crate::render::Printer;
use crate::report::{DoneView, Report};

/// Serves an emulated switch until interrupted.
///
/// # Errors
///
/// [`CliError::Usage`] for an unknown model or an unparsable terminator, [`CliError::Io`] if
/// the address cannot be bound.
pub async fn serve(args: &SimServeArgs, printer: &Printer) -> Result<Report, CliError> {
    let model = model::lookup(&args.model).ok_or_else(|| {
        CliError::usage(format!("unknown model {:?}; see `tesmart models list`", args.model))
    })?;

    let simulator = Simulator::from_model(model);
    simulator.handle().set_faults(faults(args)?);

    let (listener, addr) = Simulator::bind(args.listen)
        .await
        .map_err(|error| CliError::io(format!("cannot listen on {}", args.listen), error))?;

    printer.note(&format!(
        "emulating {} on tcp://{addr}, {} ports; press Ctrl-C to stop",
        model.key, model.ports
    ));

    tokio::select! {
        result = simulator.serve(listener) => {
            result.map_err(|error| CliError::io("the emulator stopped", error))?;
        }
        _interrupted = tokio::signal::ctrl_c() => {}
    }

    let handle = simulator.handle();
    Ok(Report::Done(DoneView::new(format!(
        "served {} connection(s); the emulated switch is on port {}",
        handle.connections(),
        handle.snapshot().active_port
    ))))
}

/// How the emulated switch should misbehave.
///
/// The default is how hardware actually behaves: address replies arrive in two segments.
fn faults(args: &SimServeArgs) -> Result<Faults, CliError> {
    let mut faults = if args.perfect { Faults::NONE } else { Faults::REALISTIC };

    if let Some(every) = args.drop_every {
        faults.drop_every = core::num::NonZeroU8::new(every);
        if faults.drop_every.is_none() {
            return Err(CliError::usage("--drop-every must be at least 1"));
        }
    }
    if let Some(terminator) = &args.bad_terminator {
        let text = terminator.trim_start_matches("0x").trim_start_matches("0X");
        let byte = u8::from_str_radix(text, 16).map_err(|_| {
            CliError::usage(format!("--bad-terminator {terminator:?} is not a hex byte"))
        })?;
        faults.terminator = Some(byte);
    }
    faults.ignore_switch = args.ignore_switch;

    Ok(faults)
}
