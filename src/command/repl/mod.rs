// src/command/repl/mod.rs

mod helper;
mod history;
mod session;

use std::borrow::Cow;
use std::time::Instant;

use bon::Builder;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use clap::Args;
use helper::ReplHelper;
use history::HistoryTarget;
use history::load_history;
use history::save_history;
use miette::Diagnostic;
use rustyline::CompletionType;
use rustyline::Config;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use session::ReplSession;
use thiserror::Error;

use crate::command::ExecuteCommand;
use crate::command::args::IndexFilterArgs;
use crate::command::args::ReplHistoryArgs;
use crate::command::args::RepoTargetArgs;
use crate::command::args::SemanticArgs;
use crate::command::runnable;
use crate::index::IndexError;
use crate::render::RenderError;
use crate::render::TerminalRenderer;
use crate::types::SearchQuery;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum ReplError {
    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(transparent)]
    Render(#[from] RenderError),

    #[error("terminal line editor error: {0}")]
    #[diagnostic(code(blite::repl::readline))]
    Readline(#[from] ReadlineError),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ReplCommand {
    #[command(flatten)]
    #[builder(default)]
    pub(crate) target: RepoTargetArgs,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(short, long)]
    pub(crate) kitchen: Option<Utf8PathBuf>,

    /// Maximum number of matching documents to return per query
    #[arg(short = 'n', long, default_value_t = 5)]
    #[builder(default = 5)]
    pub(crate) limit: usize,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) history: ReplHistoryArgs,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) semantic: SemanticArgs,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) filter: IndexFilterArgs,
}

