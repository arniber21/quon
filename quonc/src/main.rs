use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use quon_core::{
    RegressionConfig, compare, format_comparison_table, format_metrics_line, load_snapshot,
    save_snapshot,
};

use quonc::compile::{CompileRequest, compile};
use quonc::watch::{print_watch_metrics, run_watch_loop};

#[derive(Parser)]
#[command(name = "quonc", about = "Quon quantum compiler", version)]
struct Cli {
    /// Source file to compile (.qn)
    source: PathBuf,

    /// Emit OpenQASM 3.0 to stdout
    #[arg(long)]
    emit_qasm: bool,

    /// Backend target descriptor (JSON). Defaults to generic_openqasm.
    #[arg(long)]
    target: Option<PathBuf>,

    /// Dump MLIR after each pass (debug)
    #[arg(long)]
    dump_ir: bool,

    /// Run the linearity verifier pass (debug)
    #[arg(long)]
    verify_linear: bool,

    /// Print a human-readable metrics summary to stderr after a successful compile
    #[arg(long)]
    metrics: bool,

    /// Write metrics JSON to file (`-` for stdout/stderr — see `--emit-qasm`)
    #[arg(long, value_name = "PATH")]
    metrics_json: Option<String>,

    /// Save or compare a metrics snapshot baseline (`save PATH` or `compare PATH`)
    #[arg(long, num_args = 2, value_names = ["ACTION", "PATH"])]
    metrics_snapshot: Option<Vec<String>>,

    /// TOML/JSON tolerance file for `--metrics-snapshot compare`
    #[arg(long, value_name = "PATH")]
    regression_config: Option<PathBuf>,

    /// Watch source (and `--target` if set) for changes; recompile on change
    #[arg(long)]
    watch: bool,

    /// Debounce window for filesystem events in watch mode (milliseconds)
    #[arg(long, default_value_t = 300)]
    watch_debounce_ms: u64,
}

#[derive(Clone, Copy, Debug)]
enum SnapshotAction {
    Save,
    Compare,
}

fn parse_snapshot_action(action: &str) -> Result<SnapshotAction> {
    match action.to_ascii_lowercase().as_str() {
        "save" => Ok(SnapshotAction::Save),
        "compare" => Ok(SnapshotAction::Compare),
        other => Err(anyhow::anyhow!(
            "unknown metrics-snapshot action `{other}` (expected save or compare)"
        )),
    }
}

fn snapshot_from_parts(parts: &[String]) -> Result<(SnapshotAction, PathBuf)> {
    let [action, path] = parts else {
        bail!("--metrics-snapshot requires ACTION and PATH");
    };
    Ok((parse_snapshot_action(action)?, PathBuf::from(path)))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let mut cli = Cli::parse();

    if cli.watch && !cli.metrics {
        cli.metrics = true;
    }

    if cli.watch {
        return run_watch(&cli);
    }

    let request = build_request(&cli)?;
    let report = compile(&request);

    if report.snapshot.compile.status != quon_core::CompileStatus::Ok {
        if let Some(err) = &report.snapshot.compile.error {
            eprintln!("{err}");
        }
        return Ok(ExitCode::from(1));
    }

    if cli.emit_qasm {
        if let Some(qasm) = &report.qasm {
            print!("{qasm}");
        }
    } else if cli.metrics_json.is_none() && cli.metrics_snapshot.is_none() && !cli.metrics {
        eprintln!("(compiled successfully; pass --emit-qasm to print OpenQASM 3.0)");
    }

    if cli.metrics {
        eprintln!("{}", format_metrics_line(&report.snapshot));
    }

    if let Some(path) = &cli.metrics_json {
        write_metrics_json(path, &report, cli.emit_qasm)?;
    }

    if let Some(parts) = &cli.metrics_snapshot {
        let (action, path) = snapshot_from_parts(parts)?;
        match action {
            SnapshotAction::Save => {
                save_snapshot(&path, &report.snapshot).map_err(|e| anyhow::anyhow!("{e}"))?;
                eprintln!("saved snapshot → {}", path.display());
            }
            SnapshotAction::Compare => {
                let baseline = load_snapshot(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
                let config = load_regression_config(cli.regression_config.as_ref())?;
                let comparison = compare(&baseline, &report.snapshot, &config)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                eprintln!(
                    "{}",
                    format_comparison_table(&baseline, &report.snapshot, &comparison, &config)
                );
                if !comparison.passed {
                    return Ok(ExitCode::from(1));
                }
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn run_watch(cli: &Cli) -> Result<ExitCode> {
    let source = cli.source.clone();
    let target = cli.target.clone();
    let debounce = cli.watch_debounce_ms;
    let metrics_json = cli.metrics_json.clone();
    let emit_qasm = cli.emit_qasm;
    let snapshot_compare = cli
        .metrics_snapshot
        .as_ref()
        .and_then(|parts| snapshot_from_parts(parts).ok())
        .and_then(|(action, path)| {
            if matches!(action, SnapshotAction::Compare) {
                Some(path)
            } else {
                None
            }
        });
    let regression_config_path = cli.regression_config.clone();
    let print_metrics = cli.metrics;

    let regression = snapshot_compare.map(|baseline_path| {
        let config = load_regression_config(regression_config_path.as_ref()).unwrap_or_default();
        (baseline_path, config)
    });

    let mut sticky_fail = false;

    run_watch_loop(
        source.clone(),
        target.clone(),
        debounce,
        || build_request(cli),
        regression,
        |report, previous, comparison| {
            if print_metrics {
                print_watch_metrics(report, previous, comparison);
            }
            if let Some(path) = &metrics_json {
                let _ = write_metrics_json(path, report, emit_qasm);
            }
            if let Some(cmp) = comparison
                && !cmp.passed
            {
                sticky_fail = true;
            }
        },
    )?;

    if sticky_fail {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn build_request(cli: &Cli) -> Result<CompileRequest> {
    let source = std::fs::read_to_string(&cli.source)
        .with_context(|| format!("reading {}", cli.source.display()))?;

    let target = match &cli.target {
        Some(path) => backend::json::load(path)
            .map_err(|e| anyhow::anyhow!("loading target {}: {e}", path.display()))?,
        None => backend::generic_openqasm::target(64),
    };

    Ok(CompileRequest {
        source_path: cli.source.clone(),
        source,
        target,
        target_descriptor_path: cli.target.clone(),
        dump_ir: cli.dump_ir,
        verify_linear: cli.verify_linear,
    })
}

fn load_regression_config(path: Option<&PathBuf>) -> Result<RegressionConfig> {
    match path {
        Some(p) => RegressionConfig::load(p).map_err(|e| anyhow::anyhow!("{e}")),
        None => Ok(RegressionConfig::default()),
    }
}

fn write_metrics_json(path: &str, report: &quonc::CompileReport, emit_qasm: bool) -> Result<()> {
    let json = serde_json::to_string_pretty(&report.snapshot)?;
    if path == "-" {
        if emit_qasm {
            eprintln!("{json}");
        } else {
            io::stdout().write_all(json.as_bytes())?;
            io::stdout().write_all(b"\n")?;
        }
    } else {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{json}\n"))?;
    }
    Ok(())
}
