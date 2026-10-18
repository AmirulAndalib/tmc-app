//! The Console screen's engine: find `tmc`, split a typed line into an argument
//! vector, run it, and stream what it prints.
//!
//! THE RULE THIS BENDS, AND HOW FAR
//! --------------------------------
//! "No command takes a program or a path from the webview" still holds: the
//! webview sends ONE STRING, and everything below is decided in Rust.
//!
//!   * **The program is never the webview's.** It is `tmc`, looked up on `PATH`
//!     by [`find_program`]. A typed leading `tmc` is stripped by
//!     [`split_argv`]; any other first word is simply an argument to `tmc`.
//!   * **No shell.** The line becomes an argv vector and goes to
//!     `Command::args`; `;`, `|`, `&&`, backticks and `$(…)` are inert bytes to
//!     the child. Quoting is only a way to write an argument with a space in it.
//!   * **The working directory and environment are the app's.** The child runs
//!     in the user's home directory with a fixed set of extra variables
//!     ([`FIXED_ENV`]); nothing from the webview reaches either.
//!
//! What it still permits, and it is a lot: `tmc` can upload any file the user
//! can read (`tmc file upload ~/.ssh/id_ed25519`) and can send the stored key
//! to a site named by `--base-url`. That is the capability being offered, and
//! it is the same a terminal gives — the cost is that a script running in the
//! webview can now do what the user could type. The compensating controls are
//! that every run is audited with secrets redacted ([`redact_argv`]), only one
//! process runs at a time, and the screen exists only on desktop builds.
//!
//! No Tauri dependency: the shell turns [`ConsoleEvent`] into webview events.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// The program being run, as a bare name for the `PATH` lookup.
pub const PROGRAM: &str = "tmc";

/// Longest line accepted. A command line is typed; a megabyte of it is not.
pub const MAX_LINE: usize = 8 * 1024;
/// Most arguments one line may split into.
pub const MAX_ARGS: usize = 256;
/// Most bytes one stdin write may carry.
pub const MAX_STDIN: usize = 64 * 1024;

/// Variables set on the child, whatever the app's own environment says.
///
/// `PYTHONUNBUFFERED` because a piped Python process block-buffers stdout, so
/// a progress line would arrive when the process ends, which defeats a live
/// transcript. `TMC_COLOR=always` because the CLI colours only on a terminal,
/// and the screen renders ANSI; `NO_COLOR`, if the user set it, still wins.
pub const FIXED_ENV: [(&str, &str); 3] = [
    ("PYTHONUNBUFFERED", "1"),
    ("PYTHONIOENCODING", "utf-8"),
    ("TMC_COLOR", "always"),
];

// ---- Splitting ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    TooLong,
    TooMany,
    /// A quote that never closes; the character is the quote.
    Unterminated(char),
    /// A trailing backslash with nothing to escape.
    DanglingEscape,
    /// A control character other than whitespace. A NUL would truncate an
    /// argument in the child, which is a way to hide the tail of a command.
    Control,
    /// Nothing but (at most) the program name.
    Empty,
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong => write!(f, "That command line is too long."),
            Self::TooMany => write!(f, "That command has too many arguments."),
            Self::Unterminated(q) => write!(f, "A {q} quote is never closed."),
            Self::DanglingEscape => write!(f, "The line ends with a backslash."),
            Self::Control => write!(f, "The line contains a control character."),
            Self::Empty => write!(f, "Type a command, for example --help."),
        }
    }
}

