//! Runs a voice command's plan: one program with fixed arguments, never a
//! shell, on the calling (worker) thread with bounded waits.
use crate::command::{Plan, Run};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Why a command did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fail {
    /// The program is not installed.
    Missing(String),
    /// `niructl` predates `niructl do` (NIRUWM without voice command support).
    OldWm,
    /// NIRULAUNCHER did not answer like 0.14 (`--note --stdin`).
    OldLauncher,
    /// It ran and said no (first line of its error output).
    Failed(String),
}

/// What happened, for the overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    /// It ran.
    Ran,
    /// The launcher started this program.
    Launched(String),
    /// The launcher showed its list (no clear match).
    Shown,
}

/// Find a program on PATH or in ~/.local/bin.
pub fn find(program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.push(crate::config::home().join(".local/bin"));
    dirs.into_iter()
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

fn spawn(argv: &[String], stdin: bool, stdout: bool, stderr: bool) -> Result<Child, Fail> {
    let path = find(&argv[0]).ok_or_else(|| Fail::Missing(argv[0].clone()))?;
    let pipe = |on: bool| if on { Stdio::piped() } else { Stdio::null() };
    Command::new(path)
        .args(&argv[1..])
        .stdin(pipe(stdin))
        .stdout(pipe(stdout))
        .stderr(pipe(stderr))
        // Its own process group: programs it starts outlive this service.
        .process_group(0)
        .spawn()
        .map_err(|e| Fail::Failed(format!("{}: {e}", argv[0])))
}

/// Read a pipe to the end on a thread (a program it started may keep it
/// open; nothing may block on that), keeping the first 4 KB.
fn collect(pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("nirudictate-cmd-out".into())
        .spawn(move || {
            let mut kept = Vec::new();
            let mut buf = [0u8; 1024];
            let mut pipe = pipe;
            let mut sent = false;
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if kept.len() < 4096 {
                            kept.extend_from_slice(&buf[..n]);
                        }
                    }
                }
                if !sent && kept.len() >= 4096 {
                    let _ = tx.send(String::from_utf8_lossy(&kept).into_owned());
                    sent = true;
                }
            }
            if !sent {
                let _ = tx.send(String::from_utf8_lossy(&kept).into_owned());
            }
        });
    rx
}

fn reap(mut child: Child) {
    let _ = std::thread::Builder::new()
        .name("nirudictate-cmd-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

/// Wait for the child up to `limit`; None when it is still running.
fn wait(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(15)),
            _ => return None,
        }
    }
}

/// The first useful line of an error output.
pub fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    let line = line
        .strip_prefix("niructl: ")
        .or_else(|| line.strip_prefix("nirulauncher: "))
        .unwrap_or(&line)
        .to_string();
    line.chars().take(120).collect()
}

/// Whether `niructl` answered like a version without `do`.
pub fn old_niructl(argv: &[String], stderr: &str) -> bool {
    argv.first().is_some_and(|p| p == "niructl")
        && argv.get(1).is_some_and(|a| a == "do")
        && stderr.contains("Usage: niructl")
}

