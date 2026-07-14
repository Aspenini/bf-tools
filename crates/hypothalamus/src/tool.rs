use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const OUTPUT_TAIL_BYTES: usize = 4096;

#[derive(Debug)]
pub(crate) struct CapturedToolFailure {
    pub(crate) status: String,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn run_captured(mut command: Command) -> io::Result<Option<CapturedToolFailure>> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(None);
    }

    Ok(Some(CapturedToolFailure {
        status: output.status.to_string(),
        stdout: output_tail(&output.stdout),
        stderr: output_tail(&output.stderr),
    }))
}

pub(crate) fn find_tool(
    override_path: Option<&Path>,
    name: &str,
    fallback_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return Some(path.to_path_buf());
    }

    find_on_path(name).or_else(|| {
        let fallback_dir = fallback_dir?;
        let path = fallback_dir.join(name);
        path.is_file().then_some(path)
    })
}

pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

pub(crate) fn find_sibling_tool(driver: &str, candidates: &[&str]) -> Option<PathBuf> {
    let driver_path = resolve_command_path(driver)?;
    let dir = driver_path.parent()?;

    candidates.iter().find_map(|candidate| {
        let path = dir.join(candidate);
        if path.is_file() {
            return Some(path);
        }

        #[cfg(windows)]
        {
            let path = dir.join(format!("{candidate}.exe"));
            if path.is_file() {
                return Some(path);
            }
        }

        None
    })
}

pub(crate) fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 && path.is_file() {
        return Some(path.to_path_buf());
    }

    find_on_path(command)
}

fn output_tail(bytes: &[u8]) -> String {
    let tail_start = bytes.len().saturating_sub(OUTPUT_TAIL_BYTES);
    let mut output = String::new();

    if tail_start > 0 {
        output.push_str("[truncated]\n");
    }
    output.push_str(&String::from_utf8_lossy(&bytes[tail_start..]));
    output
}
