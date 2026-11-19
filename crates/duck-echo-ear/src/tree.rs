//! Starting the Ear, and stopping it with everything it started.
//!
//! The Ear will start programs of its own (`llama-server`, in step 4).
//! Stopping only the Ear could leave those running, holding the graphics
//! card's memory. So the Ear is started as the head of a group, and the
//! whole group is stopped together:
//!
//! * On Linux, the Ear gets its own **process group**, and `kill -KILL --
//!   -<group>` stops every program in it, even ones whose parent has
//!   already died.
//! * On Windows, the Ear is put in a **job object** that kills everything
//!   in it when it is closed, so even if Duck Echo itself crashes, the
//!   Ear's programs go with it.
//!
//! As a third guard, the protocol says an Ear exits as soon as its stdin
//! closes, which happens whenever Duck Echo stops, however it stops.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The program to start, and what to give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The program itself (for a Python file, the Python that runs it).
    pub command: PathBuf,
    /// Its arguments (for a Python file, the file first).
    pub args: Vec<OsString>,
    /// The file Duck Echo was pointed at, to name in messages.
    pub file: PathBuf,
}

/// The environment variable naming the Python that runs an Ear given as a
/// `.py` file. Without it: `python3` on Linux, `python` on Windows.
pub const PYTHON_VARIABLE: &str = "DUCK_ECHO_EAR_PYTHON";

impl Program {
    /// The Ear at `file`. A `.py` file is run by Python; anything else is
    /// started as it is.
    pub fn new(file: impl Into<PathBuf>) -> Program {
        let file = file.into();
        let is_python = file
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("py"));
        if is_python {
            let python = std::env::var_os(PYTHON_VARIABLE).unwrap_or_else(|| {
                if cfg!(windows) {
                    "python".into()
                } else {
                    "python3".into()
                }
            });
            Program {
                command: python.into(),
                args: vec![file.clone().into_os_string()],
                file,
            }
        } else {
            Program {
                command: file.clone(),
                args: Vec::new(),
                file,
            }
        }
    }

    /// The same program with `arg` added at the end.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Program {
        self.args.push(arg.into());
        self
    }
}

/// A running Ear and the group it heads.
pub struct Tree {
    child: Child,
    #[cfg(windows)]
    job: Option<crate::windows_job::Job>,
    /// Set once the Ear has exited and its exit status was read.
    status: Option<ExitStatus>,
    /// Set once the whole group was stopped.
    stopped: bool,
}

/// The three pipes to and from a running Ear.
pub struct Pipes {
    /// Requests go in here.
    pub stdin: ChildStdin,
    /// Answers come out here.
    pub stdout: ChildStdout,
    /// Its logging comes out here.
    pub stderr: ChildStderr,
}

/// Start `program` at the head of its own group, with all three pipes.
pub fn start(program: &Program) -> io::Result<(Tree, Pipes)> {
    let mut command = Command::new(&program.command);
    command
        .args(&program.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0); // a new group, with the Ear at its head
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no black window
    }
    let mut child = command.spawn()?;
    let pipes = Pipes {
        stdin: child.stdin.take().expect("stdin was asked for"),
        stdout: child.stdout.take().expect("stdout was asked for"),
        stderr: child.stderr.take().expect("stderr was asked for"),
    };
    #[cfg(windows)]
    let job = {
        // If Windows refuses a job object (very old Windows, or a job that
        // forbids it), the Ear still runs; stopping it then stops only the
        // Ear itself, and the stdin rule stops the rest.
        crate::windows_job::Job::new()
            .and_then(|job| job.assign(&child).map(|()| job))
            .ok()
    };
    let tree = Tree {
        child,
        #[cfg(windows)]
        job,
        status: None,
        stopped: false,
    };
    Ok((tree, pipes))
}

impl Tree {
    /// The Ear's process number.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Has the Ear exited? `Some(code)` if it has (`code` is `None` when a
    /// signal stopped it), `None` while it runs.
    pub fn exited(&mut self) -> Option<Option<i32>> {
        if self.status.is_none() {
            self.status = self.child.try_wait().ok().flatten();
        }
        self.status.map(|status| status.code())
    }

    /// Wait until the Ear exits, but not past `deadline`.
    pub fn wait_until(&mut self, deadline: Instant) -> Option<Option<i32>> {
        loop {
            if let Some(code) = self.exited() {
                return Some(code);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Stop the Ear and every program in its group, at once. Safe to call
    /// more than once.
    pub fn stop_all(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        #[cfg(unix)]
        {
            // The whole group, by its number (the Ear's own). Asked of the
            // system's `kill` program, so Duck Echo needs no `unsafe` code
            // here. It may say the group is already gone: that is fine.
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.stop_all();
        }
        let _ = self.child.kill();
        if let Ok(status) = self.child.wait() {
            self.status.get_or_insert(status);
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.stop_all();
    }
}