fn run_once(argv: &[String], plan: &Plan) -> Result<Done, Fail> {
    match plan.run {
        Run::Ui => {
            let child = spawn(argv, false, false, false)?;
            reap(child);
            Ok(Done::Ran)
        }
        Run::LaunchBest => {
            let mut child = spawn(argv, false, true, false)?;
            let stdout = child.stdout.take().expect("piped stdout");
            // The first line says what happened; the rest is drained so the
            // program never meets a closed pipe.
            let (tx, rx) = mpsc::channel();
            let _ = std::thread::Builder::new()
                .name("nirudictate-cmd-line".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line);
                    let _ = tx.send(line);
                    let _ = std::io::copy(&mut reader, &mut std::io::sink());
                });
            let line = rx.recv_timeout(Duration::from_secs(4)).unwrap_or_default();
            reap(child);
            let line = line.trim_end();
            if let Some(name) = line.strip_prefix("launched\t") {
                return Ok(Done::Launched(name.trim().to_string()));
            }
            // "shown", or an older launcher that opened its list silently.
            Ok(Done::Shown)
        }
        Run::Wait => {
            let mut child = spawn(argv, plan.stdin.is_some(), false, true)?;
            if let (Some(text), Some(mut stdin)) = (&plan.stdin, child.stdin.take()) {
                let _ = stdin.write_all(text.as_bytes());
                // Dropping it closes standard input.
            }
            let stderr = collect(child.stderr.take().expect("piped stderr"));
            // Power actions may wait for the polkit agent; never kill them.
            let limit = if argv
                .get(1)
                .is_some_and(|a| matches!(a.as_str(), "suspend" | "reboot" | "shutdown" | "logout"))
            {
                Duration::from_secs(150)
            } else {
                Duration::from_secs(8)
            };
            let status = wait(&mut child, limit);
            let Some(status) = status else {
                reap(child);
                // A note that is still running is an old launcher showing
                // its note window instead of saving.
                if plan.stdin.is_some() {
                    return Err(Fail::OldLauncher);
                }
                return Ok(Done::Ran);
            };
            if status.success() {
                return Ok(Done::Ran);
            }
            let text = stderr
                .recv_timeout(Duration::from_millis(300))
                .unwrap_or_default();
            if old_niructl(argv, &text) {
                return Err(Fail::OldWm);
            }
            let line = first_line(&text);
            Err(Fail::Failed(if line.is_empty() {
                format!("{} exited with {status}", argv[0])
            } else {
                line
            }))
        }
    }
}

/// Run a plan (after its delay), with the fallback for an older `niructl`.
pub fn run(plan: &Plan) -> Result<Done, Fail> {
    if plan.delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(plan.delay_ms));
    }
    match run_once(&plan.argv, plan) {
        Err(Fail::OldWm) => match &plan.fallback {
            Some(fallback) => run_once(fallback, plan),
            None => Err(Fail::OldWm),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(argv: &[&str], run: Run) -> Plan {
        Plan {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            stdin: None,
            run,
            dangerous: false,
            hide_overlay: false,
            delay_ms: 0,
            fallback: None,
        }
    }

    #[test]
    fn reports_missing_programs_and_failures() {
        assert_eq!(
            run(&plan(&["nirudictate-no-such-program"], Run::Wait)),
            Err(Fail::Missing("nirudictate-no-such-program".into()))
        );
        assert_eq!(run(&plan(&["true"], Run::Wait)), Ok(Done::Ran));
        match run(&plan(&["ls", "/nirudictate/no/such/dir"], Run::Wait)) {
            Err(Fail::Failed(line)) => assert!(line.contains("nirudictate"), "{line}"),
            other => panic!("{other:?}"),
        }
        // Standard input reaches the program (a note).
        let mut p = plan(&["grep", "-q", "mjölk"], Run::Wait);
        p.stdin = Some("köp mjölk".into());
        assert_eq!(run(&p), Ok(Done::Ran));
        p.stdin = Some("köp bröd".into());
        assert!(matches!(run(&p), Err(Fail::Failed(_))));
    }

    #[test]
    fn reads_what_the_launcher_did() {
        let p = plan(&["printf", "launched\\tSpotify\\n"], Run::LaunchBest);
        assert_eq!(run(&p), Ok(Done::Launched("Spotify".into())));
        let p = plan(&["printf", "shown\\n"], Run::LaunchBest);
        assert_eq!(run(&p), Ok(Done::Shown));
    }

    #[test]
    fn knows_an_old_niructl() {
        let argv: Vec<String> = ["niructl", "do", "terminal"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(old_niructl(
            &argv,
            "niructl: Usage: niructl status | tag N | movetotag N | …"
        ));
        assert!(!old_niructl(
            &argv,
            "niructl: refused: shell runs any program"
        ));
        assert_eq!(
            first_line("\nniructl: unknown action: frobnicate\nmore"),
            "unknown action: frobnicate"
        );
    }
}
