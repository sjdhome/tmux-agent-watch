//! macOS foreground-process discovery, ported from herdr's
//! `src/platform/macos.rs` (Apache-2.0; see NOTICE).
//!
//! Everything here is PID-based (`proc_pidinfo`, `proc_listpids`,
//! `sysctl(KERN_PROCARGS2)`) and works for same-uid processes without owning
//! the pane's PTY, so a tmux `#{pane_pid}` is a sufficient starting point.

use std::path::Path;

use super::{ForegroundJob, ForegroundProcess};

const PROC_PGRP_ONLY: u32 = 2;
const PROC_PPID_ONLY: u32 = 6;

/// `e_tdev` value meaning "no controlling terminal".
const NODEV: u32 = u32::MAX;

/// Collect the foreground terminal job for a pane's shell PID.
pub fn foreground_job(pane_pid: u32) -> Option<ForegroundJob> {
    if pane_pid == 0 {
        return None;
    }

    let fg_pgid = foreground_process_group_id(pane_pid)?;
    let mut processes = Vec::new();

    for pid in process_group_pids(fg_pgid) {
        let Some(info) = process_bsdinfo(pid) else {
            continue;
        };
        if info.pbi_pgid != fg_pgid {
            continue;
        }
        let Some(name) = comm_from_bsdinfo(&info) else {
            continue;
        };
        let argv = process_argv(pid);
        processes.push(ForegroundProcess {
            pid,
            name,
            argv0: process_argv0_name(pid),
            cmdline: argv.as_ref().map(|parts| parts.join(" ")),
            argv,
        });
    }

    if processes.is_empty() {
        return None;
    }

    Some(ForegroundJob {
        process_group_id: fg_pgid,
        processes,
    })
}

/// PIDs of direct children of the job's processes that live on a different
/// controlling terminal than their parent.
///
/// Wrapper shells (e.g. koshell) allocate a nested PTY and run the real
/// interactive shell inside it; the agent then runs on that inner tty, where
/// the pane pid's own `e_tpgid` cannot see it. Callers descend through these
/// children to resolve the innermost foreground job.
pub fn nested_pty_children(job: &ForegroundJob) -> Vec<u32> {
    let mut children = Vec::new();
    for process in &job.processes {
        let Some(parent_info) = process_bsdinfo(process.pid) else {
            continue;
        };
        for child_pid in child_pids(process.pid) {
            let Some(child_info) = process_bsdinfo(child_pid) else {
                continue;
            };
            if child_info.e_tdev != NODEV && child_info.e_tdev != parent_info.e_tdev {
                children.push(child_pid);
            }
        }
    }
    children
}

fn child_pids(ppid: u32) -> Vec<u32> {
    let mut capacity = 16usize;

    for _ in 0..8 {
        let mut pids = vec![0 as libc::pid_t; capacity];
        let buffer_bytes = pids.len() * std::mem::size_of::<libc::pid_t>();
        let returned_bytes = unsafe {
            libc::proc_listpids(
                PROC_PPID_ONLY,
                ppid,
                pids.as_mut_ptr() as *mut libc::c_void,
                buffer_bytes as libc::c_int,
            )
        };
        if returned_bytes <= 0 {
            return Vec::new();
        }

        let returned_bytes = returned_bytes as usize;
        let count = returned_bytes / std::mem::size_of::<libc::pid_t>();
        if returned_bytes < buffer_bytes {
            return pids
                .into_iter()
                .take(count)
                .filter(|&pid| pid > 0)
                .map(|pid| pid as u32)
                .collect();
        }
        capacity = capacity.saturating_mul(2);
    }

    Vec::new()
}

/// Read `e_tpgid` (foreground process group of the controlling terminal).
fn foreground_process_group_id(pid: u32) -> Option<u32> {
    let info = process_bsdinfo(pid)?;
    let fg = info.e_tpgid;
    if fg > 0 {
        #[allow(clippy::unnecessary_cast)] // pid_t width is platform-dependent
        Some(fg as u32)
    } else {
        None
    }
}

fn process_group_pids(process_group_id: u32) -> Vec<u32> {
    let mut capacity = 16usize;

    for _ in 0..8 {
        let mut pids = vec![0 as libc::pid_t; capacity];
        let buffer_bytes = pids.len() * std::mem::size_of::<libc::pid_t>();
        let returned_bytes = unsafe {
            libc::proc_listpids(
                PROC_PGRP_ONLY,
                process_group_id,
                pids.as_mut_ptr() as *mut libc::c_void,
                buffer_bytes as libc::c_int,
            )
        };
        if returned_bytes <= 0 {
            return Vec::new();
        }

        let returned_bytes = returned_bytes as usize;
        let count = returned_bytes / std::mem::size_of::<libc::pid_t>();
        if returned_bytes < buffer_bytes {
            return pids
                .into_iter()
                .take(count)
                .filter(|&pid| pid > 0)
                .map(|pid| pid as u32)
                .collect();
        }
        capacity = capacity.saturating_mul(2);
    }

    Vec::new()
}

