use zed_extension_api::{self as zed, LanguageServerId, Result, Worktree, serde_json::Value, settings::LspSettings};

const CLI: &str = "node_modules/orisu/dist/cli.js";

struct Orisu;

impl Orisu {
    fn settings(id: &LanguageServerId, worktree: &Worktree) -> Option<LspSettings> {
        LspSettings::for_worktree(id.as_ref(), worktree).ok()
    }
}

impl zed::Extension for Orisu {
    fn new() -> Self {
        Self
    }

    /// Prefers a configured binary, then `orisu-lsp` on `PATH`, then the project's `orisu` package run by Zed's Node.
    fn language_server_command(&mut self, id: &LanguageServerId, worktree: &Worktree) -> Result<zed::Command> {
        let env = worktree.shell_env();
        if let Some(binary) = Self::settings(id, worktree).and_then(|settings| settings.binary)
            && let Some(path) = binary.path
        {
            return Ok(zed::Command { command: path, args: binary.arguments.unwrap_or_default(), env });
        }
        if let Some(path) = worktree.which("orisu-lsp") {
            return Ok(zed::Command { command: path, args: Vec::new(), env });
        }
        if worktree.read_text_file("node_modules/orisu/package.json").is_ok() {
            let cli = format!("{}/{CLI}", worktree.root_path());
            return Ok(zed::Command { command: zed::node_binary_path()?, args: vec![cli, "lsp".into()], env });
        }
        Err("orisu: install the `orisu` package or put `orisu-lsp` on PATH".into())
    }

    fn language_server_initialization_options(&mut self, id: &LanguageServerId, worktree: &Worktree) -> Result<Option<Value>> {
        Ok(Self::settings(id, worktree).and_then(|settings| settings.settings))
    }

    fn language_server_workspace_configuration(&mut self, id: &LanguageServerId, worktree: &Worktree) -> Result<Option<Value>> {
        Ok(Self::settings(id, worktree).and_then(|settings| settings.settings))
    }
}

zed::register_extension!(Orisu);
