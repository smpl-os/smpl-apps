use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// One row from smplOS' `~/.cache/smplos/app_index`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub exec: String,
    pub category: String,
    pub icon: String,
    /// Search-only Settings cards do not appear in category browsing.
    pub search_only: bool,
}

pub fn path(home: Option<&Path>) -> Option<PathBuf> {
    home.filter(|path| path.is_absolute())
        .map(|path| path.join(".cache/smplos/app_index"))
}

pub fn from_env() -> Option<PathBuf> {
    path(std::env::var_os("HOME").as_deref().map(Path::new))
}

pub fn read(path: &Path) -> io::Result<Vec<AppEntry>> {
    std::fs::read_to_string(path).map(|source| parse(&source))
}

/// Parse the stable `Name;Exec;Category;Icon[;1]` wire format.
///
/// Malformed rows are ignored, matching the existing launcher contract. A
/// duplicate name keeps the first row.
pub fn parse(source: &str) -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let mut names = HashSet::new();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.splitn(5, ';').collect();
        if fields.len() < 3 {
            continue;
        }
        let name = fields[0].to_owned();
        if !names.insert(name.to_lowercase()) {
            continue;
        }
        apps.push(AppEntry {
            name,
            exec: fields[1].to_owned(),
            category: fields[2].to_owned(),
            icon: fields.get(3).copied().unwrap_or("").to_owned(),
            search_only: fields.get(4).is_some_and(|value| value.trim() == "1"),
        });
    }
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_and_search_only_rows_without_rewriting() {
        let apps = parse(
            "Firefox;firefox;internet;firefox\n\
             Settings;smplos-settings;settings;settings\n\
             Power;smplos-settings power;settings;;1\n\
             malformed\n\
             firefox;other;apps;other\n",
        );
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].icon, "firefox");
        assert!(apps[2].search_only);
        assert_eq!(apps[0].exec, "firefox", "the first spelling wins");
    }

    #[test]
    fn cache_path_requires_an_absolute_home() {
        assert!(path(Some(Path::new("relative"))).is_none());
        assert_eq!(
            path(Some(Path::new("/home/example"))),
            Some(PathBuf::from("/home/example/.cache/smplos/app_index"))
        );
    }
}
