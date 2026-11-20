//! Running tools and reporting, the way `macos.sh` did: GitHub Actions
//! turns `::warning::`/`::error::` lines into annotations.

use std::ffi::OsStr;
use std::process::Command;

use crate::Result;

pub fn note(text: &str) {
    println!("\n==> {text}");
}

pub fn warn(text: &str) {
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::warning::{text}");
    } else {
        eprintln!("warning: {text}");
    }
}

pub fn error(text: &str) {
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        println!("::error::{text}");
    } else {
        eprintln!("error: {text}");
    }
}

/// Runs `program args…`, its output going to ours; fails unless it exits 0.
pub fn run<I, S>(program: &str, args: I) -> Result
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(args);
    status(command)
}

pub fn status(mut command: Command) -> Result {
    let shown = show(&command);
    let status = command.status().map_err(|e| format!("can't run {shown}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{shown} failed ({status})"))
    }
}

/// Runs `program args…` and returns its stdout and stderr together (as
/// `codesign -dv` reports on stderr); fails unless it exits 0.
pub fn output<I, S>(program: &str, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(args);
    let shown = show(&command);
    let out = command.output().map_err(|e| format!("can't run {shown}: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("{shown} failed ({}): {}", out.status, text.trim()))
    }
}

fn show(command: &Command) -> String {
    let mut text = command.get_program().to_string_lossy().into_owned();
    for arg in command.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}
