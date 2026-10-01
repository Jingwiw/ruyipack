// SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
// SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
// SPDX-License-Identifier: MulanPSL-2.0

//! Shared process lifetime; callers own arguments, environment, logs and presentation.

use serde::Serialize;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    io::{self, Read, Seek, SeekFrom},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

pub(crate) fn install_handler() -> io::Result<()> {
    HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::Relaxed))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .copied()
        .map_err(|e| io::Error::other(e.clone()))
}

#[derive(Serialize)]
pub(crate) struct Termination {
    method: &'static str,
    target: u32,
    reaped: bool,
    pub(crate) error: Option<String>,
}

pub(crate) struct Outcome {
    pub(crate) started: bool,
    pub(crate) status: Option<ExitStatus>,
    pub(crate) timed_out: bool,
    pub(crate) interrupted: bool,
    pub(crate) elapsed_ms: u128,
    pub(crate) error: Option<io::Error>,
    pub(crate) termination: Option<Termination>,
}

enum Stop {
    Signalled,
    Gone,
    #[cfg(target_os = "macos")]
    ZombiesOnly,
}

fn stop(child: &mut Child, group: bool, _root_exited: bool) -> io::Result<Stop> {
    if !group {
        return if child.try_wait()?.is_some() {
            Ok(Stop::Gone)
        } else {
            child.kill().map(|()| Stop::Signalled)
        };
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        match kill_process_group(Pid::from_child(child), Signal::KILL) {
            Ok(()) => Ok(Stop::Signalled),
            Err(rustix::io::Errno::SRCH) => Ok(Stop::Gone),
            // Darwin skips zombies in killpg and returns EPERM for an all-zombie
            // group. Verify that case while the root PID is still pinned; never
            // reinterpret an ordinary permission failure as successful cleanup.
            #[cfg(target_os = "macos")]
            Err(error @ rustix::io::Errno::PERM) if _root_exited => match zombie_group(child) {
                Ok(true) => Ok(Stop::ZombiesOnly),
                Ok(false) => Err(error.into()),
                Err(query) => Err(io::Error::new(
                    query.kind(),
                    format!("{error}; group query: {query}"),
                )),
            },
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        if child.try_wait()?.is_some() {
            return Ok(Stop::Gone);
        }
        child.kill().map(|()| Stop::Signalled)
    }
}

#[cfg(target_os = "macos")]
fn zombie_group(child: &Child) -> io::Result<bool> {
    let mut command = Command::new("/bin/ps");
    command
        .args(["-o", "pid=,pgid=,stat=", "-g", &child.id().to_string()])
        .env("LC_ALL", "C");
    // The query uses this same bounded loop, but direct-child cleanup prevents
    // recursively querying the query's own zombie process group.
    let output = capture_inner(&mut command, Duration::from_secs(2), 64 * 1024, false)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "process-group query failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(io::Error::other)?;
    let mut root_seen = false;
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3
            || fields[1].parse::<u32>() != Ok(child.id())
            || !fields[2].starts_with('Z')
        {
            return Ok(false);
        }
        let pid = fields[0].parse::<u32>().map_err(io::Error::other)?;
        root_seen |= pid == child.id();
    }
    Ok(root_seen)
}

fn exited(child: &mut Child) -> io::Result<bool> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
        // Keep the root waitable until group cleanup: reaping first could let
        // its PID be reused and make a subsequent group signal hit another job.
        waitid(
            WaitId::Pid(Pid::from_child(child)),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        )
        .map(|status| status.is_some())
        .map_err(Into::into)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        child.try_wait().map(|status| status.is_some())
    }
}

