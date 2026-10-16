//! Shell completions and the manual page.
//!
//! Generated on demand rather than in a build script: it keeps builds fast, and it means the
//! completions always match the binary that printed them.

use std::io;
use std::path::Path;

use clap::CommandFactory as _;
use clap_complete::Shell;

use crate::cli::Cli;
use crate::error::CliError;
use crate::report::{DoneView, Report};

/// Writes a completion script for `shell` to standard output.
pub fn completions(shell: Shell) -> Report {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut io::stdout());

    Report::Done(DoneView::new(format!("{shell} completions written to stdout")))
}

/// Writes the manual page, to a directory or to standard output.
///
/// # Errors
///
/// [`CliError::Io`] if the page cannot be rendered or written.
pub fn manpage(out: Option<&Path>) -> Result<Report, CliError> {
    let page = clap_mangen::Man::new(Cli::command());
    let mut roff = Vec::new();
    page.render(&mut roff).map_err(|error| CliError::io("cannot render the manual page", error))?;

    let Some(directory) = out else {
        io::Write::write_all(&mut io::stdout(), &roff)
            .map_err(|error| CliError::io("cannot write the manual page", error))?;
        return Ok(Report::Done(DoneView::new("manual page written to stdout")));
    };

    let path = directory.join("tesmart.1");
    std::fs::write(&path, roff)
        .map_err(|error| CliError::io(format!("cannot write {}", path.display()), error))?;

    Ok(Report::Done(DoneView::new(format!("manual page written to {}", path.display()))))
}
