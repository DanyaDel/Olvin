//! Background `cargo check|build|run`: output lines arrive over a channel,
//! the UI thread only drains it. The process runs in its own process group so
//! Stop also kills the game `cargo run` started.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Check,
    Build,
    Run,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Check => "Check",
            Self::Build => "Build",
            Self::Run => "Run",
        }
    }

    fn subcommand(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
            Self::Run => "run",
        }
    }

    /// `cargo <kind> --message-format=json-diagnostic-rendered-ansi` in `dir`.
    pub fn command(self, dir: &Path) -> Command {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.arg(self.subcommand())
            .arg("--message-format=json-diagnostic-rendered-ansi")
            .current_dir(dir)
            .env("CARGO_TERM_PROGRESS_WHEN", "never");
        cmd
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, PartialEq)]
pub enum JobMsg {
    Line(Stream, String),
    /// Exit code (`None` = killed by a signal).
    Exit(Option<i32>),
}

pub struct Job {
    pub kind: JobKind,
    pub started: Instant,
    pid: u32,
    rx: Receiver<JobMsg>,
    /// Exit code and run time once finished.
    pub finished: Option<(Option<i32>, Duration)>,
    kills: u32,
}

impl Job {
    pub fn spawn(kind: JobKind, mut cmd: Command) -> std::io::Result<Self> {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let (tx, rx) = channel();
        let out = child
            .stdout
            .take()
            .map(|s| reader(s, Stream::Stdout, tx.clone()));
        let err = child
            .stderr
            .take()
            .map(|s| reader(s, Stream::Stderr, tx.clone()));
        std::thread::Builder::new()
            .name("olvin-ide-job".into())
            .spawn(move || {
                let status = child.wait();
                // Deliver every line before the exit message.
                for t in [out, err].into_iter().flatten() {
                    let _ = t.join();
                }
                let _ = tx.send(JobMsg::Exit(status.ok().and_then(|s| s.code())));
            })?;
        Ok(Self {
            kind,
            started: Instant::now(),
            pid,
            rx,
            finished: None,
            kills: 0,
        })
    }

    pub fn running(&self) -> bool {
        self.finished.is_none()
    }

    /// Everything that arrived since the last call (at most `limit` messages,
    /// so a flood of output can't stall a frame).
    pub fn poll(&mut self, limit: usize) -> Vec<JobMsg> {
        let mut out = Vec::new();
        while out.len() < limit {
            match self.rx.try_recv() {
                Ok(msg) => {
                    if let JobMsg::Exit(code) = msg {
                        self.finished = Some((code, self.started.elapsed()));
                    }
                    out.push(msg);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.finished.is_none() {
                        self.finished = Some((None, self.started.elapsed()));
                        out.push(JobMsg::Exit(None));
                    }
                    break;
                }
            }
        }
        out
    }

    /// Stops the process group: SIGTERM first, SIGKILL on the next call.
    pub fn kill(&mut self) {
        if !self.running() {
            return;
        }
        self.kills += 1;
        #[cfg(unix)]
        {
            let signal = if self.kills > 1 { "-KILL" } else { "-TERM" };
            let _ = Command::new("kill")
                .args([signal, "--", &format!("-{}", self.pid)])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &self.pid.to_string()])
                .status();
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.running() {
            self.kills = 1; // straight to SIGKILL
            self.kill();
        }
    }
}

fn reader(
    stream: impl Read + Send + 'static,
    kind: Stream,
    tx: Sender<JobMsg>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']).to_owned();
                    if tx.send(JobMsg::Line(kind, line)).is_err() {
                        return;
                    }
                }
            }
        }
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn drain(job: &mut Job) -> Vec<JobMsg> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while job.running() && Instant::now() < deadline {
            all.extend(job.poll(100));
            std::thread::sleep(Duration::from_millis(5));
        }
        all
    }

    #[test]
    fn streams_lines_and_exit_code() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo one; echo two >&2; echo three; exit 3"]);
        let mut job = Job::spawn(JobKind::Check, cmd).unwrap();
        let msgs = drain(&mut job);
        assert!(msgs.contains(&JobMsg::Line(Stream::Stdout, "one".into())));
        assert!(msgs.contains(&JobMsg::Line(Stream::Stderr, "two".into())));
        assert_eq!(msgs.last(), Some(&JobMsg::Exit(Some(3))));
        assert!(!job.running());
    }

    #[test]
    fn kill_stops_the_process_group() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & wait"]);
        let mut job = Job::spawn(JobKind::Run, cmd).unwrap();
        job.kill();
        let msgs = drain(&mut job);
        assert!(!job.running(), "still running after kill");
        assert_eq!(msgs.last(), Some(&JobMsg::Exit(None)));
        assert!(job.finished.unwrap().1 < Duration::from_secs(10));
    }
}
