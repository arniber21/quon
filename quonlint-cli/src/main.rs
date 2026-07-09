use std::io;
use std::path::PathBuf;
use std::process;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, ValueEnum};
use quonlint::{
    LintConfig, Severity, has_failures, lint_paths, lint_project, register_rules, report_github,
    report_human, write_json,
};

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Github,
}

#[derive(Parser)]
#[command(
    name = "quonlint",
    about = "Lint Quon source for experiment quality",
    version
)]
struct Cli {
    /// Lint all `.qn` files under project root (discover quonlint.toml).
    #[arg(long)]
    project: Option<Option<PathBuf>>,

    /// Config file path.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Minimum severity to emit (allow | info | warn | error).
    #[arg(long, value_name = "LEVEL")]
    max_severity: Option<String>,

    /// Exit with code 1 when diagnostics at or above this severity exist.
    #[arg(long, value_name = "LEVEL", default_value = "error")]
    fail_on: String,

    /// Only run these rules (comma-separated ids).
    #[arg(long, value_name = "RULES")]
    only: Option<String>,

    /// Skip these rules (comma-separated ids).
    #[arg(long, value_name = "RULES")]
    except: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,

    /// Run IR-heavy analysis (native gate decomposition).
    #[arg(long)]
    deep: bool,

    /// List registered rules and exit.
    #[arg(long)]
    list_rules: bool,

    /// Source files or directories.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.list_rules {
        for rule in register_rules() {
            println!(
                "{} [{}] — {}",
                rule.id(),
                rule.default_severity(),
                rule.description()
            );
        }
        return Ok(());
    }

    let mut config = load_config(&cli)?;
    apply_cli_overrides(&mut config, &cli)?;

    let results = if let Some(project_root) = cli.project {
        let root = project_root.unwrap_or_else(|| PathBuf::from("."));
        lint_project(&root, &config).context("project lint")?
    } else if cli.paths.is_empty() {
        bail!("provide PATH or use --project");
    } else {
        lint_paths(&cli.paths, &config).context("lint paths")?
    };

    let fail_on = parse_severity(&cli.fail_on)?;
    let mut failed = false;

    for (path, diags) in &results {
        match cli.format {
            OutputFormat::Human => {
                let src = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                report_human(path, &src, diags)?;
            }
            OutputFormat::Json => {
                write_json(&mut io::stdout(), diags)?;
            }
            OutputFormat::Github => {
                report_github(path, diags);
            }
        }
        if has_failures(diags, fail_on) {
            failed = true;
        }
    }

    if failed {
        process::exit(1);
    }
    Ok(())
}

fn load_config(cli: &Cli) -> Result<LintConfig> {
    if let Some(path) = &cli.config {
        LintConfig::load(path).context("loading config")
    } else if cli.project.is_some() {
        let root = cli
            .project
            .as_ref()
            .and_then(|o| o.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(LintConfig::discover_project(&root))
    } else if let Some(first) = cli.paths.first() {
        Ok(LintConfig::discover_for_file(first))
    } else {
        Ok(LintConfig::default())
    }
}

fn apply_cli_overrides(config: &mut LintConfig, cli: &Cli) -> Result<()> {
    if let Some(max) = &cli.max_severity {
        config.min_severity = parse_severity(max)?;
    }
    config.fail_on = parse_severity(&cli.fail_on)?;
    if cli.deep {
        config.deep = true;
    }
    if let Some(only) = &cli.only {
        config.only_rules = Some(parse_rule_list(only)?);
    }
    if let Some(except) = &cli.except {
        config.disabled_rules.extend(parse_rule_list(except)?);
    }
    Ok(())
}

fn parse_severity(s: &str) -> Result<Severity> {
    Severity::from_str(s).map_err(|()| anyhow::anyhow!("unknown severity `{s}`"))
}

fn parse_rule_list(s: &str) -> Result<Vec<String>> {
    Ok(s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect())
}
