//! Agent identification from a pane's foreground job, ported from herdr's
//! `src/detect/mod.rs` (Apache-2.0; see NOTICE). Unix subset only: the
//! Windows cmd/powershell unwrapping paths are intentionally omitted.

pub mod engine;
pub mod manifests;

use crate::platform::{ForegroundJob, ForegroundProcess};

/// Which agent we detected running in a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Agent {
    Pi,
    Claude,
    Codex,
    Gemini,
    Cursor,
    Devin,
    Antigravity,
    Cline,
    Omp,
    Mastracode,
    OpenCode,
    GithubCopilot,
    Kimi,
    Kiro,
    Droid,
    Amp,
    Grok,
    Hermes,
    Kilo,
    Qodercli,
    Maki,
}

pub fn agent_label(agent: Agent) -> &'static str {
    match agent {
        Agent::Pi => "pi",
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Gemini => "gemini",
        Agent::Cursor => "cursor",
        Agent::Devin => "devin",
        Agent::Antigravity => "agy",
        Agent::Cline => "cline",
        Agent::Omp => "omp",
        Agent::Mastracode => "mastracode",
        Agent::OpenCode => "opencode",
        Agent::GithubCopilot => "copilot",
        Agent::Kimi => "kimi",
        Agent::Kiro => "kiro",
        Agent::Droid => "droid",
        Agent::Amp => "amp",
        Agent::Grok => "grok",
        Agent::Hermes => "hermes",
        Agent::Kilo => "kilo",
        Agent::Qodercli => "qodercli",
        Agent::Maki => "maki",
    }
}

/// Manifest id for an agent, where one exists. Manifest `id` fields use the
/// canonical agent label (e.g. "agy", "copilot"). `omp` and `mastracode` have
/// no screen manifest in herdr; they fall back to known-agent-Idle.
pub fn manifest_id(agent: Agent) -> Option<&'static str> {
    match agent {
        Agent::Omp | Agent::Mastracode => None,
        other => Some(agent_label(other)),
    }
}

pub fn parse_agent_label(agent: &str) -> Option<Agent> {
    lookup_agent(&normalized_agent_lookup_name(agent))
}

fn lookup_agent(name: &str) -> Option<Agent> {
    match name {
        "pi" => Some(Agent::Pi),
        "claude" | "claude-code" => Some(Agent::Claude),
        "codex" => Some(Agent::Codex),
        "gemini" => Some(Agent::Gemini),
        "cursor" | "cursor-agent" => Some(Agent::Cursor),
        "devin" | "devin-cli" | "devin cli" => Some(Agent::Devin),
        "agy" | "antigravity" | "antigravity-cli" => Some(Agent::Antigravity),
        "cline" => Some(Agent::Cline),
        "omp" => Some(Agent::Omp),
        "mastracode" | "mastra-code" | "mastra code" => Some(Agent::Mastracode),
        "opencode" | "open-code" => Some(Agent::OpenCode),
        "copilot" | "github-copilot" | "ghcs" => Some(Agent::GithubCopilot),
        "kimi" | "kimi-code" | "kimi code" => Some(Agent::Kimi),
        "kiro" | "kiro-cli" => Some(Agent::Kiro),
        "droid" => Some(Agent::Droid),
        "amp" | "amp-local" => Some(Agent::Amp),
        "grok" | "grok-build" => Some(Agent::Grok),
        "hermes" | "hermes-agent" => Some(Agent::Hermes),
        "kilo" | "kilo-code" | "kilo code" => Some(Agent::Kilo),
        "qodercli" | "qoderclicn" | "qoder" | "qodercn" => Some(Agent::Qodercli),
        "maki" => Some(Agent::Maki),
        _ => None,
    }
}

fn identify_agent(process_name: &str) -> Option<Agent> {
    parse_agent_label(process_name)
}

/// Total anchors examined while descending through nested PTYs.
const PANE_RESOLUTION_BUDGET: usize = 8;

/// Identify the agent for a tmux pane, descending through nested-PTY wrapper
/// shells (e.g. koshell spawning zsh on an inner tty) when the pane's own
/// foreground job contains no agent.
pub fn identify_pane_agent(pane_pid: u32) -> Option<(Agent, String)> {
    let mut anchors = std::collections::VecDeque::from([pane_pid]);
    let mut seen = std::collections::HashSet::new();
    let mut budget = PANE_RESOLUTION_BUDGET;

    while let Some(anchor) = anchors.pop_front() {
        if budget == 0 || !seen.insert(anchor) {
            break;
        }
        budget -= 1;

        let Some(job) = crate::platform::foreground_job(anchor) else {
            continue;
        };
        if let Some(found) = identify_agent_in_job(&job) {
            return Some(found);
        }
        anchors.extend(crate::platform::nested_pty_children(&job));
    }

    None
}

