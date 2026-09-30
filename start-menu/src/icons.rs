use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Root priority is stronger than theme, size or format preference.
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

    pub(crate) fn from_paths(
        home: Option<&Path>,
        data_home: Option<&Path>,
        data_dirs: Option<&OsStr>,
        system_flatpak: &Path,
    ) -> Self {
        let home = home.filter(|p| p.is_absolute());
        let data_home = data_home
            .filter(|p| p.is_absolute())
            .map(Path::to_path_buf)
            .or_else(|| home.map(|p| p.join(".local/share")));
        let data_dirs = data_dirs
            .filter(|p| !p.is_empty())
            .unwrap_or(OsStr::new("/usr/local/share:/usr/share"));
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
        for data in std::env::split_paths(data_dirs).filter(|p| p.is_absolute()) {
            resolver.add_icon_root(&data.join("icons"));
            resolver.add_dir(data.join("pixmaps"));
        }
        // Flatpak's export directory need not be present in XDG_DATA_DIRS.
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
        // Keep hicolor first, then deterministic installed-theme fallbacks.
        // Both size/context (hicolor) and context/size (e.g. Breeze) layouts
        // fit this traversal, bounded even with directory symlinks.
        let mut themes = subdirs(root);
        themes.sort_by_key(|p| (p.file_name() != Some(OsStr::new("hicolor")), p.clone()));
        for theme in themes {
            let mut sizes = subdirs(&theme);
            sizes.sort_by_key(|p| (size_rank(p), p.clone()));
            for size in sizes {
                let mut contexts = subdirs(&size);
                contexts.sort_by_key(|p| {
                    (
                        p.file_name() != Some(OsStr::new("apps")),
                        size_rank(p),
                        p.clone(),
                    )
                });
                for context in contexts {
                    self.add_dir(context);
                }
                self.add_dir(size);
            }
            self.add_dir(theme);
        }
        // Unthemed local/AppImage icons can live directly in the icon root.
        self.add_dir(root.to_path_buf());
    }

    pub fn resolve(&self, icon_name: &str) -> Option<PathBuf> {
        let name = Path::new(icon_name);
        if name.is_absolute() {
            return name.is_file().then(|| name.to_path_buf());
        }
        // A relative desktop Icon is a name, not a path relative to our cwd.
        if icon_name.is_empty() || icon_name.contains('/') || matches!(icon_name, "." | "..") {
            return None;
        }
        let explicit_extension = matches!(
            name.extension().and_then(OsStr::to_str),
            Some("svg" | "png")
        );
        for dir in &self.search_dirs {
            if explicit_extension {
                let path = dir.join(name);
                if path.is_file() {
                    return Some(path);
                }
            } else {
                for extension in ["svg", "png"] {
                    let path = dir.join(format!("{icon_name}.{extension}"));
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
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            eprintln!(
                "start-menu: cannot scan icon directory {}: {error}",
                root.display()
            );
            return Vec::new();
        }
    };
    entries
        .filter_map(|entry| match entry {
            Ok(entry) => {
                let path = entry.path();
                path.is_dir().then_some(path)
            }
            Err(error) => {
                eprintln!(
                    "start-menu: cannot read icon directory {}: {error}",
                    root.display()
                );
                None
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#;
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\
        \x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x04\x00\x00\x00\xb5\x1c\x0c\x02\
        \x00\x00\x00\x0bIDAT\x78\xda\x63\x64\xf8\x0f\x00\x01\x05\x01\x01\x27\x18\xe3\x66\
        \x00\x00\x00\x00IEND\xae\x42\x60\x82";

    struct Fixture {
        root: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                root: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.path().join(relative)
        }

        fn icon(&self, relative: &str) -> PathBuf {
            let path = self.path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let contents = if path.extension() == Some(OsStr::new("png")) {
                PNG
            } else {
                SVG.as_bytes()
            };
            std::fs::write(&path, contents).unwrap();
            path
        }

        fn resolver(&self, data_home: Option<&Path>, data_dirs: &[&str]) -> IconResolver {
            let dirs = std::env::join_paths(data_dirs.iter().map(|dir| self.path(dir))).unwrap();
            IconResolver::from_paths(
                Some(&self.path("home")),
                data_home,
                Some(&dirs),
                &self.path("system-flatpak"),
            )
        }
    }

    #[test]
    fn user_root_wins_over_system_size_and_format_preferences() {
        let fixture = Fixture::new();
        fixture.icon("system/icons/hicolor/scalable/apps/example.svg");
        fixture.icon("system/icons/hicolor/48x48/apps/example.png");
        let expected = fixture.icon("home/.local/share/icons/hicolor/256x256/apps/example.png");
        let resolved = fixture
            .resolver(None, &["system"])
            .resolve("example")
            .unwrap();
        assert_eq!(resolved, expected);
        assert!(slint::Image::load_from_path(&resolved).is_ok());
    }

    #[test]
    fn legacy_user_root_wins_over_system_and_xdg_home_wins_over_legacy() {
        let fixture = Fixture::new();
        fixture.icon("system/icons/hicolor/scalable/apps/example.svg");
        let legacy = fixture.icon("home/.icons/hicolor/16x16/apps/example.png");
        assert_eq!(
            fixture.resolver(None, &["system"]).resolve("example"),
            Some(legacy)
        );
        let xdg = fixture.icon("data/icons/hicolor/512x512/apps/example.png");
        assert_eq!(
            fixture
                .resolver(Some(&fixture.path("data")), &["system"])
                .resolve("example"),
            Some(xdg)
        );
    }

    #[test]
    fn xdg_data_dirs_order_is_stronger_than_size_and_format() {
        let fixture = Fixture::new();
        let first = fixture.icon("first/icons/hicolor/256x256/apps/example.png");
        let second = fixture.icon("second/icons/hicolor/scalable/apps/example.svg");
        assert_eq!(
            fixture
                .resolver(None, &["first", "second"])
                .resolve("example"),
            Some(first)
        );
        assert_eq!(
            fixture
                .resolver(None, &["second", "first"])
                .resolve("example"),
            Some(second)
        );
    }

    #[test]
    fn explicit_data_home_replaces_default_and_relative_home_is_ignored() {
        let fixture = Fixture::new();
        let default = fixture.icon("home/.local/share/icons/example.svg");
        let system = fixture.icon("system/icons/example.svg");
        assert_eq!(
            fixture
                .resolver(Some(&fixture.path("empty")), &["system"])
                .resolve("example"),
            Some(system)
        );
        assert_eq!(
            fixture
                .resolver(Some(Path::new("relative")), &["system"])
                .resolve("example"),
            Some(default)
        );
    }

    #[test]
    fn absolute_files_and_symlinks_are_honored_but_missing_paths_are_not() {
        let fixture = Fixture::new();
        fixture.icon("system/icons/example.svg");
        let absolute = fixture.icon("outside/example.svg");
        let symlink = fixture.path("outside/link.svg");
        std::os::unix::fs::symlink(&absolute, &symlink).unwrap();
        let resolver = fixture.resolver(None, &["system"]);
        assert_eq!(resolver.resolve(absolute.to_str().unwrap()), Some(absolute));
        assert_eq!(resolver.resolve(symlink.to_str().unwrap()), Some(symlink));
        assert_eq!(
            resolver.resolve(fixture.path("missing/example.svg").to_str().unwrap()),
            None
        );
        assert_eq!(
            resolver.resolve(fixture.path("outside").to_str().unwrap()),
            None
        );
    }

    #[test]
    fn missing_or_non_file_candidates_fall_through() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.path("home/.icons/hicolor/scalable/apps/example.svg"))
            .unwrap();
        std::os::unix::fs::symlink(
            fixture.path("missing.svg"),
            fixture.path("home/.icons/hicolor/scalable/apps/example.png"),
        )
        .unwrap();
        let expected = fixture.icon("system/pixmaps/example.png");
        let resolver = fixture.resolver(None, &["system"]);
        assert_eq!(resolver.resolve("example"), Some(expected));
        for name in [
            "",
            "not-installed",
            "../example",
            "./example",
            "relative/example.svg",
        ] {
            assert_eq!(resolver.resolve(name), None, "{name}");
        }
    }

    #[test]
    fn flatpak_exports_local_icons_and_appimage_names_work() {
        let fixture = Fixture::new();
        let user_flatpak = fixture.icon("home/.local/share/flatpak/exports/share/icons/hicolor/128x128/apps/org.example.App.png");
        fixture.icon("system-flatpak/icons/hicolor/scalable/apps/org.example.App.svg");
        let system_flatpak =
            fixture.icon("system-flatpak/icons/hicolor/64x64/apps/org.example.Other.png");
        let local = fixture.icon("first/icons/hicolor/scalable/apps/local.svg");
        let appimage = fixture.icon("home/.local/share/icons/appimage_example.png");
        let webapp =
            fixture.icon("home/.local/share/icons/hicolor/scalable/apps/webapp-example.svg");
        let resolver = fixture.resolver(None, &["first", "system"]);
        for (name, expected) in [
            ("org.example.App", user_flatpak),
            ("org.example.Other", system_flatpak),
            ("local", local),
            ("appimage_example", appimage.clone()),
            ("appimage_example.png", appimage),
            ("webapp-example.svg", webapp),
        ] {
            assert_eq!(resolver.resolve(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn hicolor_preference_and_installed_theme_fallbacks_stay_within_root() {
        let fixture = Fixture::new();
        let hicolor = fixture.icon("data/icons/hicolor/48x48/apps/example.png");
        fixture.icon("data/icons/Adwaita/scalable/apps/example.svg");
        let themed = fixture.icon("data/icons/Adwaita/scalable/apps/themed.svg");
        let breeze = fixture.icon("data/icons/breeze/apps/48/breeze.svg");
        fixture.icon("system/icons/hicolor/scalable/apps/themed.svg");
        let resolver = fixture.resolver(Some(&fixture.path("data")), &["system"]);
        assert_eq!(resolver.resolve("example"), Some(hicolor));
        assert_eq!(resolver.resolve("themed"), Some(themed));
        assert_eq!(resolver.resolve("breeze"), Some(breeze));
    }
}
