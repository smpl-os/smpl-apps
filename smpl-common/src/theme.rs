//! Shared opaque palette colors and background-only opacity for native apps.

use slint::Color;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeRole {
    Application,
    Popup,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemePalette {
    pub bg: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub accent: Color,
    pub bg_light: Color,
    pub bg_lighter: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    pub info: Color,
    pub opacity: f32,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self {
            bg: Color::from_rgb_u8(30, 30, 46),
            fg: Color::from_rgb_u8(205, 214, 244),
            fg_dim: Color::from_rgb_u8(166, 173, 200),
            accent: Color::from_rgb_u8(137, 180, 250),
            bg_light: Color::from_rgb_u8(69, 71, 90),
            bg_lighter: Color::from_rgb_u8(88, 91, 112),
            danger: Color::from_rgb_u8(243, 139, 168),
            success: Color::from_rgb_u8(166, 227, 161),
            warning: Color::from_rgb_u8(249, 226, 175),
            info: Color::from_rgb_u8(148, 226, 213),
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeError(String);

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ThemeError {}

pub fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(Color::from_rgb_u8(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
    ))
}

pub fn parse_theme(content: &str, role: ThemeRole) -> Result<ThemePalette, ThemeError> {
    let mut vars = HashMap::new();
    for line in content.lines() {
        let line = line
            .split_once("//")
            .map_or(line, |(value, _)| value)
            .trim();
        let Some(line) = line.strip_prefix("$theme-") else {
            continue;
        };
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ThemeError(format!("invalid theme declaration: {line}")))?;
        let (value, trailing) = value
            .split_once(';')
            .ok_or_else(|| ThemeError(format!("unterminated theme declaration: {name}")))?;
        if !trailing.trim().is_empty() {
            return Err(ThemeError(format!(
                "unexpected content after theme value: {name}"
            )));
        }
        if vars.insert(name.trim(), value.trim()).is_some() {
            return Err(ThemeError(format!(
                "duplicate theme declaration: {}",
                name.trim()
            )));
        }
    }

    // These core colors distinguish a complete palette from an empty/partial write.
    for name in ["bg", "fg", "accent"] {
        if !vars.contains_key(name) {
            return Err(ThemeError(format!(
                "theme is missing required color: {name}"
            )));
        }
    }

    let mut palette = ThemePalette::default();
    for (name, field) in [
        ("bg", &mut palette.bg),
        ("fg", &mut palette.fg),
        ("fg-dim", &mut palette.fg_dim),
        ("accent", &mut palette.accent),
        ("bg-light", &mut palette.bg_light),
        ("bg-lighter", &mut palette.bg_lighter),
        ("danger", &mut palette.danger),
        ("success", &mut palette.success),
        ("warning", &mut palette.warning),
        ("info", &mut palette.info),
    ] {
        if let Some(value) = vars.get(name) {
            *field = parse_hex_color(value).ok_or_else(|| {
                ThemeError(format!("invalid opaque RGB color for {name}: {value}"))
            })?;
        }
    }

    let opacity = |name| -> Result<Option<f32>, ThemeError> {
        vars.get(name)
            .map(|value| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                    .map(|value| value as f32)
                    .ok_or_else(|| {
                        ThemeError(format!("invalid {name}: expected a finite value in [0, 1]"))
                    })
            })
            .transpose()
    };
    let popup = opacity("popup-opacity")?;
    let application = opacity("app-background-opacity")?;
    palette.opacity = match role {
        ThemeRole::Application => application.or(popup),
        ThemeRole::Popup => popup,
    }
    .unwrap_or(1.0);
    Ok(palette)
}

