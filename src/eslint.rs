use std::{
    env, fs,
    path::{Path, PathBuf},
};
use zed_extension_api::{
    self as zed,
    serde_json::{from_str, Value},
    settings::LspSettings,
    LanguageServerId, LanguageServerInstallationStatus as Status, Result,
};

const SERVER_BIN_PATH: &str = "node_modules/eslint/bin/eslint.js";
const PACKAGE_NAME: &str = "eslint";

const ESLINT_CONFIG_PATHS: &[&str] = &[
    "eslint.json",
    ".eslintrc.json",
    ".eslintrc.js",
    ".eslintrc.yml",
    ".eslintrc.yaml",
    ".eslintrc",
    ".eslintrc.cjs",
    ".eslintrc.mjs",
];

struct EslintExtension;

impl EslintExtension {
    fn file_exists(&self, path: &PathBuf) -> bool {
        fs::metadata(path).map_or(false, |stat| stat.is_file())
    }

    fn get_local_lsp_path(&mut self, worktree: &zed::Worktree) -> Option<String> {
        let package_json = worktree
            .read_text_file("package.json")
            .ok()
            .and_then(|content| from_str::<Value>(content.as_str()).ok())?;

        let server_exists = package_json["dependencies"][PACKAGE_NAME].is_string()
            || package_json["devDependencies"][PACKAGE_NAME].is_string();

        let path = server_exists.then(|| {
            Path::new(worktree.root_path().as_str())
                .join(SERVER_BIN_PATH)
                .to_string_lossy()
                .to_string()
        })?;

        self.file_exists(&PathBuf::from(path.as_str()))
            .then_some(path)
    }

    fn install_lsp_server(&mut self, language_server_id: &LanguageServerId) -> Result<()> {
        zed::set_language_server_installation_status(
            language_server_id,
            &Status::CheckingForUpdate,
        );

        let version = zed::npm_package_latest_version(PACKAGE_NAME)?; // no eslint on remote is oy vey

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );
        let result = zed::npm_install_package(PACKAGE_NAME, &version);

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::None,
        );
        return result;
    }

    fn server_script_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        // check if the server is installed in the current worktree
        if let Some(local_path) = self.get_local_lsp_path(worktree) {
            return Ok(local_path);
        }

        let global_server_path = &Path::new("./").join(SERVER_BIN_PATH);

        // check if the server is installed globally, if not install it
        match self.file_exists(global_server_path) {
            true => return Ok(global_server_path.to_string_lossy().to_string()),
            false => self.install_lsp_server(&language_server_id)?,
        }

        // check again, if it's not installed, then error
        return match self.file_exists(global_server_path) {
            true => Ok(global_server_path.to_string_lossy().to_string()),
            false => Err(format!(
                "Failed to install eslint server for language server '{global_server_path:?}'",
            )),
        };
    }

    // Returns the path if a config file exists
    pub fn config_path(&self, worktree: &zed::Worktree, settings: &Value) -> Option<String> {
        let config_path_setting = settings.get("config_path").and_then(|value| value.as_str());

        if let Some(config_path) = config_path_setting {
            return worktree
                .read_text_file(config_path)
                .is_ok()
                .then_some(config_path.to_string());
        }

        return ESLINT_CONFIG_PATHS.into_iter().find_map(|config_path| {
            // TODO: walk up the directory tree to find the config file
            worktree
                .read_text_file(config_path)
                .is_ok()
                .then_some(config_path.to_string())
        });
    }
}

impl zed_extension_api::Extension for EslintExtension {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed_extension_api::LanguageServerId,
        worktree: &zed_extension_api::Worktree,
    ) -> zed_extension_api::Result<zed_extension_api::Command> {
        let lsp_path = self.server_script_path(language_server_id, worktree)?;
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        let mut args = vec![];

        if let Some(settings) = settings.settings {
            let config_path = self.config_path(worktree, &settings);

            let require_config_file = settings
                .get("require_config_file")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);

            if let Some(config_path) = config_path {
                args.push("--config".to_string());
                args.push(config_path.clone());
            } else if require_config_file {
                return Err("eslint.json is not found but require_config_file is true".to_string());
            }
        }

        let bin = env::current_dir()
            .unwrap()
            .join(lsp_path)
            .to_string_lossy()
            .to_string();

        if let Some(binary) = settings.binary {
            return Ok(zed::Command {
                command: binary.path.map_or(bin, |path| path),
                args: binary.arguments.map_or(args, |args| args),
                env: Default::default(),
            });
        }

        Ok(zed::Command {
            command: bin,
            args,
            env: Default::default(),
        })
    }
}

zed_extension_api::register_extension!(EslintExtension);