/// Split a typed line into arguments with shell-like quoting, and drop a leading
/// `tmc`.
///
/// `posix_escapes` is true everywhere except Windows. A POSIX shell treats an
/// unquoted backslash as "the next character is literal"; on Windows that would
/// eat the separators of every path (`C:\Users\me` becomes `C:Usersme`), so
/// there a backslash is literal and only `\"` is an escape.
///
/// Rules, both modes: whitespace separates; `'…'` is literal; `"…"` is literal
/// except `\"` (and, with POSIX escapes, `\\`, `\$` and `` \` ``); adjacent
/// quoted and bare runs join into one argument; `""` is an empty argument.
pub fn split_argv(line: &str, posix_escapes: bool) -> Result<Vec<String>, SplitError> {
    if line.len() > MAX_LINE {
        return Err(SplitError::TooLong);
    }

    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    // Distinguishes "no argument yet" from "an empty argument" (`""`).
    let mut started = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\t' | '\n' | '\r' | ' ' => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c if c.is_control() => return Err(SplitError::Control),
            '\'' => {
                started = true;

                loop {
                    match chars.next() {
                        None => return Err(SplitError::Unterminated('\'')),
                        Some('\'') => break,
                        Some(c) if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {
                            return Err(SplitError::Control)
                        }
                        Some(c) => cur.push(c),
                    }
                }
            }
            '"' => {
                started = true;

                loop {
                    match chars.next() {
                        None => return Err(SplitError::Unterminated('"')),
                        Some('"') => break,
                        Some('\\') => match chars.peek().copied() {
                            Some('"') => {
                                cur.push('"');
                                chars.next();
                            }
                            Some(n @ ('\\' | '$' | '`')) if posix_escapes => {
                                cur.push(n);
                                chars.next();
                            }
                            _ => cur.push('\\'),
                        },
                        Some(c) if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {
                            return Err(SplitError::Control)
                        }
                        Some(c) => cur.push(c),
                    }
                }
            }
            '\\' => {
                started = true;

                match chars.peek().copied() {
                    Some('"') => {
                        cur.push('"');
                        chars.next();
                    }
                    Some(n) if posix_escapes && !n.is_control() => {
                        cur.push(n);
                        chars.next();
                    }
                    None if posix_escapes => return Err(SplitError::DanglingEscape),
                    _ => cur.push('\\'),
                }
            }
            c => {
                started = true;
                cur.push(c);
            }
        }

        if args.len() > MAX_ARGS {
            return Err(SplitError::TooMany);
        }
    }

    if started {
        args.push(cur);
    }

    if args.len() > MAX_ARGS {
        return Err(SplitError::TooMany);
    }

    // Only the first word, and only the bare name: `tmc` the user typed because
    // the prompt looks like a shell. It cannot select a different program, as
    // the program is fixed either way.
    if args
        .first()
        .is_some_and(|a| a.eq_ignore_ascii_case(PROGRAM) || a.eq_ignore_ascii_case("tmc.exe"))
    {
        args.remove(0);
    }

    if args.is_empty() {
        return Err(SplitError::Empty);
    }

    Ok(args)
}

// ---- Redaction ---------------------------------------------------------------

const REDACTED: &str = "«redacted»";

/// Words that make a flag or a `key=value` key name a secret.
const SECRET_WORDS: [&str; 7] = [
    "token",
    "secret",
    "password",
    "passwd",
    "authorization",
    "api-key",
    "apikey",
];

fn is_secret_name(name: &str) -> bool {
    let lower = name
        .trim_start_matches('-')
        .to_ascii_lowercase()
        .replace('_', "-");

    SECRET_WORDS.iter().any(|w| lower.contains(w))
}

/// Looks like a value that is a credential whatever flag carried it: a bearer
/// key (`tmc_…`), or a JWT (three base64url runs, the first starting `eyJ`).
fn looks_like_secret(value: &str) -> bool {
    if let Some(rest) = value.strip_prefix("tmc_") {
        return rest.len() >= 8
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    }

    value.starts_with("eyJ") && value.matches('.').count() == 2
}

/// The argument vector with anything credential-shaped replaced, for the audit
/// log and nothing else.
///
/// Three shapes: `--token VALUE` (the value after a flag whose name says
/// secret), `--token=VALUE`, and `--set token=VALUE` (a `key=value` whose key
/// says secret); plus any bare argument that looks like a key or a JWT. A
/// private key is passed by FILE (`--private-key path`), which is not a secret
/// and is kept, because "which key file was used" is the useful audit fact.
pub fn redact_argv(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut hide_next = false;

    for arg in args {
        if hide_next {
            hide_next = false;
            out.push(REDACTED.to_string());

            continue;
        }

        if arg.starts_with('-') {
            match arg.split_once('=') {
                Some((name, value)) if is_secret_name(name) || looks_like_secret(value) => {
                    out.push(format!("{name}={REDACTED}"));
                }
                Some(_) => out.push(arg.clone()),
                None => {
                    hide_next = is_secret_name(arg);
                    out.push(arg.clone());
                }
            }

            continue;
        }

        if let Some((k, _)) = arg.split_once('=') {
            if is_secret_name(k) {
                out.push(format!("{k}={REDACTED}"));

                continue;
            }
        }

        out.push(if looks_like_secret(arg) {
            REDACTED.to_string()
        } else {
            arg.clone()
        });
    }

    out
}