/// Identify the agent running in a pane's foreground job. Prefers the process
/// group leader when it maps to an agent; otherwise the best-scoring match
/// across the job's processes.
pub fn identify_agent_in_job(job: &ForegroundJob) -> Option<(Agent, String)> {
    if let Some(process) = job
        .processes
        .iter()
        .find(|process| process.pid == job.process_group_id)
    {
        let candidate = normalized_process_name(process);
        if let Some(agent) = identify_agent(&candidate) {
            return Some((agent, candidate));
        }
    }

    let mut best: Option<(u8, Agent, String)> = None;

    for process in &job.processes {
        let candidate = normalized_process_name(process);
        let Some(agent) = identify_agent(&candidate) else {
            continue;
        };
        let score = process_priority(process, &candidate);

        match &best {
            Some((best_score, _, _)) if *best_score >= score => {}
            _ => best = Some((score, agent, candidate)),
        }
    }

    best.map(|(_, agent, name)| (agent, name))
}

fn normalized_process_name(process: &ForegroundProcess) -> String {
    let effective = process.argv0.as_deref().unwrap_or(&process.name);
    let lower_effective = effective.to_lowercase();

    if is_generic_runtime_or_shell(&lower_effective)
        && let Some(wrapped_agent) =
            wrapped_agent_name_from_runtime_argv(&lower_effective, process.argv.as_deref())
    {
        return wrapped_agent;
    }

    if identify_agent(effective).is_some() {
        return effective.to_string();
    }

    if let Some(wrapped_agent) = argv0_agent_name(process.argv.as_deref())
        .or_else(|| cmdline_argv0_agent_name(process.cmdline.as_deref().unwrap_or_default()))
    {
        return wrapped_agent;
    }

    effective.to_string()
}

fn wrapped_agent_name_from_runtime_argv(runtime: &str, argv: Option<&[String]>) -> Option<String> {
    let argv = argv?;
    let runtime = normalized_agent_lookup_name(path_basename(runtime));

    match runtime.as_str() {
        "node" | "bun" => script_arg_agent_name(argv, &["-e", "--eval", "-p", "--print"], &[]),
        "python" | "python3" => script_arg_agent_name(argv, &["-c"], &["-m"]),
        "sh" | "bash" | "zsh" | "fish" => script_arg_agent_name(argv, &["-c"], &[]),
        _ => None,
    }
}

fn script_arg_agent_name(
    argv: &[String],
    eval_flags: &[&str],
    module_flags: &[&str],
) -> Option<String> {
    let mut args = argv.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--" {
            return args
                .next()
                .and_then(|token| agent_name_from_path_token(token));
        }

        if flag_matches(arg, eval_flags) || flag_matches(arg, module_flags) {
            return None;
        }

        if arg.starts_with('-') {
            if option_takes_value(arg) {
                let _ = args.next();
            }
            continue;
        }

        return agent_name_from_path_token(arg);
    }

    None
}

fn flag_matches(arg: &str, flags: &[&str]) -> bool {
    flags
        .iter()
        .any(|flag| arg == *flag || short_flag_payload(arg, flag) || long_flag_value(arg, flag))
}

fn short_flag_payload(arg: &str, flag: &str) -> bool {
    flag.starts_with('-')
        && !flag.starts_with("--")
        && arg.starts_with(flag)
        && arg.len() > flag.len()
}

fn long_flag_value(arg: &str, flag: &str) -> bool {
    flag.starts_with("--")
        && arg
            .strip_prefix(flag)
            .is_some_and(|rest| rest.starts_with('='))
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-r" | "--require"
            | "--loader"
            | "--import"
            | "--experimental-loader"
            | "--inspect-port"
            | "-W"
            | "-X"
            | "-S"
            | "-L"
            | "-o"
    )
}

fn argv0_agent_name(argv: Option<&[String]>) -> Option<String> {
    agent_name_from_path_token(argv?.first()?)
}

fn cmdline_argv0_agent_name(cmdline: &str) -> Option<String> {
    agent_name_from_path_token(cmdline.split_whitespace().next()?)
}

fn agent_name_from_path_token(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|c| matches!(c, '"' | '\''));
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return None;
    }

    agent_name_from_basename(path_basename(trimmed))
        .or_else(|| agent_name_from_known_package_path(trimmed))
        .or_else(|| resolved_agent_name_from_path_token(trimmed))
}

fn agent_name_from_known_package_path(path: &str) -> Option<String> {
    let components: Vec<String> = path
        .split(['/', '\\'])
        .filter(|component| !component.is_empty())
        .map(normalized_agent_lookup_name)
        .collect();

    for window in components.windows(5) {
        if window
            == [
                "node_modules",
                "@earendil-works",
                "pi-coding-agent",
                "dist",
                "cli",
            ]
        {
            return Some(agent_label(Agent::Pi).to_string());
        }
    }
    None
}

fn resolved_agent_name_from_path_token(token: &str) -> Option<String> {
    let path = std::path::Path::new(token);
    if path.components().count() < 2 {
        return None;
    }

    let resolved = std::fs::canonicalize(path).ok()?;
    let basename = resolved.file_name()?.to_str()?;
    agent_name_from_basename(basename)
}

fn agent_name_from_basename(basename: &str) -> Option<String> {
    let agent = parse_agent_label(basename)?;
    Some(agent_label(agent).to_string())
}

