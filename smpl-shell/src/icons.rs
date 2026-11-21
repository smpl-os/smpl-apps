use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Root-priority XDG icon resolver shared by launch surfaces.
pub struct IconResolver {
    search_dirs: Vec<PathBuf>,
}

impl IconResolver {
    pub fn from_env() -> Self {
        Self::from_paths(
            std::env::var_os("HOME").as_deref().map(Path::new),
            std::env::var_os("XDG_DATA_HOME").as_deref().map(Path::new),
            std::env::var_os("XDG_DATA_DIRS").as_deref(),
            Path::new("/var/lib/flatpak/exports/share"),
        )
    }

    pub fn from_paths(
        home: Option<&Path>,
        data_home: Option<&Path>,
        data_dirs: Option<&OsStr>,
        system_flatpak: &Path,
    ) -> Self {
        let home = home.filter(|path| path.is_absolute());
        let data_home = data_home
            .filter(|path| path.is_absolute())
            .map(Path::to_path_buf)
            .or_else(|| home.map(|path| path.join(".local/share")));
        let data_dirs = data_dirs
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsStr::new("/usr/local/share:/usr/share"));
        let mut resolver = Self {
            search_dirs: Vec::new(),
        };
        if let Some(data) = &data_home {
            resolver.add_icon_root(&data.join("icons"));
        }
        if let Some(home) = home {
            resolver.add_icon_root(&home.join(".icons"));
        }
        if let Some(data) = &data_home {
            resolver.add_icon_root(&data.join("flatpak/exports/share/icons"));
            resolver.add_dir(data.join("pixmaps"));
        }
        for data in std::env::split_paths(data_dirs).filter(|path| path.is_absolute()) {
            resolver.add_icon_root(&data.join("icons"));
            resolver.add_dir(data.join("pixmaps"));
        }
        resolver.add_icon_root(&system_flatpak.join("icons"));
        resolver.add_dir(system_flatpak.join("pixmaps"));
        resolver
    }

    fn add_dir(&mut self, path: PathBuf) {
        if path.is_dir() && !self.search_dirs.contains(&path) {
            self.search_dirs.push(path);
        }
    }

    fn add_icon_root(&mut self, root: &Path) {
        if self.search_dirs.iter().any(|dir| dir == root) {
            return;
        }
        let mut themes = subdirs(root);
        themes.sort_by_key(|path| {
            (
                path.file_name() != Some(OsStr::new("hicolor")),
                path.clone(),
            )
        });
        for theme in themes {
            let mut sizes = subdirs(&theme);
            sizes.sort_by_key(|path| (size_rank(path), path.clone()));
            for size in sizes {
                let mut contexts = subdirs(&size);
                contexts.sort_by_key(|path| {
                    (path.file_name() != Some(OsStr::new("apps")), path.clone())
                });
                for context in contexts {
                    self.add_dir(context);
                }
                self.add_dir(size);
            }
            self.add_dir(theme);
        }
        self.add_dir(root.to_path_buf());
    }

    pub fn resolve(&self, icon_name: &str) -> Option<PathBuf> {
        let name = Path::new(icon_name);
        if name.is_absolute() {
            return name.is_file().then(|| name.to_path_buf());
        }
        if icon_name.is_empty() || icon_name.contains('/') || matches!(icon_name, "." | "..") {
            return None;
        }
        let explicit_extension = matches!(
            name.extension().and_then(OsStr::to_str),
            Some("svg" | "png")
        );
        for directory in &self.search_dirs {
            if explicit_extension {
                let path = directory.join(name);
                if path.is_file() {
                    return Some(path);
                }
            } else {
                for extension in ["svg", "png"] {
                    let path = directory.join(format!("{icon_name}.{extension}"));
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
        None
    }
}

fn size_rank(path: &Path) -> usize {
    const SIZES: &[&str] = &[
        "scalable", "48x48", "64x64", "128x128", "32x32", "256x256", "512x512", "96x96", "24x24",
        "22x22", "16x16", "symbolic",
    ];
    SIZES
        .iter()
        .position(|size| path.file_name() == Some(OsStr::new(size)))
        .unwrap_or(SIZES.len())
}

fn subdirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.is_dir().then_some(path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#;

    #[test]
    fn user_root_beats_system_theme_and_format_preferences() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let system = root.join("system/icons/hicolor/scalable/apps/example.svg");
        let user = root.join("home/.local/share/icons/hicolor/256x256/apps/example.png");
        for path in [&system, &user] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, SVG).unwrap();
        }
        let dirs = std::env::join_paths([root.join("system")]).unwrap();
        let resolver = IconResolver::from_paths(
            Some(&root.join("home")),
            None,
            Some(&dirs),
            &root.join("flatpak"),
        );
        assert_eq!(resolver.resolve("example"), Some(user));
    }

    #[test]
    fn invalid_relative_icon_names_do_not_escape_the_cache() {
        let fixture = tempfile::tempdir().unwrap();
        let resolver = IconResolver::from_paths(
            Some(fixture.path()),
            None,
            None,
            &fixture.path().join("flatpak"),
        );
        for name in ["", ".", "..", "dir/icon", "../icon"] {
            assert!(resolver.resolve(name).is_none(), "{name}");
        }
    }
}