// ---- Finding the program -----------------------------------------------------

/// Look `name` up in `path_var`, the way a shell would, and nowhere else.
///
/// The current directory is deliberately NOT searched: on Windows `CreateProcess`
/// does that by default, which is how a `tmc.exe` dropped into a download folder
/// gets run by something that meant the installed one. `pathext` is the
/// `PATHEXT` value (Windows only); pip installs `tmc.exe` there, but a `.cmd`
/// or `.bat` shim is accepted when that is what the user has.
pub fn find_program(
    name: &str,
    path_var: Option<&OsStr>,
    pathext: Option<&OsStr>,
    windows: bool,
) -> Option<PathBuf> {
    let path_var = path_var?;

    let exts: Vec<String> = if windows {
        let raw = pathext
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());

        let mut list: Vec<String> = raw
            .split(';')
            .filter(|e| e.starts_with('.') && e.len() > 1)
            .map(str::to_ascii_lowercase)
            .collect();

        // A script with no extension is what some installers leave behind; it
        // cannot be started directly on Windows, so it is not offered.
        list.retain(|e| matches!(e.as_str(), ".com" | ".exe" | ".bat" | ".cmd"));

        list
    } else {
        vec![String::new()]
    };

    for dir in std::env::split_paths(path_var) {
        // An empty PATH entry means "here" on Unix; skipped for the reason above.
        if dir.as_os_str().is_empty() || !dir.is_absolute() {
            continue;
        }

        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));

            if is_runnable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(unix)]
fn is_runnable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_runnable(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// `tmc` on this process's `PATH`.
pub fn locate() -> Option<PathBuf> {
    find_program(
        PROGRAM,
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("PATHEXT").as_deref(),
        cfg!(windows),
    )
}

/// Where the child starts: the user's home, never a webview-chosen directory.
fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };

    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
}

fn command(program: &Path, args: &[String]) -> Command {
    let mut cmd = Command::new(program);

    cmd.args(args);

    for (k, v) in FIXED_ENV {
        cmd.env(k, v);
    }

    if let Some(home) = home_dir() {
        cmd.current_dir(home);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // CREATE_NO_WINDOW: a console flashing up for every run.
        cmd.creation_flags(0x0800_0000);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // Its own process group, so Stop reaches whatever it spawned.
        cmd.process_group(0);
    }

    cmd
}

/// What `tmc --version` printed, trimmed, or `None` when it did not run.
///
/// Bounded: a hung interpreter must not hang the screen.
pub fn version_of(program: &Path) -> Option<String> {
    let mut child = command(program, &["--version".to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + Duration::from_secs(8);

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                kill_tree(child.id());
                let _ = child.wait();

                return None;
            }
        }
    }

    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() {
        &out.stderr
    } else {
        &out.stdout
    })
    .trim()
    .to_string();

    // First line only, and not empty: this goes in a header.
    text.lines()
        .next()
        .map(|l| l.chars().take(120).collect())
        .filter(|l: &String| !l.is_empty())
}

// ---- Running -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub enum ConsoleEvent {
    Output {
        run_id: u64,
        stream: Stream,
        text: String,
    },
    Exit {
        run_id: u64,
        code: Option<i32>,
        stopped: bool,
        millis: u64,
    },
}

pub type Emit = Arc<dyn Fn(ConsoleEvent) + Send + Sync>;

#[derive(Debug)]
pub enum StartError {
    /// Another run is in progress.
    Busy,
    Spawn(std::io::Error),
}

struct Active {
    run_id: u64,
    pid: u32,
    stdin: Option<ChildStdin>,
    stopped: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

/// One process at a time.
#[derive(Default)]
pub struct Console {
    active: Arc<Mutex<Option<Active>>>,
    next_id: AtomicU64,
}

impl Console {
    pub fn new() -> Self {
        Self::default()
    }