fn resolve_theme_paths(
    config: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<(PathBuf, Option<PathBuf>), ThemeError> {
    let legacy = home
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .map(|home| home.join(".config/eww/theme-colors.scss"));
    if let Some(config) = config {
        let config = PathBuf::from(config);
        if config.is_absolute() {
            let preferred = config.join("eww/theme-colors.scss");
            let fallback = legacy.filter(|path| path != &preferred);
            return Ok((preferred, fallback));
        }
    }
    legacy
        .map(|path| (path, None))
        .ok_or_else(|| ThemeError("cannot locate theme: HOME must be an absolute path".into()))
}

/// A polling reader that never replaces a good palette with a read/parse failure.
pub struct ThemeWatcher {
    path: PathBuf,
    fallback: Option<PathBuf>,
    role: ThemeRole,
    palette: ThemePalette,
    source: Option<String>,
}

impl ThemeWatcher {
    pub fn new(path: impl AsRef<Path>, role: ThemeRole) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            fallback: None,
            role,
            palette: ThemePalette::default(),
            source: None,
        }
    }

    pub fn palette(&self) -> &ThemePalette {
        &self.palette
    }

    pub fn reload(&mut self) -> Result<Option<ThemePalette>, ThemeError> {
        let (path, source) = match std::fs::read_to_string(&self.path) {
            Ok(source) => (&self.path, source),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && self.fallback.is_some() =>
            {
                let fallback = self.fallback.as_ref().expect("fallback checked above");
                let source = std::fs::read_to_string(fallback)
                    .map_err(|error| ThemeError(format!("{}: {error}", fallback.display())))?;
                (fallback, source)
            }
            Err(error) => {
                return Err(ThemeError(format!("{}: {error}", self.path.display())));
            }
        };
        if self.source.as_deref() == Some(&source) {
            return Ok(None);
        }
        let palette = parse_theme(&source, self.role)
            .map_err(|error| ThemeError(format!("{}: {error}", path.display())))?;
        let changed = self.source.is_none() || palette != self.palette;
        self.source = Some(source);
        self.palette = palette;
        Ok(changed.then(|| self.palette.clone()))
    }
}

