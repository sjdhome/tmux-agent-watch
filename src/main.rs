#![warn(clippy::unwrap_used, clippy::expect_used)]

mod app;
mod cli;
mod detect;
mod pi_ask_user;
mod platform;
mod snapshot;
mod state;
mod tmux;
mod ui;

const USAGE: &str = "\
tmux-agent-watch — monitor AI coding agents in tmux panes

USAGE:
  tmux-agent-watch                 run the TUI
  tmux-agent-watch --once          one scan, plain-text tree to stdout
  tmux-agent-watch --once --json   one scan, JSON to stdout
  tmux-agent-watch --explain <%N>  trace every manifest rule for one pane
  tmux-agent-watch --help          this help
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut once = false;
    let mut json = false;
    let mut explain: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--once" => once = true,
            "--json" => json = true,
            "--explain" => match iter.next() {
                Some(pane_id) => explain = Some(pane_id.clone()),
                None => {
                    eprintln!("--explain requires a pane id (e.g. %3)");
                    std::process::exit(2);
                }
            },
            "--help" | "-h" => {
                print!("{USAGE}");
                return;
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let code = if let Some(pane_id) = explain {
        cli::run_explain(&pane_id)
    } else if once || json {
        cli::run_once(json)
    } else {
        match app::run() {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("error: {err:#}");
                1
            }
        }
    };
    std::process::exit(code);
}
