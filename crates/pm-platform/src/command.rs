//! Running developer tools (`xcrun`, `adb`) and starting apps (M21's
//! launchers) behind a trait, so the code that builds their command lines
//! and parses their output is tested without them.

use std::ffi::OsString;
use std::process::Command;

use crate::PlatformError;

/// What a finished command printed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// `stdout` if the command succeeded, else its error output as an error.
    pub fn ok(self) -> Result<String, PlatformError> {
        if self.success {
            Ok(self.stdout)
        } else {
            let message = match self.stderr.trim() {
                "" => self.stdout.trim().to_owned(),
                err => err.to_owned(),
            };
            Err(PlatformError::Os(if message.is_empty() {
                "the command failed".into()
            } else {
                message
            }))
        }
    }
}

/// Extra environment variables for a spawned process.
pub type Env = [(OsString, OsString)];

pub trait Runner: Send + Sync {
    /// Runs `program` with `args` to completion. `Err` if it couldn't start.
    fn run(&self, program: &OsString, args: &[OsString]) -> std::io::Result<Output>;

    /// Starts `program` with `args` and `env` added to ours, without waiting
    /// for it, and returns its pid. It gets its own process group and no
    /// stdio, and outlives us.
    fn spawn(&self, _program: &OsString, _args: &[OsString], _env: &Env) -> std::io::Result<u32> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Shared runners, e.g. a test keeping a handle on the fake it handed out.
impl<R: Runner + ?Sized> Runner for std::sync::Arc<R> {
    fn run(&self, program: &OsString, args: &[OsString]) -> std::io::Result<Output> {
        (**self).run(program, args)
    }

    fn spawn(&self, program: &OsString, args: &[OsString], env: &Env) -> std::io::Result<u32> {
        (**self).spawn(program, args, env)
    }
}

/// Runs real processes.
pub struct System;

impl Runner for System {
    fn run(&self, program: &OsString, args: &[OsString]) -> std::io::Result<Output> {
        let out = Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()?;
        Ok(Output {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn spawn(&self, program: &OsString, args: &[OsString], env: &Env) -> std::io::Result<u32> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Its own process group, so a Ctrl-C to the app's terminal doesn't
        // reach the browser it started.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = command.spawn()?;
        let pid = child.id();
        // Reaped when it exits, so it never lingers as a zombie.
        std::thread::Builder::new()
            .name("pm-launched".into())
            .spawn(move || child.wait())?;
        Ok(pid)
    }
}

/// Builds an argument list from anything string-like.
pub fn args<I, S>(items: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    items.into_iter().map(Into::into).collect()
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// Records every command line and answers from a queue of outputs.
    /// Spawned processes get pids from 4242 up; their environment is kept
    /// in `envs`, one `NAME=value` list per spawn.
    #[derive(Default)]
    pub struct Fake {
        pub calls: Mutex<Vec<Vec<String>>>,
        pub replies: Mutex<VecDeque<std::io::Result<Output>>>,
        pub envs: Mutex<Vec<Vec<String>>>,
    }

    impl Fake {
        pub fn reply(&self, stdout: &str) -> &Self {
            self.replies.lock().unwrap().push_back(Ok(Output {
                success: true,
                stdout: stdout.into(),
                stderr: String::new(),
            }));
            self
        }

        pub fn fail(&self, stderr: &str) -> &Self {
            self.replies.lock().unwrap().push_back(Ok(Output {
                success: false,
                stdout: String::new(),
                stderr: stderr.into(),
            }));
            self
        }

        pub fn missing(&self) -> &Self {
            self.replies
                .lock()
                .unwrap()
                .push_back(Err(std::io::ErrorKind::NotFound.into()));
            self
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().iter().map(|c| c.join(" ")).collect()
        }
    }

    impl Runner for Fake {
        fn run(&self, program: &OsString, args: &[OsString]) -> std::io::Result<Output> {
            let mut line = vec![program.to_string_lossy().into_owned()];
            line.extend(args.iter().map(|a| a.to_string_lossy().into_owned()));
            self.calls.lock().unwrap().push(line);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(Output::default()))
        }

        fn spawn(&self, program: &OsString, args: &[OsString], env: &Env) -> std::io::Result<u32> {
            let mut line = vec![program.to_string_lossy().into_owned()];
            line.extend(args.iter().map(|a| a.to_string_lossy().into_owned()));
            let mut calls = self.calls.lock().unwrap();
            calls.push(line);
            self.envs.lock().unwrap().push(
                env.iter()
                    .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
                    .collect(),
            );
            Ok(4242 + calls.len() as u32 - 1)
        }
    }
}