fn reap(child: &mut Child) -> io::Result<ExitStatus> {
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= end {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "terminated child did not exit within two seconds",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn run_inner(
    command: &mut Command,
    budget: Duration,
    cancellable: bool,
    group: bool,
    mut check: impl FnMut() -> io::Result<()>,
) -> Outcome {
    let group = group && cfg!(any(target_os = "linux", target_os = "macos"));
    let start = Instant::now();
    let mut outcome = Outcome {
        started: false,
        status: None,
        timed_out: false,
        interrupted: false,
        elapsed_ms: 0,
        error: None,
        termination: None,
    };
    let result = (|| -> io::Result<()> {
        if cancellable {
            install_handler()?;
        }
        if cancellable && INTERRUPTED.load(Ordering::Relaxed) {
            outcome.interrupted = true;
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "process interrupted before launch",
            ));
        }
        if budget.is_zero() {
            outcome.timed_out = true;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process deadline exhausted before launch",
            ));
        }
        #[cfg(unix)]
        if group {
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        outcome.started = true;
        let mut root_exited = false;
        let mut poll = Duration::from_millis(1);
        let result = (|| -> io::Result<()> {
            loop {
                check()?;
                if exited(&mut child)? {
                    root_exited = true;
                    return Ok(());
                }
                if cancellable && INTERRUPTED.load(Ordering::Relaxed) {
                    outcome.interrupted = true;
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "process interrupted",
                    ));
                }
                if start.elapsed() >= budget {
                    outcome.timed_out = true;
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "process exceeded deadline",
                    ));
                }
                thread::sleep(poll);
                poll = (poll * 2).min(Duration::from_millis(25));
            }
        })();
        let stopped = stop(&mut child, group, root_exited);
        let mut termination = Termination {
            method: match &stopped {
                #[cfg(target_os = "macos")]
                Ok(Stop::ZombiesOnly) => "unix-process-group-zombies-only",
                _ if group => "unix-process-group",
                _ => "direct-child",
            },
            target: child.id(),
            reaped: false,
            error: stopped.as_ref().err().map(ToString::to_string),
        };
        // A root can leave its initial group. ESRCH alone is not proof it exited.
        if !root_exited && !matches!(stopped, Ok(Stop::Signalled)) {
            termination.method = "direct-child-fallback";
            if let Err(error) = child.kill() {
                termination.error = Some(match termination.error.take() {
                    Some(previous) => format!("{previous}; direct child: {error}"),
                    None => format!("direct child: {error}"),
                });
            }
        }
        match reap(&mut child) {
            Ok(status) => {
                outcome.status = Some(status);
                termination.reaped = true;
            }
            Err(error) => {
                termination.error = Some(match termination.error.take() {
                    Some(previous) => format!("{previous}; {error}"),
                    None => error.to_string(),
                });
            }
        }
        // No group left on ordinary completion: avoid claiming a termination happened.
        if !matches!(stopped, Ok(Stop::Gone)) || result.is_err() || termination.error.is_some() {
            outcome.termination = Some(termination);
        }
        if let Some(error) = outcome.termination.as_ref().and_then(|t| t.error.as_ref()) {
            return Err(match result {
                Err(original) => {
                    io::Error::new(original.kind(), format!("{original}; cleanup: {error}"))
                }
                Ok(()) => io::Error::other(format!("process cleanup: {error}")),
            });
        }
        result
    })();
    outcome.error = result.err();
    outcome.elapsed_ms = start.elapsed().as_millis();
    outcome
}

/// Temporary regular files avoid waiting for pipe EOF held by grandchildren.
/// Limits reject incomplete output, never truncate it into a successful Git answer.
pub(crate) fn capture(command: &mut Command, budget: Duration, limit: u64) -> io::Result<Output> {
    capture_inner(command, budget, limit, true)
}

