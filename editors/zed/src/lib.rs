//! Zed extension for Cranium.
//!
//! Everything an editor needs is already provided by `wernicke`, so this is
//! only responsible for starting it: finding the binary and handing Zed the
//! command to run.

use zed_extension_api::{
    self as zed, Command, LanguageServerId, Result, Worktree, settings::LspSettings,
};

/// The name of the language server binary this extension launches.
const SERVER: &str = "wernicke";

struct CraniumExtension;

impl zed::Extension for CraniumExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        // A path configured in settings wins, so a project can pin a build.
        let configured = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);

        if let Some(binary) = configured
            && let Some(path) = binary.path
        {
            return Ok(Command {
                command: path,
                args: binary.arguments.unwrap_or_else(default_args),
                env: worktree.shell_env(),
            });
        }

        let command = worktree.which(SERVER).ok_or_else(|| {
            format!(
                "`{SERVER}` is not on your PATH. Install it with \
                 `cargo install --git https://github.com/Aspenini/bf-tools {SERVER}`, \
                 or set `lsp.{SERVER}.binary.path` in your Zed settings."
            )
        })?;

        Ok(Command {
            command,
            args: default_args(),
            env: worktree.shell_env(),
        })
    }
}

fn default_args() -> Vec<String> {
    vec!["--stdio".to_string()]
}

zed::register_extension!(CraniumExtension);