    /// The run in progress, if any.
    pub fn running(&self) -> Option<u64> {
        self.active.lock().ok()?.as_ref().map(|a| a.run_id)
    }

    /// Start `program args…`, streaming through `emit`. Returns the run id.
    ///
    /// `emit` gets every [`ConsoleEvent::Output`] in order per stream and then
    /// exactly one [`ConsoleEvent::Exit`], after both pipes have drained (or
    /// after a short grace, when a grandchild holds one open).
    pub fn start(&self, program: &Path, args: &[String], emit: Emit) -> Result<u64, StartError> {
        let mut slot = self.active.lock().map_err(|_| StartError::Busy)?;

        if slot.is_some() {
            return Err(StartError::Busy);
        }

        let mut child: Child = command(program, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(StartError::Spawn)?;

        let run_id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let stopped = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));

        *slot = Some(Active {
            run_id,
            pid: child.id(),
            stdin: child.stdin.take(),
            stopped: Arc::clone(&stopped),
            finished: Arc::clone(&finished),
        });

        drop(slot);

        let (done_tx, done_rx) = mpsc::channel::<()>();

        for (stream, pipe) in [
            (
                Stream::Stdout,
                child
                    .stdout
                    .take()
                    .map(|p| Box::new(p) as Box<dyn Read + Send>),
            ),
            (
                Stream::Stderr,
                child
                    .stderr
                    .take()
                    .map(|p| Box::new(p) as Box<dyn Read + Send>),
            ),
        ] {
            let emit = Arc::clone(&emit);
            let done = done_tx.clone();

            std::thread::spawn(move || {
                if let Some(pipe) = pipe {
                    pump(pipe, |text| {
                        emit(ConsoleEvent::Output {
                            run_id,
                            stream,
                            text,
                        })
                    });
                }

                let _ = done.send(());
            });
        }

        drop(done_tx);

        let started = Instant::now();
        let (code_tx, code_rx) = mpsc::channel::<Option<i32>>();

        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());

            finished.store(true, Ordering::SeqCst);

            let _ = code_tx.send(code);
        });

        let this = Arc::clone(&self.active);

        std::thread::spawn(move || {
            let code = code_rx.recv().ok().flatten();

            // Both readers, but not forever: a grandchild that inherited a pipe
            // and outlives the child would otherwise keep the run "running".
            let grace = Instant::now() + Duration::from_millis(1500);

            for _ in 0..2 {
                let left = grace.saturating_duration_since(Instant::now());

                if done_rx.recv_timeout(left).is_err() {
                    break;
                }
            }

            // Cleared BEFORE the exit event: the screen starts its next run the
            // moment it hears this one ended, and must not be told Busy.
            if let Ok(mut slot) = this.lock() {
                *slot = None;
            }

            emit(ConsoleEvent::Exit {
                run_id,
                code,
                stopped: stopped.load(Ordering::SeqCst),
                millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            });
        });

        Ok(run_id)
    }

    /// Write `text` to the running process's stdin. `false` when nothing runs.
    pub fn write_stdin(&self, text: &str) -> bool {
        let Ok(mut slot) = self.active.lock() else {
            return false;
        };

        let Some(stdin) = slot.as_mut().and_then(|a| a.stdin.as_mut()) else {
            return false;
        };

        stdin
            .write_all(text.as_bytes())
            .and_then(|()| stdin.flush())
            .is_ok()
    }

    /// Close stdin (end of input). `false` when nothing runs.
    pub fn close_stdin(&self) -> bool {
        let Ok(mut slot) = self.active.lock() else {
            return false;
        };

        slot.as_mut().is_some_and(|a| a.stdin.take().is_some())
    }

    /// Ask the running process to end, and kill it if it ignores that.
    pub fn stop(&self) -> bool {
        let Ok(slot) = self.active.lock() else {
            return false;
        };

        let Some(active) = slot.as_ref() else {
            return false;
        };

        if active.finished.load(Ordering::SeqCst) {
            return false;
        }

        active.stopped.store(true, Ordering::SeqCst);

        let pid = active.pid;
        let finished = Arc::clone(&active.finished);

        drop(slot);

        terminate(pid);

        // A polite signal first on Unix; the hard kill if it is still there.
        #[cfg(unix)]
        std::thread::spawn(move || {
            for _ in 0..20 {
                if finished.load(Ordering::SeqCst) {
                    return;
                }

                std::thread::sleep(Duration::from_millis(100));
            }

            if !finished.load(Ordering::SeqCst) {
                kill_tree(pid);
            }
        });

        #[cfg(not(unix))]
        let _ = finished;

        true
    }
}