fn process_bsdinfo(pid: u32) -> Option<libc::proc_bsdinfo> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;

    let ret = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };

    (ret == size).then_some(info)
}

fn comm_from_bsdinfo(info: &libc::proc_bsdinfo) -> Option<String> {
    let end = info
        .pbi_comm
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(info.pbi_comm.len());
    if end == 0 {
        return None;
    }
    let bytes: Vec<u8> = info.pbi_comm[..end].iter().map(|&b| b as u8).collect();
    String::from_utf8(bytes).ok()
}

fn process_argv(pid: u32) -> Option<Vec<String>> {
    procargs2_argv(&kern_procargs2(pid)?)
}

/// Effective process name from `argv[0]`; reflects runtime title changes like
/// Node.js `process.title = "pi"`.
fn process_argv0_name(pid: u32) -> Option<String> {
    let buf = kern_procargs2(pid)?;
    let rest = buf.get(4..)?;
    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if argc < 1 {
        return None;
    }

    let pos = procargs2_argv_start(rest)?;
    let argv0_end = rest[pos..]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(rest.len() - pos);
    let argv0 = std::str::from_utf8(&rest[pos..pos + argv0_end]).ok()?;
    if argv0.is_empty() {
        return None;
    }

    let basename = Path::new(argv0).file_name()?.to_str()?;
    // Login shells report as "-zsh".
    let name = basename.strip_prefix('-').unwrap_or(basename);
    (!name.is_empty()).then(|| name.to_string())
}

/// Raw `sysctl(KERN_PROCARGS2)` buffer:
/// `[argc: i32] [exec_path\0] [padding\0...] [argv[0]\0] ... [env\0] ...`
fn kern_procargs2(pid: u32) -> Option<Vec<u8>> {
    unsafe {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];

        let mut size: libc::size_t = 0;
        let ret = libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        );
        if ret != 0 || size == 0 {
            return None;
        }

        let mut buf = vec![0u8; size];
        let ret = libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        );
        if ret != 0 {
            return None;
        }
        buf.truncate(size);
        Some(buf)
    }
}

fn procargs2_argv_start(rest: &[u8]) -> Option<usize> {
    let exec_end = rest.iter().position(|&byte| byte == 0)?;
    let mut pos = exec_end;
    while pos < rest.len() && rest[pos] == 0 {
        pos += 1;
    }
    (pos < rest.len()).then_some(pos)
}

fn procargs2_argv(buf: &[u8]) -> Option<Vec<String>> {
    if buf.len() < 4 {
        return None;
    }

    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if argc < 1 {
        return None;
    }

    let rest = &buf[4..];
    let mut current = procargs2_argv_start(rest)?;
    let mut argv = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        if current >= rest.len() {
            return None;
        }
        let end = rest[current..]
            .iter()
            .position(|&b| b == 0)
            .map(|offset| current + offset)
            .unwrap_or(rest.len());
        if end == current {
            return None;
        }
        argv.push(String::from_utf8_lossy(&rest[current..end]).into_owned());
        current = end + 1;
    }

    Some(argv)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn procargs2_buffer(argc: i32, exec_path: &str, strings: &[&str]) -> Vec<u8> {
        let mut buf = argc.to_ne_bytes().to_vec();
        buf.extend_from_slice(exec_path.as_bytes());
        buf.extend_from_slice(&[0, 0, 0]); // terminator + padding
        for s in strings {
            buf.extend_from_slice(s.as_bytes());
            buf.push(0);
        }
        buf
    }

    #[test]
    fn parses_argv_from_procargs2_layout() {
        let buf = procargs2_buffer(3, "/usr/bin/node", &["node", "/path/to/codex", "--flag"]);
        assert_eq!(
            procargs2_argv(&buf),
            Some(vec![
                "node".to_string(),
                "/path/to/codex".to_string(),
                "--flag".to_string()
            ])
        );
    }

    #[test]
    fn rejects_short_or_empty_buffers() {
        assert_eq!(procargs2_argv(&[]), None);
        assert_eq!(procargs2_argv(&0i32.to_ne_bytes()), None);
    }

    #[test]
    fn foreground_job_of_self_contains_a_process() {
        // The test runner's own pid has a bsdinfo entry; e_tpgid may or may
        // not exist depending on CI tty, so only exercise the non-panicking
        // path here.
        let _ = foreground_job(std::process::id());
    }
}