/// Apply a palette immediately and refresh it every two seconds.
///
/// Keep the returned timer alive for the window lifetime. Errors go to stderr
/// once per distinct failure; a later successful read clears that diagnostic.
pub fn watch(role: ThemeRole, apply: impl Fn(&ThemePalette) + 'static) -> slint::Timer {
    let timer = slint::Timer::default();
    let (path, fallback) = match resolve_theme_paths(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    ) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("Theme: {error}; using opaque fallback palette");
            apply(&ThemePalette::default());
            return timer;
        }
    };
    let mut watcher = ThemeWatcher::new(path, role);
    watcher.fallback = fallback;
    let mut last_error = None;
    match watcher.reload() {
        Ok(_) => {}
        Err(error) => {
            eprintln!("Theme: {error}; using opaque fallback palette");
            last_error = Some(error);
        }
    }
    apply(watcher.palette());
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(2),
        move || match watcher.reload() {
            Ok(palette) => {
                last_error = None;
                if let Some(palette) = palette {
                    apply(&palette);
                }
            }
            Err(error) => {
                if last_error.as_ref() != Some(&error) {
                    eprintln!("Theme: {error}; keeping last good palette");
                }
                last_error = Some(error);
            }
        },
    );
    timer
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "$theme-bg: #1e1e2e;\n$theme-fg: #cdd6f4;\n$theme-accent: #89b4fa;\n";

    #[test]
    fn opacity_roles_and_legacy_fallback() {
        for (extra, application, popup) in [
            ("", 1.0, 1.0),
            ("$theme-popup-opacity: 0.5;", 0.5, 0.5),
            ("$theme-app-background-opacity: 0.7;", 0.7, 1.0),
            (
                "$theme-app-background-opacity: 0.7;\n$theme-popup-opacity: 0.5;",
                0.7,
                0.5,
            ),
            (
                "$theme-app-background-opacity: 0;\n$theme-popup-opacity: 1;",
                0.0,
                1.0,
            ),
        ] {
            let source = format!("{CORE}{extra}");
            assert_eq!(
                parse_theme(&source, ThemeRole::Application)
                    .unwrap()
                    .opacity,
                application
            );
            assert_eq!(
                parse_theme(&source, ThemeRole::Popup).unwrap().opacity,
                popup
            );
        }
    }

    #[test]
    fn rejects_invalid_explicit_opacity_without_fallback() {
        for value in [
            "NaN",
            "inf",
            "-inf",
            "-0.01",
            "1.01",
            "1.00000001",
            "50%",
            "no",
            "",
        ] {
            for key in ["popup-opacity", "app-background-opacity"] {
                let source = format!("{CORE}$theme-{key}: {value};");
                for role in [ThemeRole::Application, ThemeRole::Popup] {
                    assert!(parse_theme(&source, role).is_err(), "{key}={value}");
                }
            }
        }
    }

    #[test]
    fn foregrounds_are_opaque_and_invalid_colors_do_not_panic() {
        for value in ["#aé123", "#12345678", "#abcd", "notrgb"] {
            assert!(parse_hex_color(value).is_none());
        }
        let palette = parse_theme(CORE, ThemeRole::Application).unwrap();
        for color in [
            palette.bg,
            palette.fg,
            palette.fg_dim,
            palette.accent,
            palette.bg_light,
            palette.bg_lighter,
            palette.danger,
            palette.success,
            palette.warning,
            palette.info,
        ] {
            assert_eq!(color.alpha(), 255);
        }
    }

    #[test]
    fn accepts_generated_comments_and_rejects_incomplete_palettes() {
        assert!(parse_theme(
            &format!("// generated\n{CORE}$theme-popup-opacity: .5; // glass\n"),
            ThemeRole::Popup
        )
        .is_ok());
        for source in ["", "$theme-bg: #1e1e2e;", "$theme-bg: #1e1e2e"] {
            assert!(parse_theme(source, ThemeRole::Popup).is_err());
        }
        assert!(parse_theme(&format!("{CORE}$theme-bg: #000000;"), ThemeRole::Popup).is_err());
    }

    #[test]
    fn theme_location_obeys_absolute_xdg_and_home_fallback() {
        assert_eq!(
            resolve_theme_paths(Some("/config".into()), Some("/home/user".into())).unwrap(),
            (
                PathBuf::from("/config/eww/theme-colors.scss"),
                Some(PathBuf::from("/home/user/.config/eww/theme-colors.scss"))
            )
        );
        for config in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                resolve_theme_paths(config, Some("/home/user".into())).unwrap(),
                (
                    PathBuf::from("/home/user/.config/eww/theme-colors.scss"),
                    None
                )
            );
        }
        assert!(resolve_theme_paths(None, None).is_err());
        assert!(resolve_theme_paths(None, Some("relative".into())).is_err());
    }

    #[test]
    fn reload_retains_last_good_and_recovers_after_atomic_replacement() {
        let dir = std::env::temp_dir().join(format!(
            "smpl-theme-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("theme.scss");
        let mut watcher = ThemeWatcher::new(&path, ThemeRole::Application);
        assert!(watcher.reload().is_err());
        assert_eq!(watcher.palette().opacity, 1.0);
        std::fs::write(&path, format!("{CORE}$theme-popup-opacity: .55;")).unwrap();
        assert_eq!(watcher.reload().unwrap().unwrap().opacity, 0.55);
        let preferred = dir.join("xdg-theme.scss");
        let mut discovery = ThemeWatcher::new(&preferred, ThemeRole::Application);
        discovery.fallback = Some(path.clone());
        assert_eq!(discovery.reload().unwrap().unwrap().opacity, 0.55);
        std::fs::write(&preferred, "invalid preferred file").unwrap();
        assert!(discovery.reload().is_err());
        assert_eq!(discovery.palette().opacity, 0.55);
        std::fs::write(
            &preferred,
            format!("{CORE}$theme-app-background-opacity: .8;"),
        )
        .unwrap();
        assert_eq!(discovery.reload().unwrap().unwrap().opacity, 0.8);
        std::fs::remove_file(&preferred).unwrap();
        assert_eq!(discovery.reload().unwrap().unwrap().opacity, 0.55);
        std::fs::create_dir(&preferred).unwrap();
        assert!(discovery.reload().is_err());
        std::fs::remove_dir(&preferred).unwrap();
        assert!(watcher.reload().unwrap().is_none());
        for source in [
            "",
            "$theme-bg: #bad;",
            &format!("{CORE}$theme-popup-opacity: NaN;"),
        ] {
            std::fs::write(&path, source).unwrap();
            assert!(watcher.reload().is_err());
            assert_eq!(watcher.palette().opacity, 0.55);
        }
        std::fs::remove_file(&path).unwrap();
        assert!(watcher.reload().is_err());
        assert_eq!(watcher.palette().opacity, 0.55);
        let replacement = dir.join("replacement.scss");
        std::fs::write(
            &replacement,
            format!("{CORE}$theme-app-background-opacity: 1;"),
        )
        .unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(watcher.reload().unwrap().unwrap().opacity, 1.0);
        std::fs::write(&path, format!("{CORE}$theme-app-background-opacity: .55;")).unwrap();
        assert_eq!(watcher.reload().unwrap().unwrap().opacity, 0.55);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
