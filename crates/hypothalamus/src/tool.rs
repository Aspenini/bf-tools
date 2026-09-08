use std::env;
use std::ffi::OsStr;
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

    find_on_path(name).or_else(|| executable_in(fallback_dir?, OsStr::new(name)))
}

pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|dir| executable_in(&dir, OsStr::new(name)))
}

pub(crate) fn find_sibling_tool(driver: &str, candidates: &[&str]) -> Option<PathBuf> {
    let driver_path = resolve_command_path(driver)?;
    let dir = driver_path.parent()?;

    candidates
        .iter()
        .find_map(|candidate| executable_in(dir, OsStr::new(candidate)))
}

pub(crate) fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return executable_in(path.parent()?, path.file_name()?);
    }

    find_on_path(command)
}

/// Look for the executable `name` in `dir`.
///
/// Windows leaves `.exe` off a command name but not off the file on disk, so a
/// tool invoked as `ld.lld` is `ld.lld.exe` there. `Command` appends the
/// extension itself when it runs something; discovery has to do it by hand, or
/// an installed toolchain looks missing.
fn executable_in(dir: &Path, name: &OsStr) -> Option<PathBuf> {
    let path = dir.join(name);
    if path.is_file() {
        return Some(path);
    }

    #[cfg(windows)]
    {
        let mut with_extension = name.to_os_string();
        with_extension.push(".exe");
        let path = dir.join(with_extension);
        if path.is_file() {
            return Some(path);
        }
    }

    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_an_executable_by_its_command_name() {
        let dir = env::temp_dir().join(format!("hypothalamus-tool-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create probe directory");
        let file_name = if cfg!(windows) { "probe.exe" } else { "probe" };
        fs::write(dir.join(file_name), b"").expect("write probe executable");

        let found = executable_in(&dir, OsStr::new("probe"));

        let _ = fs::remove_dir_all(&dir);
        assert_eq!(found, Some(dir.join(file_name)));
    }

    #[test]
    fn reports_a_missing_executable() {
        assert_eq!(
            executable_in(&env::temp_dir(), OsStr::new("hypothalamus-absent")),
            None
        );
    }
}