impl Default for ReplCommand {
    fn default() -> Self {
        Self::builder().build()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReplAction {
    Continue,
    Message(Cow<'static, str>),
    Exit,
    Reload,
    SetLimit(usize),
    Help,
    HistoryPath,
    History(SearchQuery),
    Search(SearchQuery),
}

impl ExecuteCommand for ReplCommand {
    type Error = ReplError;
    type Output = ();

    fn execute(self) -> Result<Self::Output, Self::Error> {
        let start = Instant::now();
        let build_opts = self.semantic.to_build_options(&self.filter, true);
        let mut session = ReplSession::new(self.target.repo.clone(), self.kitchen.as_deref(), self.limit, build_opts)?;
        let renderer = TerminalRenderer::new();

        let history_path = HistoryTarget::from_args(&self.history, &self.target.repo).resolve_path();
        let mut rl = init_editor(&session, history_path.as_deref())?;
        print_startup_banner(&self.target.repo, start, &session, history_path.as_deref());

        loop {
            match rl.readline("\x1b[1;34mblite>\x1b[0m ") {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if history_path.is_some() {
                        let _ = rl.add_history_entry(trimmed);
                    }

                    let action = parse_input(trimmed);
                    if !handle_action(action, &mut session, &mut rl, &renderer, history_path.as_deref())? {
                        break;
                    }
                }
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => break,
                Err(err) => return Err(err.into()),
            }
        }

        if let Some(path) = &history_path {
            save_history(&mut rl, path);
        }

        Ok(())
    }
}

runnable!(ReplCommand);

fn init_editor(session: &ReplSession, history_path: Option<&Utf8Path>) -> Result<Editor<ReplHelper, DefaultHistory>, ReplError> {
    let rl_config = Config::builder()
        .max_history_size(1_000)?
        .history_ignore_dups(true)?
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .build();

    let mut rl: Editor<ReplHelper, DefaultHistory> = Editor::with_config(rl_config)?;
    let mut helper = ReplHelper::default();
    helper.update_from_index(session.index());
    rl.set_helper(Some(helper));

    if let Some(path) = history_path {
        load_history(&mut rl, path);
    }

    Ok(rl)
}

fn print_startup_banner(repo: &Utf8Path, start: Instant, session: &ReplSession, history_path: Option<&Utf8Path>) {
    let history_status = history_path.map_or("disabled", Utf8Path::as_str);
    let semantic_status = match session.index().active_semantic_info() {
        Some((model, device)) => format!("{model} on {device}"),
        None => "lexical only".to_string(),
    };

    eprintln!(
        "\x1b[1;32mIndexed {}\x1b[0m in {:.0?} \x1b[2m(search: {}, history: {})\x1b[0m\nType :help for commands, :exit to quit",
        repo,
        start.elapsed(),
        semantic_status,
        history_status,
    );
}

/// Dispatches a parsed `ReplAction`, returning `Ok(true)` to continue the loop or `Ok(false)` to exit.
fn handle_action(
    action: ReplAction,
    session: &mut ReplSession,
    rl: &mut Editor<ReplHelper, DefaultHistory>,
    renderer: &TerminalRenderer,
    history_path: Option<&Utf8Path>,
) -> Result<bool, ReplError> {
    match action {
        ReplAction::Continue => {}
        ReplAction::Message(msg) => eprintln!("{msg}"),
        ReplAction::Exit => return Ok(false),
        ReplAction::Help => print_help(),
        ReplAction::SetLimit(new_limit) => {
            session.set_limit(new_limit);
            eprintln!("Result limit set to {new_limit}.");
        }
        ReplAction::Reload => {
            let reload_start = Instant::now();
            match session.reload() {
                Ok(()) => {
                    if let Some(helper) = rl.helper_mut() {
                        helper.update_from_index(session.index());
                    }
                    eprintln!("Reloaded index in {:.0?}.", reload_start.elapsed());
                }
                Err(err) => {
                    eprintln!("\x1b[1;31mReload failed:\x1b[0m {err}");
                }
            }
        }
        ReplAction::History(query) => match session.search_history(&query) {
            Ok(hits) => {
                let rendered = renderer.render_commit_hits(query.as_ref(), &hits);
                print!("{rendered}");
            }
            Err(err) => {
                eprintln!("\x1b[1;31mHistory error:\x1b[0m {err}");
            }
        },
        ReplAction::HistoryPath => match history_path {
            Some(path) => eprintln!("REPL history: {path}"),
            None => eprintln!("REPL history: disabled"),
        },
        ReplAction::Search(query) => match session.search(&query) {
            Ok(hits) => {
                let rendered = renderer.render_hits(query.as_ref(), &hits)?;
                print!("{rendered}");
            }
            Err(err) => {
                eprintln!("\x1b[1;31mQuery error:\x1b[0m {err}");
            }
        },
    }

    Ok(true)
}

fn parse_input(input: &str) -> ReplAction {
    if let Some(cmd) = input.strip_prefix(':') {
        let mut parts = cmd.split_whitespace();
        return match parts.next().unwrap_or_default() {
            "exit" | "quit" | "q" => ReplAction::Exit,
            "reload" | "r" => ReplAction::Reload,
            "help" | "h" | "?" => ReplAction::Help,
            "history" | "git" | "log" => {
                let rest: String = parts.collect::<Vec<_>>().join(" ");
                match SearchQuery::try_from(rest) {
                    Ok(query) => ReplAction::History(query),
                    Err(_) => ReplAction::Message(Cow::Borrowed("Usage: :history <query>")),
                }
            }
            "history-path" | "history-file" | "hp" => ReplAction::HistoryPath,
            "limit" => match parts.next().and_then(|s| s.parse::<usize>().ok()) {
                Some(n) if n > 0 => ReplAction::SetLimit(n),
                _ => ReplAction::Message(Cow::Borrowed("Usage: :limit <positive integer>")),
            },
            other => ReplAction::Message(Cow::Owned(format!(
                "Unknown command ':{other}'. Type :help for available commands."
            ))),
        };
    }

    match SearchQuery::try_from(input) {
        Ok(query) => ReplAction::Search(query),
        Err(_) => ReplAction::Continue,
    }
}

fn print_help() {
    eprintln!(
        "\
Commands:
  <query>           Search manifests & code (supports kind:, ns:, labels:, annotations:, symbol:, doc_index:)
  :history <query>  Search Git commit messages, authors, and touched file paths
  :history-path     Show the active REPL history file path (or if disabled)
  :limit <N>        Set max results shown per query
  :reload           Re-index the repository from disk
  :help             Show this help message
  :exit             Exit the REPL (or press Ctrl+D)"
    );
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::ReplAction;
    use super::parse_input;
    use crate::index::IndexBuildOptions;
    use crate::index::RepoIndex;
    use crate::index::SemanticMode;
    use crate::test_util::TempDir;
    use crate::types::LineRange;
    use crate::types::SearchQuery;

    #[test]
    fn parses_repl_meta_commands_and_aliases() {
        assert_eq!(parse_input(":exit"), ReplAction::Exit);
        assert_eq!(parse_input(":quit"), ReplAction::Exit);
        assert_eq!(parse_input(":q"), ReplAction::Exit);

        assert_eq!(parse_input(":reload"), ReplAction::Reload);
        assert_eq!(parse_input(":r"), ReplAction::Reload);

        assert_eq!(parse_input(":help"), ReplAction::Help);
        assert_eq!(parse_input(":h"), ReplAction::Help);
        assert_eq!(parse_input(":?"), ReplAction::Help);

        assert_eq!(parse_input(":history-path"), ReplAction::HistoryPath);
        assert_eq!(parse_input(":hp"), ReplAction::HistoryPath);
    }

    #[test]
    fn parses_parameterized_commands_and_usage_errors() {
        assert_eq!(parse_input(":limit 15"), ReplAction::SetLimit(15));
        assert_eq!(
            parse_input(":limit 0"),
            ReplAction::Message(Cow::Borrowed("Usage: :limit <positive integer>"))
        );
        assert_eq!(
            parse_input(":limit abc"),
            ReplAction::Message(Cow::Borrowed("Usage: :limit <positive integer>"))
        );

        assert_eq!(
            parse_input(":history flux-system"),
            ReplAction::History(SearchQuery::try_from("flux-system").unwrap())
        );
        assert_eq!(parse_input(":git "), ReplAction::Message(Cow::Borrowed("Usage: :history <query>")));

        assert_eq!(
            parse_input(":bogus"),
            ReplAction::Message(Cow::Owned(
                "Unknown command ':bogus'. Type :help for available commands.".to_string()
            ))
        );
    }

    #[test]
    fn parses_regular_queries_and_blank_inputs() {
        assert_eq!(
            parse_input("kind:Deployment web"),
            ReplAction::Search(SearchQuery::try_from("kind:Deployment web").unwrap())
        );
        assert_eq!(parse_input("   "), ReplAction::Continue);
    }
    fn lexical_build_opts() -> IndexBuildOptions {
        IndexBuildOptions::builder()
            .semantic_mode(SemanticMode::Off)
            .include_untracked(true)
            .build()
    }

    #[test]
    fn omits_doc_index_on_single_document_manifests() {
        let tmp = TempDir::new("single-doc-index");
        let manifest = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-gateway
  namespace: edge
";
        fs_err::write(tmp.path().join("deployment.yaml"), manifest).unwrap();

        let index = RepoIndex::build_with_options(tmp.path(), &lexical_build_opts()).unwrap();
        let query = SearchQuery::try_from("api-gateway").unwrap();
        let hits = index.search(&query, 5).unwrap();

        assert_eq!(hits.len(), 1);
        let hit = hits.first().unwrap();
        assert_eq!(hit.doc_index, None);
        assert_eq!(hit.lines, None);
    }

    #[test]
    fn indexes_and_filters_by_doc_index_in_multi_doc_streams() {
        let tmp = TempDir::new("multi-doc-index");
        let stream = "\
apiVersion: v1
kind: Namespace
metadata:
  name: flux-system
---
# non-k8s YAML block occupies doc_index 1
notes: skipped during k8s extraction
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kustomize-controller
  namespace: flux-system
";
        fs_err::write(tmp.path().join("gotk-components.yaml"), stream).unwrap();

        let index = RepoIndex::build_with_options(tmp.path(), &lexical_build_opts()).unwrap();

        let filtered = SearchQuery::try_from("path:gotk-components.yaml AND doc_index:2").unwrap();
        let hits = index.search(&filtered, 5).unwrap();

        assert_eq!(hits.len(), 1);
        let hit = hits.first().unwrap();
        assert_eq!(hit.doc_index, Some(2));
        assert_eq!(hit.lines, Some(LineRange { start: 8, end: 13 }));
        assert_eq!(hit.k8s.as_ref().and_then(|k| k.name.as_deref()), Some("kustomize-controller"));
    }
}
