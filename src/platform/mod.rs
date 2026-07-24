//! Platform-specific process discovery behind a clean boundary.
//!
//! v1 supports macOS only; a future `linux.rs` reading `/proc/<pid>/stat`
//! (tpgid) and `/proc/<pid>/cmdline` can slot in without changing callers.

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{foreground_job, nested_pty_children};

#[cfg(not(target_os = "macos"))]
compile_error!("tmux-agent-watch v1 only supports macOS; see src/platform/mod.rs");

/// One process inside a pane's foreground job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub pid: u32,
    /// Kernel-reported command name (`pbi_comm`, truncated to 16 bytes).
    pub name: String,
    /// Basename of argv[0]; reflects runtime title changes.
    pub argv0: Option<String>,
    pub argv: Option<Vec<String>>,
    pub cmdline: Option<String>,
}

/// The foreground process group of a pane's controlling terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundJob {
    pub process_group_id: u32,
    pub processes: Vec<ForegroundProcess>,
}
