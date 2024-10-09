use crate::cmds::{get_command, Exec};
use crate::scope::Scope;
use crate::utils;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use yaml_rust::yaml::{Yaml, YamlLoader};

///
/// Example configuration:
/// ```
/// hooks:
///   on_change_dir:
///   - action: "detect_git_branch.my"
/// ```
/// Example hook:
/// ```
/// if $__interactive (
///     # Suppress errors from git commands
///     __stderr = NULL;
//      # Set GIT_BRANCH variable if git repository detected.
///     if (git branch --show-current | b && eval -x "GIT_BRANCH = \\$b")
///         ()
///     # Otherwise clear variable if previously defined.
///     else (if (defined GIT_BRANCH) ($GIT_BRANCH=));
/// )
/// ```
pub struct Hooks {
    config: Yaml,
    path: PathBuf, // path to scripts
}

impl Hooks {
    pub fn new(config_path: &Path) -> Result<Self, io::Error> {
        // Hook scripts are expected in ~/.shmy/hooks
        let path = config_path.parent().expect("Invalid hooks path").to_owned();
        let config = Self::load_yaml(config_path)?;
        Ok(Self { config, path })
    }

    /// Loads the YAML configuration from the specified file.
    fn load_yaml(file_path: &Path) -> Result<Yaml, io::Error> {
        let mut file_content = String::new();
        let mut file = fs::File::open(file_path)?;
        file.read_to_string(&mut file_content)?;
        let yaml_docs = YamlLoader::load_from_str(&file_content)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Failed to parse YAML"))?;
        Ok(yaml_docs[0].clone())
    }

    /// Executes the hooks for a given event (e.g., `change_dir`).
    pub fn run(
        &self,
        scope: &Arc<Scope>,
        event: &str,
        event_args: &[String],
    ) -> Result<(), String> {
        // Do not run any hooks if elevated (or root).
        if utils::is_elevated() {
            return Ok(());
        }

        let hooks = self.config["hooks"][format!("on_{}", event).as_str()].as_vec();
        if let Some(hooks) = hooks {
            for hook in hooks {
                if let Some(action) = hook["action"].as_str() {
                    self.run_action(scope, action, event_args)?;
                }
            }
        }
        Ok(())
    }

    /// Executes the specified action.
    fn run_action(
        &self,
        scope: &Arc<Scope>,
        action: &str,
        event_args: &[String],
    ) -> Result<(), String> {
        let eval = get_command("eval").expect("eval command not registered?");
        let action_path = self.path.join(action);

        let mut args = Vec::new();
        args.push("-s".to_string());
        args.push(action_path.to_string_lossy().to_string());
        args.push("-q".to_string()); // suppress stdout output
        args.extend_from_slice(event_args);

        eval.exec("hook", &args, scope)?;
        Ok(())
    }
}
