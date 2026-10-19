//! `jumbo`: developer CLI over jumbo-core and jumbo-ledger, for scripting and debugging projects
//! without the desktop app.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use jumbo_core::{ChapterId, ChapterState, Project, chapter};
use jumbo_ledger::{BookError, EntityType, MemoryDelta, ProjectPanelIndex, ReceiptBook, ValidationReport};

#[derive(Parser)]
#[command(name = "jumbo", version, about = "Project Jumbo developer CLI")]
struct Cli {
    /// Project root folder.
    #[arg(short = 'C', long, global = true, default_value = ".")]
    project: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new project in the folder.
    Init { title: String },
    /// Show the project manifest and Receipt Book status.
    Info,
    /// Import a chapter PDF.
    Import {
        pdf: PathBuf,
        #[arg(long)]
        title: Option<String>,
    },
    /// List chapters and their pipeline state.
    Chapters,
    /// Move a chapter to the next stage.
    Advance { chapter: ChapterId, to: String },
    /// Send a chapter back to an earlier stage.
    Rewind { chapter: ChapterId, to: String },
    /// Validate a MemoryDelta JSON file without committing it.
    Check { delta: PathBuf },
    /// Validate and commit a MemoryDelta JSON file.
    Commit {
        delta: PathBuf,
        #[arg(long)]
        session: Option<String>,
    },
    /// Unlock the Receipt Book for a new working session.
    BeginSession,
    /// Snapshot and lock the Receipt Book (the Stop / Consolidate step).
    Snapshot {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Verify ledger, snapshots and lock.
    Verify,
    /// List entities in the Receipt Book.
    Entities {
        #[arg(long = "type")]
        entity_type: Option<String>,
    },
    /// Rewrite receipt_book/state from the ledger.
    RebuildViews,
}

fn parse_state(s: &str) -> Result<ChapterState, String> {
    ChapterState::ALL.into_iter().find(|st| st.as_str() == s).ok_or_else(|| {
        format!("unknown state {s:?}; expected one of {:?}", ChapterState::ALL.map(ChapterState::as_str))
    })
}

fn parse_type(s: &str) -> Result<EntityType, String> {
    EntityType::ALL.into_iter().find(|t| t.as_str() == s).ok_or_else(|| format!("unknown entity type {s:?}"))
}

fn read_delta(path: &PathBuf) -> Result<MemoryDelta, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn print_report(report: &ValidationReport) {
    if report.findings.is_empty() {
        println!("no findings");
    }
    for f in &report.findings {
        let at = f.op_index.map_or_else(|| "delta".to_owned(), |i| format!("op {i}"));
        println!("{:?}\t{at}\t{}", f.severity, f.message);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    if let Command::Init { title } = &cli.command {
        let project = Project::create(&cli.project, title).map_err(|e| err(&e))?;
        println!("created {} ({})", project.manifest().title, project.manifest().id);
        return Ok(());
    }
    let project = Project::open(&cli.project).map_err(|e| err(&e))?;
    let panels = ProjectPanelIndex::new(&project);

    match cli.command {
        Command::Init { .. } => unreachable!("handled above"),
        Command::Info => {
            let book = ReceiptBook::open(&project).map_err(|e| err(&e))?;
            let m = project.manifest();
            println!("title     {}\nid        {}\nlanguage  {}", m.title, m.id, m.language);
            println!("chapters  {}", chapter::list_chapters(&project).map_err(|e| err(&e))?.len());
            println!("ledger    {} entries, head {}", book.ledger().len(), &book.ledger().head()[..12]);
            println!("entities  {}", book.state().len());
            println!("locked    {}", book.is_locked());
            if let Some(r) = book.repair() {
                println!("repaired  dropped {} event(s) from an interrupted commit", r.dropped_events);
            }
        }
        Command::Import { pdf, title } => {
            let c = chapter::import_chapter(&project, &pdf, title.as_deref()).map_err(|e| err(&e))?;
            println!("imported {} from {}", c.id, c.original_name);
        }
        Command::Chapters => {
            for c in chapter::list_chapters(&project).map_err(|e| err(&e))? {
                let stale = if c.stale { " (stale)" } else { "" };
                println!("{}\t{}{stale}\t{}", c.id, c.state, c.title.as_deref().unwrap_or(&c.original_name));
            }
        }
        Command::Advance { chapter: id, to } => {
            let c = chapter::advance(&project, id, parse_state(&to)?).map_err(|e| err(&e))?;
            println!("{} -> {}", c.id, c.state);
        }
        Command::Rewind { chapter: id, to } => {
            let c = chapter::rewind(&project, id, parse_state(&to)?).map_err(|e| err(&e))?;
            println!("{} -> {}", c.id, c.state);
        }
        Command::Check { delta } => {
            let book = ReceiptBook::open(&project).map_err(|e| err(&e))?;
            let report = book.check(&read_delta(&delta)?, &panels);
            print_report(&report);
            if !report.is_committable() {
                return Err("delta would be rejected".into());
            }
        }
        Command::Commit { delta, session } => {
            let mut book = ReceiptBook::open(&project).map_err(|e| err(&e))?;
            match book.commit(&read_delta(&delta)?, session.as_deref(), &panels) {
                Ok(outcome) => {
                    print_report(&outcome.report);
                    println!("committed {} event(s); ledger now {}", outcome.events.len(), book.ledger().len());
                }
                Err(BookError::Rejected(report)) => {
                    print_report(&report);
                    return Err("delta rejected".into());
                }
                Err(e) => return Err(err(&e)),
            }
        }
        Command::BeginSession => {
            ReceiptBook::open(&project).and_then(|mut b| b.begin_session()).map_err(|e| err(&e))?;
            println!("Receipt Book unlocked");
        }
        Command::Snapshot { label, session } => {
            let mut book = ReceiptBook::open(&project).map_err(|e| err(&e))?;
            let snap = book.snapshot_and_lock(label.as_deref(), session.as_deref()).map_err(|e| err(&e))?;
            println!(
                "{} at ledger {} ({} tier files); Receipt Book locked",
                snap.id,
                snap.ledger_len,
                snap.tiers.len()
            );
        }
        Command::Verify => {
            let book = ReceiptBook::open(&project).map_err(|e| err(&e))?;
            let report = book.verify().map_err(|e| err(&e))?;
            println!("ledger {} entries, {} snapshot(s)", report.ledger_len, report.snapshots);
            for p in &report.problems {
                println!("problem\t{p}");
            }
            if !report.is_ok() {
                return Err("verification failed".into());
            }
            println!("ok");
        }
        Command::Entities { entity_type } => {
            let book = ReceiptBook::open(&project).map_err(|e| err(&e))?;
            let filter = entity_type.as_deref().map(parse_type).transpose()?;
            for e in book.state().entities().filter(|e| filter.is_none_or(|t| t == e.entity_type)) {
                println!("{}\t{}\t{}", e.entity_type, e.id, serde_json::Value::Object(e.data.clone()));
            }
        }
        Command::RebuildViews => {
            ReceiptBook::open(&project).and_then(|b| b.rebuild_views()).map_err(|e| err(&e))?;
            println!("views rebuilt");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