/// Read a pipe in chunks and hand each over as text, holding back a trailing
/// partial UTF-8 sequence until its remaining bytes arrive.
fn pump<R: Read>(mut pipe: R, mut emit: impl FnMut(String)) {
    let mut buf = [0u8; 4096];
    let mut carry: Vec<u8> = Vec::new();

    loop {
        let n = match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };

        carry.extend_from_slice(&buf[..n]);

        let (text, rest) = decode_prefix(&carry);

        if !text.is_empty() {
            emit(text);
        }

        carry = rest;
    }

    if !carry.is_empty() {
        emit(String::from_utf8_lossy(&carry).into_owned());
    }
}

/// Decode as much of `bytes` as is complete. Invalid bytes become U+FFFD; an
/// incomplete tail (at most three bytes) is returned for the next chunk.
fn decode_prefix(bytes: &[u8]) -> (String, Vec<u8>) {
    let mut out = String::new();
    let mut rest = bytes;

    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);

                return (out, Vec::new());
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());

                out.push_str(&String::from_utf8_lossy(good));

                match e.error_len() {
                    Some(len) => {
                        out.push('\u{fffd}');
                        rest = &bad[len..];
                    }
                    None => return (out, bad.to_vec()),
                }
            }
        }
    }
}

// ---- Ending a process --------------------------------------------------------

/// Ask politely: SIGTERM to the group on Unix; on Windows there is no polite
/// form for a console-less child, so it is the tree kill.
fn terminate(pid: u32) {
    #[cfg(unix)]
    signal_group(pid, libc::SIGTERM);

    #[cfg(not(unix))]
    kill_tree(pid);
}