fn normalized_agent_lookup_name(name: &str) -> String {
    let mut name = name.trim().to_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".js"] {
        if name.ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    name
}

fn path_basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(path)
}

fn process_priority(process: &ForegroundProcess, normalized_name: &str) -> u8 {
    let lower_name = normalized_name.to_lowercase();
    if lower_name != process.name.to_lowercase() {
        return 3;
    }
    if !is_generic_runtime_or_shell(&lower_name) {
        return 2;
    }
    1
}

fn is_generic_runtime_or_shell(name: &str) -> bool {
    let name = normalized_agent_lookup_name(path_basename(name));
    matches!(
        name.as_str(),
        "sh" | "bash" | "zsh" | "fish" | "tmux" | "node" | "bun" | "python" | "python3"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foreground_process(pid: u32, name: &str, argv: &[&str]) -> ForegroundProcess {
        ForegroundProcess {
            pid,
            name: name.to_string(),
            argv0: None,
            argv: Some(argv.iter().map(|arg| (*arg).to_string()).collect()),
            cmdline: Some(argv.join(" ")),
        }
    }

    #[test]
    fn identifies_known_agents_and_aliases() {
        assert_eq!(parse_agent_label("claude"), Some(Agent::Claude));
        assert_eq!(parse_agent_label("claude-code"), Some(Agent::Claude));
        assert_eq!(parse_agent_label("CLAUDE"), Some(Agent::Claude));
        assert_eq!(parse_agent_label("cursor-agent"), Some(Agent::Cursor));
        assert_eq!(parse_agent_label("ghcs"), Some(Agent::GithubCopilot));
        assert_eq!(parse_agent_label("opencode.exe"), Some(Agent::OpenCode));
        assert_eq!(parse_agent_label("kiro-cli"), Some(Agent::Kiro));
        assert_eq!(parse_agent_label("bash"), None);
        assert_eq!(parse_agent_label("vim"), None);
        assert_eq!(parse_agent_label("node"), None);
    }

    #[test]
    fn prefers_wrapped_agent_over_shell() {
        let job = ForegroundJob {
            process_group_id: 123,
            processes: vec![
                foreground_process(1, "node", &["node", "/path/to/bin/codex"]),
                foreground_process(2, "bash", &["bash"]),
            ],
        };
        assert_eq!(
            identify_agent_in_job(&job),
            Some((Agent::Codex, "codex".to_string()))
        );
    }

    #[test]
    fn prefers_recognized_process_group_leader() {
        let job = ForegroundJob {
            process_group_id: 42,
            processes: vec![
                foreground_process(42, "claude", &["claude"]),
                foreground_process(43, "node", &["node", "/tmp/mcp/bin/codex"]),
            ],
        };
        assert_eq!(
            identify_agent_in_job(&job),
            Some((Agent::Claude, "claude".to_string()))
        );
    }

    #[test]
    fn eval_arguments_are_not_agents() {
        let job = ForegroundJob {
            process_group_id: 123,
            processes: vec![foreground_process(
                1,
                "python3",
                &["python3", "-c", "import time; time.sleep(60)", "/tmp/codex"],
            )],
        };
        assert_eq!(identify_agent_in_job(&job), None);

        let job = ForegroundJob {
            process_group_id: 123,
            processes: vec![foreground_process(
                1,
                "node",
                &["node", "-e", "setTimeout(() => {}, 60000)", "/tmp/codex"],
            )],
        };
        assert_eq!(identify_agent_in_job(&job), None);
    }

    #[test]
    fn shell_wrapped_script_is_detected() {
        let job = ForegroundJob {
            process_group_id: 123,
            processes: vec![foreground_process(
                1,
                "sh",
                &["/bin/sh", "/tmp/test-bin/pi"],
            )],
        };
        assert_eq!(
            identify_agent_in_job(&job),
            Some((Agent::Pi, "pi".to_string()))
        );
    }

    #[test]
    fn pi_package_cli_path_is_detected() {
        let job = ForegroundJob {
            process_group_id: 123,
            processes: vec![foreground_process(
                123,
                "node",
                &[
                    "node",
                    "/usr/local/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js",
                ],
            )],
        };
        assert_eq!(
            identify_agent_in_job(&job),
            Some((Agent::Pi, "pi".to_string()))
        );
    }

    #[test]
    fn plain_shell_job_is_not_an_agent() {
        let job = ForegroundJob {
            process_group_id: 9,
            processes: vec![foreground_process(9, "zsh", &["-zsh"])],
        };
        assert_eq!(identify_agent_in_job(&job), None);
    }

    #[test]
    fn every_agent_with_manifest_has_matching_label() {
        for (agent, expected) in [
            (Agent::Claude, Some("claude")),
            (Agent::GithubCopilot, Some("copilot")),
            (Agent::Antigravity, Some("agy")),
            (Agent::Omp, None),
            (Agent::Mastracode, None),
        ] {
            assert_eq!(manifest_id(agent), expected);
        }
    }
}