fn capture_inner(
    command: &mut Command,
    budget: Duration,
    limit: u64,
    group: bool,
) -> io::Result<Output> {
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    let check = || {
        for (label, file) in [("stdout", &stdout), ("stderr", &stderr)] {
            if file.metadata()?.len() > limit {
                return Err(io::Error::other(format!(
                    "{label} exceeded {limit} bytes; output is incomplete"
                )));
            }
        }
        Ok(())
    };
    let outcome = run_inner(command, budget, group, group, check);
    if let Some(error) = outcome.error {
        return Err(error);
    }
    let read = |file: &mut std::fs::File| -> io::Result<Vec<u8>> {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(io::Error::other(
                "process output exceeded limit; output is incomplete",
            ));
        }
        Ok(bytes)
    };
    Ok(Output {
        status: outcome
            .status
            .ok_or_else(|| io::Error::other("child exit status is unavailable"))?,
        stdout: read(&mut stdout)?,
        stderr: read(&mut stderr)?,
    })
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script).arg("host-process-test");
        command
    }

    #[test]
    fn timeout_stops_root_that_left_its_original_group() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let marker = dir.path().join("moved-root");
        let mut command = Command::new("python3");
        command.arg("-c").arg(
            "import os, sys, time; os.setpgid(0, os.getpgid(os.getppid())); open(sys.argv[1], 'w').write('ready'); time.sleep(2); open(sys.argv[2], 'w').write('survived')",
        ).arg(&ready).arg(&marker).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let outcome = run_inner(&mut command, Duration::from_secs(1), false, true, || Ok(()));
        assert!(
            ready.exists(),
            "fixture did not reach process-group migration"
        );
        assert!(outcome.timed_out);
        let termination = outcome.termination.unwrap();
        assert_eq!(termination.method, "direct-child-fallback");
        assert!(termination.reaped);
        assert!(termination.error.is_none());
        thread::sleep(Duration::from_millis(1200));
        assert!(
            !marker.exists(),
            "root escaped its deadline by changing process group"
        );
    }

    #[test]
    fn capture_keeps_exit_status_and_cleans_background_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let mut markers = Vec::new();
        for exit_code in [0, 7] {
            let marker = dir.path().join(format!("exit-{exit_code}"));
            let mut command = shell(
                r#"(sleep 0.5; printf descendant > "$1") &
                   printf root-out
                   printf root-err >&2
                   exit "$2""#,
            );
            command.arg(&marker).arg(exit_code.to_string());
            let start = Instant::now();
            let output = capture(&mut command, Duration::from_secs(2), 64).unwrap();
            assert_eq!(output.status.code(), Some(exit_code));
            assert_eq!(output.stdout, b"root-out");
            assert_eq!(output.stderr, b"root-err");
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "capture must not wait for descendant-held pipe EOF"
            );
            markers.push(marker);
        }
        thread::sleep(Duration::from_millis(700));
        for marker in markers {
            assert!(
                !marker.exists(),
                "same-group descendant survived completed root: {}",
                marker.display()
            );
        }
    }

    #[test]
    fn capture_deadline_is_an_error_and_stops_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("timed-out-descendant");
        let mut command = shell(
            r#"(sleep 0.5; printf descendant > "$1") &
               wait"#,
        );
        command.arg(&marker);
        let start = Instant::now();
        let error = capture(&mut command, Duration::from_millis(100), 64).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("deadline"));
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "deadline cleanup must finish within a bounded interval"
        );
        thread::sleep(Duration::from_millis(700));
        assert!(
            !marker.exists(),
            "timed-out same-group descendant survived cleanup"
        );
    }

    #[test]
    fn capture_rejects_each_oversized_stream_instead_of_truncating_success() {
        for (stream, script) in [
            ("stdout", "printf 0123456789abcdefg; sleep 10"),
            ("stderr", "printf 0123456789abcdefg >&2; sleep 10"),
        ] {
            let error = capture(&mut shell(script), Duration::from_secs(2), 16).unwrap_err();
            let message = error.to_string();
            assert!(message.contains(stream), "{message}");
            assert!(message.contains("exceeded 16 bytes"), "{message}");
            assert!(message.contains("output is incomplete"), "{message}");
        }
    }

    #[test]
    fn capture_preserves_binary_bytes_and_exact_per_stream_limit() {
        let output = capture(
            &mut shell(r#"printf '\000\377A\000'; printf '\376\000B\377' >&2"#),
            Duration::from_secs(2),
            4,
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, [0, 0xff, b'A', 0]);
        assert_eq!(output.stderr, [0xfe, 0, b'B', 0xff]);
    }

    #[test]
    fn zero_budget_never_launches_run_or_capture() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("must-not-launch");
        let mut command = shell(r#"printf launched > "$1""#);
        command.arg(&marker);
        let mut checks = 0;
        let outcome = run_inner(&mut command, Duration::ZERO, false, true, || {
            checks += 1;
            Ok(())
        });
        assert!(!outcome.started);
        assert!(outcome.status.is_none());
        assert!(outcome.timed_out);
        assert!(!outcome.interrupted);
        assert!(outcome.termination.is_none());
        assert_eq!(outcome.error.unwrap().kind(), io::ErrorKind::TimedOut);
        assert_eq!(checks, 0);
        assert!(!marker.exists());

        let mut command = shell(r#"printf launched > "$1""#);
        command.arg(&marker);
        let error = capture(&mut command, Duration::ZERO, 64).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(!marker.exists());
    }
}