/// Kill the process and everything it started.
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    signal_group(pid, libc::SIGKILL);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // `/T` is the tree, `/F` is force. A Python entry-point script is
        // `tmc.exe` launching the interpreter, so killing only the pid would
        // leave the interpreter running.
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, sig: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };

    if pid <= 1 {
        return;
    }

    // SAFETY: a plain syscall. The child was started with `process_group(0)`,
    // so its pid is its group id and `-pid` addresses the group only.
    unsafe {
        libc::kill(-pid, sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(line: &str) -> Result<Vec<String>, SplitError> {
        split_argv(line, true)
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_on_whitespace() {
        assert_eq!(sp("mod list --mine"), Ok(v(&["mod", "list", "--mine"])));
        assert_eq!(
            sp("  mod\t list \n --all "),
            Ok(v(&["mod", "list", "--all"]))
        );
    }

    #[test]
    fn strips_a_leading_tmc_only() {
        assert_eq!(sp("tmc auth whoami"), Ok(v(&["auth", "whoami"])));
        assert_eq!(sp("TMC.exe --help"), Ok(v(&["--help"])));
        assert_eq!(sp("'tmc' --help"), Ok(v(&["--help"])));
        // Only the first word; later ones are arguments.
        assert_eq!(sp("mod get tmc"), Ok(v(&["mod", "get", "tmc"])));
        assert_eq!(sp("tmc tmc"), Ok(v(&["tmc"])));
    }

    #[test]
    fn empty_is_an_error() {
        assert_eq!(sp(""), Err(SplitError::Empty));
        assert_eq!(sp("   "), Err(SplitError::Empty));
        assert_eq!(sp("tmc"), Err(SplitError::Empty));
    }

    #[test]
    fn quotes() {
        assert_eq!(
            sp(r#"mod update 42 --set description="Now with fewer bugs""#),
            Ok(v(&[
                "mod",
                "update",
                "42",
                "--set",
                "description=Now with fewer bugs"
            ]))
        );
        assert_eq!(sp("a 'b  c' d"), Ok(v(&["a", "b  c", "d"])));
        assert_eq!(sp(r#"a "b 'c' d""#), Ok(v(&["a", "b 'c' d"])));
        assert_eq!(sp("a 'it''s'"), Ok(v(&["a", "its"])));
        assert_eq!(sp(r#"x ""  y"#), Ok(v(&["x", "", "y"])));
        assert_eq!(sp("x pre\"mid dle\"post"), Ok(v(&["x", "premid dlepost"])));
    }

    #[test]
    fn escapes() {
        assert_eq!(sp(r"a b\ c"), Ok(v(&["a", "b c"])));
        assert_eq!(sp(r#"a "q\"q" "s\\s""#), Ok(v(&["a", "q\"q", "s\\s"])));
        // Inside double quotes an unknown escape keeps its backslash.
        assert_eq!(sp(r#"a "x\ny""#), Ok(v(&["a", "x\\ny"])));
        // Single quotes take everything literally.
        assert_eq!(sp(r"a 'x\ny'"), Ok(v(&["a", "x\\ny"])));
    }

    #[test]
    fn windows_paths_keep_their_backslashes() {
        assert_eq!(
            split_argv(r"file upload C:\Users\me\a.zip", false),
            Ok(v(&["file", "upload", r"C:\Users\me\a.zip"]))
        );
        assert_eq!(
            split_argv(r#"file upload "C:\Program Files\a.zip""#, false),
            Ok(v(&["file", "upload", r"C:\Program Files\a.zip"]))
        );
    }

    #[test]
    fn shell_syntax_is_inert() {
        assert_eq!(
            sp("mod get 1; rm -rf ~ | cat && $(id) `id`"),
            Ok(v(&[
                "mod", "get", "1;", "rm", "-rf", "~", "|", "cat", "&&", "$(id)", "`id`"
            ]))
        );
    }

    #[test]
    fn refuses_what_it_cannot_split_safely() {
        assert_eq!(sp("a 'b"), Err(SplitError::Unterminated('\'')));
        assert_eq!(sp("a \"b"), Err(SplitError::Unterminated('"')));
        assert_eq!(sp("a b\\"), Err(SplitError::DanglingEscape));
        assert_eq!(sp("a b\0c"), Err(SplitError::Control));
        assert_eq!(sp("a 'b\u{7}c'"), Err(SplitError::Control));
        assert_eq!(sp(&"x".repeat(MAX_LINE + 1)), Err(SplitError::TooLong));
        assert_eq!(sp(&"a ".repeat(MAX_ARGS + 1)), Err(SplitError::TooMany));
    }

    #[test]
    fn redacts_flag_values() {
        assert_eq!(
            redact_argv(&v(&["auth", "login", "--token", "tmc_abcdef123456"])),
            v(&["auth", "login", "--token", REDACTED])
        );
        assert_eq!(
            redact_argv(&v(&["auth", "login", "--token=hunter2"])),
            v(&["auth", "login", &format!("--token={REDACTED}")])
        );
        assert_eq!(
            redact_argv(&v(&["x", "--api-key", "k", "--password", "p"])),
            v(&["x", "--api-key", REDACTED, "--password", REDACTED])
        );
    }

    #[test]
    fn redacts_set_pairs_and_lookalikes() {
        assert_eq!(
            redact_argv(&v(&[
                "mod",
                "update",
                "1",
                "--set",
                "apiToken=abc",
                "--set",
                "name=Fine"
            ])),
            v(&[
                "mod",
                "update",
                "1",
                "--set",
                &format!("apiToken={REDACTED}"),
                "--set",
                "name=Fine"
            ])
        );
        assert_eq!(
            redact_argv(&v(&["get", "tmc_abcdefgh12"])),
            v(&["get", REDACTED])
        );
        assert_eq!(
            redact_argv(&v(&["get", "eyJhbGciOi.payload.sig"])),
            v(&["get", REDACTED])
        );
        assert_eq!(
            redact_argv(&v(&["--key=tmc_abcdefgh12"])),
            v(&[&format!("--key={REDACTED}")])
        );
    }

    #[test]
    fn keeps_what_is_not_secret() {
        let plain = v(&[
            "mod",
            "list",
            "--mine",
            "--private-key",
            "/home/me/k.pem",
            "--key-id",
            "tmcak_x",
            "tmc_x",
        ]);

        assert_eq!(redact_argv(&plain), plain);
    }

    #[test]
    fn decodes_split_utf8() {
        let bytes = "héllo ✓".as_bytes();
        // Cut inside the 3-byte check mark.
        let (a, rest) = decode_prefix(&bytes[..bytes.len() - 1]);

        assert_eq!(a, "héllo ");
        assert_eq!(rest.len(), 2);

        let mut joined = rest;
        joined.push(bytes[bytes.len() - 1]);

        assert_eq!(decode_prefix(&joined), ("✓".to_string(), Vec::new()));
        assert_eq!(
            decode_prefix(b"a\xffb"),
            ("a\u{fffd}b".to_string(), Vec::new())
        );
    }

    #[test]
    fn path_lookup_ignores_the_current_directory() {
        let dir = std::env::temp_dir().join(format!("tmc-console-{}", std::process::id()));
        let bin = dir.join("bin");

        std::fs::create_dir_all(&bin).unwrap();

        let name = if cfg!(windows) { "tmc.exe" } else { "tmc" };
        let file = bin.join(name);

        std::fs::write(&file, b"x").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // Present but not executable: not a candidate.
            assert_eq!(
                find_program("tmc", Some(bin.as_os_str()), None, false),
                None
            );

            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let joined =
            std::env::join_paths([PathBuf::from("relative"), PathBuf::new(), bin.clone()]).unwrap();

        assert_eq!(
            find_program(
                "tmc",
                Some(&joined),
                Some(OsStr::new(".EXE;.CMD")),
                cfg!(windows)
            ),
            Some(file.clone())
        );
        assert_eq!(
            find_program("nope", Some(&joined), None, cfg!(windows)),
            None
        );
        assert_eq!(find_program("tmc", None, None, cfg!(windows)), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn runs_streams_and_stops() {
        let events: Arc<Mutex<Vec<ConsoleEvent>>> = Arc::default();
        let sink = Arc::clone(&events);
        let emit: Emit = Arc::new(move |e| sink.lock().unwrap().push(e));

        let console = Console::new();
        let sh = Path::new("/bin/sh");

        let id = console
            .start(
                sh,
                &v(&["-c", "echo out; echo err 1>&2; read x; echo got:$x; exit 3"]),
                Arc::clone(&emit),
            )
            .unwrap();

        assert!(matches!(
            console.start(sh, &v(&["-c", "true"]), Arc::clone(&emit)),
            Err(StartError::Busy)
        ));
        assert!(console.write_stdin("hi\n"));

        wait_exit(&events);

        let all = events.lock().unwrap().clone();
        let text = |s: Stream| {
            all.iter()
                .filter_map(|e| match e {
                    ConsoleEvent::Output { stream, text, .. } if *stream == s => Some(text.clone()),
                    _ => None,
                })
                .collect::<String>()
        };

        assert_eq!(text(Stream::Stdout), "out\ngot:hi\n");
        assert_eq!(text(Stream::Stderr), "err\n");
        assert!(matches!(
            all.last(),
            Some(ConsoleEvent::Exit { run_id, code: Some(3), stopped: false, .. }) if *run_id == id
        ));

        events.lock().unwrap().clear();
        console.start(sh, &v(&["-c", "sleep 30"]), emit).unwrap();

        assert!(console.stop());

        wait_exit(&events);

        assert!(matches!(
            events.lock().unwrap().last(),
            Some(ConsoleEvent::Exit {
                stopped: true,
                code: None,
                ..
            })
        ));
        assert!(!console.stop());
    }

    #[cfg(unix)]
    fn wait_exit(events: &Arc<Mutex<Vec<ConsoleEvent>>>) {
        for _ in 0..200 {
            if events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, ConsoleEvent::Exit { .. }))
            {
                return;
            }

            std::thread::sleep(Duration::from_millis(50));
        }

        panic!("the run never ended");
    }
}
