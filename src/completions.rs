/// Custom (user-defined) completions.
///
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use yaml_rust::yaml::{Yaml, YamlLoader};

/// Retrieves a list of suggestions based on the provided input and YAML configuration.
///
/// This function analyzes the user's input and suggests possible commands, subcommands, or options
/// based on a hierarchical configuration defined in YAML. The hierarchy is commands → subcommands → options.
///
/// # Arguments
///
/// * `config` - A reference to a `Yaml` object representing the hierarchical command configuration.
/// * `input` - A string slice that holds the user's input to match against commands and subcommands.
///
/// # Returns
///
/// Returns a `Vec<String>` containing suggestions based on the input. Suggestions are formatted as
/// "command subcommand" or "command subcommand option" depending on the input's completeness.
/// # Example
///
/// ```
/// let config_str = r#"
/// commands:
///   - name: git
///     subcommands:
///       - name: commit
///         options:
///           - amend
///           - no-verify
///       - name: clone
///         options:
///           - depth
///           - branch
///   - name: docker
///     subcommands:
///       - name: run
///         options:
///           - detach
///           - rm
///       - name: build
///         options:
///           - tag
///           - no-cache
/// "#;
/// let config = YamlLoader::load_from_str(config_str).unwrap()[0].clone();
/// let suggestions = suggest(&config, "git c");
/// assert_eq!(suggestions, vec!["git commit", "git clone"]);
/// ```
pub fn suggest(config: &Yaml, input: &str) -> Vec<String> {
    const LEVELS: &[&str] = &["commands", "subcommands", "options"];

    let parts: Vec<&str> = input.split_whitespace().collect();

    let mut current = config;
    let mut prefix = Vec::new();
    let mut suggestions = Vec::new();

    fn elem_to_str(elem: &Yaml) -> &str {
        if let Some(elem_name) = elem["name"].as_str() {
            elem_name.trim()
        } else {
            elem.as_str().unwrap_or("")
        }
    }

    for i in 0..LEVELS.len() {
        if let Some(elems) = current[LEVELS[i]].as_vec() {
            match parts.get(i) {
                None => {
                    if !prefix.is_empty() {
                        let prefix = prefix.join(" ");
                        for elem in elems {
                            suggestions.push(format!("{} {}", prefix, elem_to_str(elem)));
                        }
                    }
                    break;
                }
                Some(mut part) => {
                    for j in i + 1.. {
                        for elem in elems {
                            let elem_name = elem_to_str(elem);
                            if *part == elem_name {
                                prefix.push(*part);
                                current = elem;
                                break;
                            }

                            if elem_name.starts_with(part) {
                                if prefix.is_empty() {
                                    suggestions.push(elem_name.to_string());
                                } else {
                                    suggestions.push(format!("{} {}", prefix.join(" "), elem_name));
                                };
                            }
                        }

                        // Match all remaining input parts against the last hierarchy level
                        if j < LEVELS.len() {
                            break; // Not last level
                        }
                        if let Some(next) = parts.get(j) {
                            part = next;
                        } else {
                            break; // No more input parts
                        }
                    }
                }
            }
        }
    }
    suggestions
}

/// Loads the YAML configuration from the specified file.
///
/// # Arguments
///
/// * `file_path` - A reference to the Path to the YAML file.
///
/// # Returns
///
/// Returns a `Result` with the loaded `Yaml` document if successful,
/// or an `io::Error` if reading the file fails.
pub fn load_config_from_file(file_path: &Path) -> Result<Yaml, io::Error> {
    // Read the file contents into a string
    let mut file_content = String::new();
    let mut file = fs::File::open(file_path)?;
    file.read_to_string(&mut file_content)?;

    // Parse the YAML content
    let yaml_docs = YamlLoader::load_from_str(&file_content)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Failed to parse YAML"))?;

    // Return the first document, as we are only dealing with one YAML document
    Ok(yaml_docs[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yaml_rust::YamlLoader;

    #[test]
    fn test_suggest_empty_input() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: commit
              - name: clone
          - name: docker
            subcommands:
              - name: run
              - name: build
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test that no suggestions are returned for empty input
        let suggestions = suggest(config, "");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_suggest_top_level_command() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: commit
              - name: clone
          - name: docker
            subcommands:
              - name: run
              - name: build
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test that suggestions are returned for top-level commands
        let suggestions = suggest(config, "git");
        assert_eq!(suggestions, vec!["git commit", "git clone"]);
    }

    #[test]
    fn test_suggest_subcommands_exact_match() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: commit
                options:
                  - amend
                  - no-verify
              - name: clone
                options:
                  - depth
                  - branch
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test suggestions for exact subcommand match
        let suggestions = suggest(config, "git commit");
        assert_eq!(
            suggestions,
            vec!["git commit amend", "git commit no-verify"]
        );

        let suggestions = suggest(config, "git clone");
        assert_eq!(suggestions, vec!["git clone depth", "git clone branch"]);
    }

    #[test]
    fn test_suggest_subcommands_partial_match() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: commit
                options:
                  - amend
                  - no-verify
              - name: clone
                options:
                  - depth
                  - branch
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test partial match for subcommands
        let suggestions = suggest(config, "git c");
        assert_eq!(suggestions, vec!["git commit", "git clone"]);

        // Test partial match for subcommands with no exact match
        let suggestions = suggest(config, "git co");
        assert_eq!(suggestions, vec!["git commit"]);
    }

    #[test]
    fn test_suggest_subcommands_partial_option() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: commit
                options:
                  - amend
                  - no-verify
              - name: clone
                options:
                  - depth
                  - branch
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test partial match for subcommands with no exact match
        let suggestions = suggest(config, "git commit a");
        assert_eq!(suggestions, vec!["git commit amend"]);
    }

    #[test]
    fn test_unknown_command() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: commit
              - name: clone
          - name: docker
            subcommands:
              - name: run
              - name: build
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test unknown command, should return an empty vector
        let suggestions = suggest(config, "unknown");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_exhaust_options() {
        let config_str = r#"
        commands:
          - name: git
            subcommands:
              - name: clone
                options:
                  - --verbose
                  - --no-hard-links
              - name: commit
          - name: docker
            subcommands:
              - name: run
              - name: build
        "#;
        let config = &YamlLoader::load_from_str(config_str).unwrap()[0];

        // Test unknown command, should return an empty vector
        let suggestions = suggest(config, "git clone --verbose --n");
        assert_eq!(suggestions, vec!["git clone --verbose --no-hard-links"]);
    }
}
