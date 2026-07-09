use std::io::{self, Read, Write};
use std::path::Path;
use std::process;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use quonfmt::{FormatError, check_str, format_str};

#[derive(Parser)]
#[command(
    name = "quonfmt",
    about = "Canonical formatter for Quon source (.qn). Comments are stripped on format.",
    version
)]
struct Cli {
    /// Exit with status 1 if any file would change (no writes).
    #[arg(short, long)]
    check: bool,

    /// Write formatted output back to files in place.
    #[arg(short, long)]
    write: bool,

    /// Source files to format (reads stdin when omitted).
    #[arg(value_name = "FILE")]
    files: Vec<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.check && cli.write {
        bail!("--check and --write are mutually exclusive");
    }

    if cli.files.is_empty() {
        let mut src = String::new();
        io::stdin()
            .read_to_string(&mut src)
            .context("reading stdin")?;
        emit(&cli, &src, Path::new("-"))?;
        return Ok(());
    }

    let mut exit_code = 0;
    for path in &cli.files {
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if let Err(code) = emit_with_status(&cli, &src, path) {
            exit_code = exit_code.max(code);
        }
    }

    if exit_code != 0 {
        process::exit(exit_code);
    }
    Ok(())
}

/// Returns `Ok(())` on success, `Err(exit_code)` for check/format failures.
fn emit_with_status(cli: &Cli, src: &str, path: &Path) -> std::result::Result<(), i32> {
    if cli.check {
        return check_str(src).map_err(|e| match e {
            FormatError::Parse { diagnostics } => {
                report_parse_error(path, src, &diagnostics);
                2
            }
            FormatError::NotFormatted { .. } => {
                eprintln!("{}: would reformat", path.display());
                1
            }
        });
    }

    let formatted = match format_str(src) {
        Ok(f) => f,
        Err(FormatError::Parse { diagnostics }) => {
            report_parse_error(path, src, &diagnostics);
            return Err(2);
        }
        Err(e) => {
            return Err(match e {
                FormatError::NotFormatted { .. } => 1,
                FormatError::Parse { .. } => 2,
            });
        }
    };

    if cli.write {
        if path.as_os_str() != "-" {
            if std::fs::write(path, &formatted).is_err() {
                return Err(2);
            }
        } else if io::stdout().write_all(formatted.as_bytes()).is_err() {
            return Err(2);
        }
        return Ok(());
    }

    print!("{formatted}");
    Ok(())
}

fn emit(cli: &Cli, src: &str, path: &Path) -> Result<()> {
    emit_with_status(cli, src, path).map_err(|code| anyhow::anyhow!("exit code {code}"))
}

fn report_parse_error(path: &Path, src: &str, diagnostics: &[frontend::diagnostics::Diagnostic]) {
    use ariadne::{Color, Label, Report, ReportKind, Source};

    let id = path.display().to_string();
    for diag in diagnostics {
        let _ = Report::build(ReportKind::Error, id.clone(), diag.span.start)
            .with_message(&diag.message)
            .with_label(
                Label::new((id.clone(), diag.span.start..diag.span.end))
                    .with_message(&diag.message)
                    .with_color(Color::Red),
            )
            .finish()
            .eprint((id.clone(), Source::from(src)));
    }
}
