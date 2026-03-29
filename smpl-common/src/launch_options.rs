//! Launch options — per-app environment variables and prefix commands.
//!
//! Config files live at `~/.config/smplos/launch-options/<app-id>.conf`
//! using a simple INI-like format:
//!
//! ```ini
//! [environment]
//! BLENDER_DISABLE_WAYLAND=1
//! MESA_LOADER_DRIVER_OVERRIDE=zink
//!
//! [launch]
//! prefix = gamemoderun
//! ```
//!
//! The `app-id` is derived from the exec string — typically the desktop
//! file stem (e.g. `org.blender.Blender`) or the binary name.

use std::path::{Path, PathBuf};

/// Per-app launch options.
#[derive(Clone, Debug, Default)]
pub struct LaunchOptions {
    /// Environment variables, each as `KEY=VALUE`.
    pub env_vars: Vec<String>,
    /// Command prefix (e.g. `gamemoderun`, `mangohud`).
    pub prefix: String,
}

impl LaunchOptions {
    pub fn is_empty(&self) -> bool {
        self.env_vars.is_empty() && self.prefix.is_empty()
    }
}

/// Directory where launch option configs are stored.
fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!(
        "{}/{}",
        std::env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| format!("{}/.config", home)),
        "smplos/launch-options"
    ))
}

/// Derive an app-id from an exec string.
///
/// For Flatpak: extracts the reverse-domain ID (e.g. `org.blender.Blender`).
/// For others: uses the binary basename (e.g. `blender`).
pub fn app_id_from_exec(exec: &str) -> String {
    let exec = exec.trim();

    // Flatpak: `flatpak run [--flags] org.example.App [args]`
    if exec.contains("flatpak run") {
        let parts: Vec<&str> = exec.split_whitespace().collect();
        if let Some(pos) = parts.iter().position(|&p| p == "run") {
            // Skip flags that start with --
            for part in &parts[pos + 1..] {
                if !part.starts_with("--") {
                    return part.to_string();
                }
            }
        }
    }

    // launch-webapp: use the --name slug
    if exec.contains("launch-webapp") {
        if let Some(pos) = exec.find("--name") {
            let after = &exec[pos + 6..];
            let after = after.trim_start().trim_start_matches('"');
            let end = after.find(|c: char| c == '"' || c.is_whitespace()).unwrap_or(after.len());
            let slug = &after[..end];
            if !slug.is_empty() {
                return format!("webapp-{}", slug);
            }
        }
    }

    // Generic: first token, basename only
    let first = exec.split_whitespace().next().unwrap_or(exec);
    Path::new(first)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| first.to_string())
}

/// Config file path for a given app-id.
fn config_path(app_id: &str) -> PathBuf {
    config_dir().join(format!("{}.conf", app_id))
}

/// Load launch options for an app. Returns default (empty) if no config exists.
pub fn load(exec: &str) -> LaunchOptions {
    let app_id = app_id_from_exec(exec);
    load_by_id(&app_id)
}

/// Load launch options by app-id directly.
pub fn load_by_id(app_id: &str) -> LaunchOptions {
    let path = config_path(app_id);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return LaunchOptions::default(),
    };
    parse(&content)
}

/// Parse a config file's contents.
fn parse(content: &str) -> LaunchOptions {
    let mut opts = LaunchOptions::default();
    let mut section = String::new();

    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            continue;
        }
        match section.as_str() {
            "environment" => {
                if line.contains('=') {
                    opts.env_vars.push(line.to_string());
                }
            }
            "launch" => {
                if let Some((key, val)) = line.split_once('=') {
                    let key = key.trim();
                    let val = val.trim();
                    if key == "prefix" {
                        opts.prefix = val.to_string();
                    }
                }
            }
            _ => {}
        }
    }
    opts
}

/// Save launch options for an app. Creates/overwrites the config file.
/// If options are empty, removes the config file.
pub fn save(exec: &str, opts: &LaunchOptions) {
    let app_id = app_id_from_exec(exec);
    save_by_id(&app_id, opts);
}

/// Save launch options by app-id.
pub fn save_by_id(app_id: &str, opts: &LaunchOptions) {
    let path = config_path(app_id);

    if opts.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }

    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);

    let mut content = String::new();
    if !opts.env_vars.is_empty() {
        content.push_str("[environment]\n");
        for ev in &opts.env_vars {
            content.push_str(ev);
            content.push('\n');
        }
        content.push('\n');
    }
    if !opts.prefix.is_empty() {
        content.push_str("[launch]\n");
        content.push_str(&format!("prefix = {}\n", opts.prefix));
    }
    let _ = std::fs::write(&path, content);
}

/// Check if an app has custom launch options configured.
pub fn has_options(exec: &str) -> bool {
    let app_id = app_id_from_exec(exec);
    config_path(&app_id).exists()
}

/// Build the effective exec command with launch options applied.
///
/// If options exist, wraps the command with `launch-with-options <app-id>`.
/// For flatpak commands, injects `--env=` flags directly.
pub fn wrap_exec(exec: &str) -> String {
    let app_id = app_id_from_exec(exec);
    let opts = load_by_id(&app_id);

    if opts.is_empty() {
        return exec.to_string();
    }

    // For Flatpak apps, inject --env= flags directly into the flatpak run command
    if exec.contains("flatpak run") {
        let mut parts: Vec<&str> = exec.split_whitespace().collect();
        if let Some(run_pos) = parts.iter().position(|&p| p == "run") {
            let insert_at = run_pos + 1;
            let env_flags: Vec<String> = opts
                .env_vars
                .iter()
                .map(|ev| format!("--env={}", ev))
                .collect();
            for (i, flag) in env_flags.iter().enumerate() {
                parts.insert(insert_at + i, flag);
            }
            let base = parts.join(" ");
            if opts.prefix.is_empty() {
                return base;
            }
            return format!("{} {}", opts.prefix, base);
        }
    }

    // For everything else, use the launch-with-options wrapper
    format!("launch-with-options {} {}", app_id, exec)
}

/// Remove all launch options for an app.
pub fn reset(exec: &str) {
    let app_id = app_id_from_exec(exec);
    let path = config_path(&app_id);
    let _ = std::fs::remove_file(&path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_id_flatpak() {
        assert_eq!(
            app_id_from_exec("flatpak run org.blender.Blender"),
            "org.blender.Blender"
        );
        assert_eq!(
            app_id_from_exec("flatpak run --env=FOO=1 org.blender.Blender"),
            "org.blender.Blender"
        );
    }

    #[test]
    fn test_app_id_native() {
        assert_eq!(app_id_from_exec("/usr/bin/blender"), "blender");
        assert_eq!(app_id_from_exec("blender %F"), "blender");
    }

    #[test]
    fn test_app_id_webapp() {
        assert_eq!(
            app_id_from_exec("launch-webapp \"--name\" \"chat\" \"https://example.com\""),
            "webapp-chat"
        );
    }

    #[test]
    fn test_parse() {
        let content = "[environment]\nFOO=bar\nBAZ=1\n\n[launch]\nprefix = gamemoderun\n";
        let opts = parse(content);
        assert_eq!(opts.env_vars, vec!["FOO=bar", "BAZ=1"]);
        assert_eq!(opts.prefix, "gamemoderun");
    }

    #[test]
    fn test_parse_empty() {
        let opts = parse("");
        assert!(opts.is_empty());
    }

    #[test]
    fn test_wrap_exec_flatpak() {
        // This test won't find a config file, so it should return the exec unchanged
        let exec = "flatpak run org.blender.Blender";
        assert_eq!(wrap_exec(exec), exec);
    }
}
