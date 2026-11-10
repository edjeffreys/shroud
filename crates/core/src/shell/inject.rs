//! Nested shells skip the outer terminal's integration, so we reuse its scripts rather than ship ours.

/// Environment for a pane's shell so it loads the integration in `resources`
/// (the outer terminal's resources directory). Empty when the shell isn't supported.
pub fn integration_env(
    shell: &str,
    resources: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, String)> {
    let dir = format!("{resources}/shell-integration");
    let name = shell.rsplit('/').next().unwrap_or(shell).trim_start_matches('-');
    match name {
        "zsh" => {
            let mut out = vec![("ZDOTDIR", format!("{dir}/zsh"))];
            // The integration's .zshenv restores this, so the user's own dotfiles still load.
            if let Some(orig) = env("ZDOTDIR") {
                out.push(("GHOSTTY_ZSH_ZDOTDIR", orig));
            }
            out
        }
        "fish" => {
            let data_dirs = match env("XDG_DATA_DIRS") {
                Some(existing) if !existing.is_empty() => format!("{dir}:{existing}"),
                _ => dir.clone(),
            };
            vec![("XDG_DATA_DIRS", data_dirs), ("GHOSTTY_SHELL_INTEGRATION_XDG_DIR", dir)]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = "/App/Resources/x";

    #[test]
    fn zsh_keeps_the_users_zdotdir() {
        let env = integration_env("/bin/zsh", RES, |k| (k == "ZDOTDIR").then(|| "/Users/ed/.config/zsh".into()));
        assert_eq!(
            env,
            vec![
                ("ZDOTDIR", format!("{RES}/shell-integration/zsh")),
                ("GHOSTTY_ZSH_ZDOTDIR", "/Users/ed/.config/zsh".into())
            ]
        );
        assert_eq!(integration_env("zsh", RES, |_| None).len(), 1);
    }

    #[test]
    fn fish_prepends_to_data_dirs() {
        let env = integration_env("/opt/homebrew/bin/fish", RES, |_| Some("/usr/share".into()));
        assert_eq!(env[0], ("XDG_DATA_DIRS", format!("{RES}/shell-integration:/usr/share")));
    }

    #[test]
    fn unsupported_shells_are_left_alone() {
        assert!(integration_env("/bin/bash", RES, |_| None).is_empty());
        assert!(integration_env("nu", RES, |_| None).is_empty());
    }
}
