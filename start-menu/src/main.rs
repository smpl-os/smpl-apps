mod commands;
mod icons;
mod instance;
mod signals;
mod stamp;
mod usage;

use i_slint_backend_winit::WinitWindowAccessor;
use smpl_common::theme::{self, ThemePalette, ThemeRole};
use slint::{Image, Model, ModelRc, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use crate::icons::IconResolver;
use crate::signals::Request;
use crate::stamp::FileStamp;
use crate::usage::Usage;

slint::include_modules!();

// ── App index entry (parsed from ~/.cache/smplos/app_index) ──

#[derive(Clone, Debug)]
struct AppEntry {
    name: String,
    exec: String,
    category: String,
    #[allow(dead_code)]
    icon: String,
    /// True when this entry should appear in search results but NOT in the
    /// browse (category) view.  Used for settings card-level keywords so users
    /// can type "resolution" or "power saver" and land on the right tab without
    /// cluttering the Settings category list.
    search_only: bool,
}

// ── Category definitions ──

const CATEGORIES: &[(&str, &str, &str)] = &[
    ("internet",    "Internet", "\u{f0ac3}"),  // 󰫃 globe
    ("multimedia",  "Media",    "\u{f038a}"),  // 󰎊 music
    ("office",      "Office",   "\u{f0219}"),  // 󰈙 document
    ("development", "Dev",      "\u{f0169}"),  // 󰅩 code
    ("graphics",    "Graphics", "\u{f03e8}"),  // 󰏨 palette
    ("apps",        "Apps",     "\u{f00bb}"),  // 󰂻 grid
    ("settings",    "Settings", "\u{f0493}"),  // 󰒓 gear
];

fn category_label(key: &str) -> &str {
    CATEGORIES
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, v, _)| *v)
        .unwrap_or(key)
}

// ── Load the app index cache ──

fn app_index_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{}/.cache/smplos/app_index", home))
}

/// Read the app index together with the stamp taken *before* reading, so a
/// rewrite racing with the read still differs from the stamp next time.
/// `None` only when the cache is unreadable and no rebuild was requested.
fn read_app_index(
    path: &Path,
    rebuild_if_missing: bool,
) -> Option<(Vec<AppEntry>, Option<FileStamp>)> {
    let mut stamp = FileStamp::of(path);
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) if rebuild_if_missing => {
            // Try rebuilding the cache if it doesn't exist
            let _ = std::process::Command::new("rebuild-app-cache").status();
            stamp = FileStamp::of(path);
            std::fs::read_to_string(path).unwrap_or_default()
        }
        Err(_) => return None,
    };
    Some((parse_app_index(&content), stamp))
}

fn parse_app_index(content: &str) -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(5, ';').collect();
        if parts.len() < 3 {
            continue;
        }

        let name = parts[0].to_string();
        let exec = parts[1].to_string();
        let category = parts[2].to_string();
        let icon = parts.get(3).unwrap_or(&"").to_string();
        let search_only = parts.get(4).map(|s| s.trim() == "1").unwrap_or(false);

        // Deduplicate by lowercase name
        if !seen.insert(name.to_lowercase()) {
            continue;
        }

        apps.push(AppEntry {
            name,
            exec,
            category,
            icon,
            search_only,
        });
    }

    apps
}

// ── Pinned apps persistence ──

fn pinned_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{}/.config/smplos/pinned-apps.txt", home))
}

fn load_pinned() -> Vec<String> {
    std::fs::read_to_string(pinned_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

fn save_pinned(pinned: &[String]) {
    let path = pinned_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, pinned.join("\n") + "\n");
}

// ── Icon resolution ──

/// Pre-resolve icon paths (fast: just stat calls, no image decoding).
fn build_icon_path_cache<'a>(
    apps: impl IntoIterator<Item = &'a AppEntry>,
    resolver: &IconResolver,
) -> HashMap<String, PathBuf> {
    let mut cache = HashMap::new();
    for app in apps {
        if app.icon.is_empty() || cache.contains_key(&app.icon) {
            continue;
        }
        if let Some(path) = resolver.resolve(&app.icon) {
            cache.insert(app.icon.clone(), path);
        }
    }
    cache
}

// ── Filter: global search or category browse ──

/// In Settings browse mode (no query), only smpl-managed entries are shown.
/// System utilities (Blueman, Kvantum, nm-connection-editor, Qt5, etc.) that
/// happen to have the "Settings" XDG category are hidden — they clutter the
/// list and users should access them through the relevant settings tab instead.
/// The whitelist covers the top-level smpl apps that live in the settings area.
fn is_settings_browse_visible(app: &AppEntry) -> bool {
    app.exec.starts_with("smplos-settings")
        || matches!(
            app.exec.as_str(),
            "toggle-app-center" | "webapp-center" | "sync-center-gui"
        )
}

#[cfg(test)]
fn filter_apps(all: &[AppEntry], category_key: &str, query: &str) -> Vec<AppEntry> {
    filter_and_rank(all, category_key, query, &Usage::default(), 0)
}

/// Fuzzy-match `query` against `name`. Returns None only when neither nucleo
/// nor the typo-tolerant fallback accept the pair; Some(score) otherwise
/// (higher is better — see nucleo-matcher docs for the scoring model).
///
/// Tried in order (cheapest first):
///   1. nucleo — fzf-style subsequence match with quality scoring.
///   2. Damerau–Levenshtein fallback for typos nucleo can't reach, e.g.
///      "pwoer"→"Power Profile" (single adjacent transposition). Returns a
///      small constant score so real subsequence matches always rank higher.
///
/// `name` and `query` are converted on each call; this is fine because we run
/// at most once per visible app per keystroke (≈ a few hundred ops/frame).
fn fuzzy_score(matcher: &mut nucleo_matcher::Matcher, name: &str, query: &str) -> Option<u16> {
    use nucleo_matcher::Utf32Str;
    let mut buf_name = Vec::new();
    let mut buf_q = Vec::new();
    let n = Utf32Str::new(name, &mut buf_name);
    let q = Utf32Str::new(query, &mut buf_q);
    if let Some(s) = matcher.fuzzy_match(n, q) {
        return Some(s);
    }
    // Nucleo said no — try a typo-tolerant Damerau-Levenshtein pass so that
    // transpositions like "pwoer"→"power" still find the right app. Only
    // reached when subsequence match failed, so this is not on the hot path
    // for well-typed queries.
    if typo_tolerant_match(name, query) {
        // Fixed low score keeps typo hits below real fuzzy matches in ranking.
        Some(1)
    } else {
        None
    }
}

/// True when `query` matches any whitespace-separated word in `name` within a
/// small Damerau-Levenshtein budget. Budget grows with query length so short
/// queries don't collapse into "matches everything".
fn typo_tolerant_match(name: &str, query: &str) -> bool {
    let query_lc = query.to_lowercase();
    let query_chars: Vec<char> = query_lc.chars().collect();
    if query_chars.len() < 4 {
        // Too short to safely allow edits without matching everything.
        return false;
    }
    let threshold = 1 + query_chars.len() / 4;
    let name_lc = name.to_lowercase();
    for word in name_lc.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let word_chars: Vec<char> = word.chars().collect();
        if word_chars.len().abs_diff(query_chars.len()) > threshold {
            continue;
        }
        if damerau_levenshtein(&word_chars, &query_chars) <= threshold {
            return true;
        }
    }
    false
}

/// Optimal-String-Alignment (restricted Damerau–Levenshtein) distance.
/// Counts insertions, deletions, substitutions, and single adjacent
/// transpositions. Lets "pwoer" match "power" with distance 1.
fn damerau_levenshtein(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate().take(n + 1) {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate().take(m + 1) {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

/// Filter by category/search, then in search mode rank by:
///   1. frecency score (descending) — apps you actually launch float to top
///   2. fuzzy score   (descending) — fzf-style subsequence quality (handles
///      "vsc"→"VS Code", "vstdio"→"Visual Studio Code", typos, etc.)
///   3. name          (ascending, case-insensitive) — stable tiebreaker
///
/// In search mode an app is included only if the fuzzy matcher accepts it
/// (subsequence match, smart-case). `now` is passed in for deterministic tests.
fn filter_and_rank(
    all: &[AppEntry],
    category_key: &str,
    query: &str,
    usage: &Usage,
    now: u64,
) -> Vec<AppEntry> {
    let q = query.trim();

    if q.is_empty() {
        // Browse mode: filter by category, hide search-only entries.
        return all
            .iter()
            .filter(|app| {
                if app.search_only { return false; }
                if app.category != category_key { return false; }
                if category_key == "settings" {
                    return is_settings_browse_visible(app);
                }
                true
            })
            .cloned()
            .collect();
    }

    // Search mode: fuzzy-match against name, drop non-matches, then sort.
    let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);

    let mut scored: Vec<(AppEntry, u16)> = all
        .iter()
        .filter_map(|app| {
            // Include ALL entries in search (search_only too) so card keywords
            // like "resolution" or "power saver" surface the right tab.
            fuzzy_score(&mut matcher, &app.name, q).map(|s| (app.clone(), s))
        })
        .collect();

    scored.sort_by(|a, b| {
        let fa = usage.score(&a.0.exec, now);
        let fb = usage.score(&b.0.exec, now);
        fb.partial_cmp(&fa)                                          // frecency desc
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))                              // fuzzy score desc
            .then_with(|| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase()))
    });

    scored.into_iter().map(|(a, _)| a).collect()
}

// ── Convert to Slint model item ──

fn to_ui_item(app: &AppEntry, path_cache: &HashMap<String, PathBuf>, img_cache: &RefCell<HashMap<String, Image>>, pinned: &[String]) -> AppItem {
    let initial = app
        .name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();

    // Lazy image load: check image cache first, then try loading from path cache
    let (icon_image, has_icon) = {
        let cache = img_cache.borrow();
        if let Some(img) = cache.get(&app.icon) {
            (img.clone(), true)
        } else {
            drop(cache);
            if let Some(path) = path_cache.get(&app.icon) {
                match Image::load_from_path(path) {
                    Ok(img) => {
                        img_cache.borrow_mut().insert(app.icon.clone(), img.clone());
                        (img, true)
                    }
                    Err(error) => {
                        eprintln!("start-menu: cannot load icon {}: {error}", path.display());
                        (Image::default(), false)
                    }
                }
            } else {
                (Image::default(), false)
            }
        }
    };

    let is_web_app = app.exec.contains("launch-webapp");

    let source = if is_web_app {
        "Web App"
    } else if app.exec.contains("flatpak run") || app.exec.contains("/flatpak/") {
        "Flatpak"
    } else if app.exec.ends_with(".AppImage") || app.exec.ends_with(".appimage") {
        "AppImage"
    } else if app.exec.starts_with("smplos-settings") {
        ""
    } else {
        "AUR"
    };

    AppItem {
        name: SharedString::from(&app.name),
        exec: SharedString::from(&app.exec),
        category: SharedString::from(category_label(&app.category)),
        category_key: SharedString::from(&app.category),
        initial: SharedString::from(&initial),
        icon_image,
        has_icon,
        is_web_app,
        source: SharedString::from(source),
        is_pinned: commands::is_pinned(pinned, &app.exec),
    }
}

// ── Update the visible app list ──

#[allow(clippy::too_many_arguments)]
fn update_view(
    ui: &MainWindow,
    all_apps: &[AppEntry],
    model: &Rc<VecModel<AppItem>>,
    path_cache: &HashMap<String, PathBuf>,
    img_cache: &RefCell<HashMap<String, Image>>,
    category_key: &str,
    query: &str,
    pinned: &[String],
    usage: &Usage,
) {
    let filtered = filter_and_rank(all_apps, category_key, query, usage, usage::now_unix());
    model.set_vec(filtered.iter().map(|a| to_ui_item(a, path_cache, img_cache, pinned)).collect::<Vec<_>>());
    ui.set_apps(ModelRc::from(model.clone()));
    ui.set_app_count(model.row_count() as i32);
    ui.set_selected_app(if model.row_count() > 0 { 0 } else { -1 });
    ui.set_is_searching(!query.trim().is_empty());
}

fn pinned_apps<'a>(all_apps: &'a [AppEntry], pinned: &[String]) -> Vec<&'a AppEntry> {
    pinned.iter().filter_map(|exec| {
        all_apps.iter().find(|app| app.exec == *exec)
            .or_else(|| all_apps.iter().find(|app| commands::equivalent(&app.exec, exec)))
    }).collect()
}

fn update_pinned_model(
    ui: &MainWindow,
    all_apps: &[AppEntry],
    pinned_model: &Rc<VecModel<AppItem>>,
    path_cache: &HashMap<String, PathBuf>,
    img_cache: &RefCell<HashMap<String, Image>>,
    pinned: &[String],
) {
    let items: Vec<AppItem> = pinned_apps(all_apps, pinned)
        .into_iter()
        .map(|a| to_ui_item(a, path_cache, img_cache, pinned))
        .collect();
    let count = items.len() as i32;
    pinned_model.set_vec(items);
    ui.set_pinned_apps(ModelRc::from(pinned_model.clone()));
    ui.set_pinned_count(count);
}

// ── Theme ──

fn apply_theme(ui: &MainWindow, palette: &ThemePalette) {
    let theme = Theme::get(ui);
    theme.set_bg(palette.bg);
    theme.set_fg(palette.fg);
    theme.set_fg_dim(palette.fg_dim);
    theme.set_accent(palette.accent);
    theme.set_bg_light(palette.bg_light);
    theme.set_bg_lighter(palette.bg_lighter);
    theme.set_danger(palette.danger);
    theme.set_success(palette.success);
    theme.set_warning(palette.warning);
    theme.set_info(palette.info);
    theme.set_opacity(palette.opacity);
}

// ── Hidden-state reset (resident mode) ──

/// The view state that hiding resets, as the generated window's setters.
/// `MainWindow` implements it; tests use a recording fake.
trait MenuView {
    fn set_search_text(&self, text: SharedString);
    fn set_active_category(&self, index: i32);
    fn set_apps(&self, apps: ModelRc<AppItem>);
    fn set_app_count(&self, count: i32);
    fn set_selected_app(&self, index: i32);
    fn set_is_searching(&self, searching: bool);
    fn set_show_context_menu(&self, shown: bool);
    fn set_show_power_menu(&self, shown: bool);
    fn invoke_reset_scroll(&self);
    fn release_input(&self);
    fn invoke_focus_search(&self);
}

impl MenuView for MainWindow {
    fn set_search_text(&self, text: SharedString) {
        MainWindow::set_search_text(self, text)
    }
    fn set_active_category(&self, index: i32) {
        MainWindow::set_active_category(self, index)
    }
    fn set_apps(&self, apps: ModelRc<AppItem>) {
        MainWindow::set_apps(self, apps)
    }
    fn set_app_count(&self, count: i32) {
        MainWindow::set_app_count(self, count)
    }
    fn set_selected_app(&self, index: i32) {
        MainWindow::set_selected_app(self, index)
    }
    fn set_is_searching(&self, searching: bool) {
        MainWindow::set_is_searching(self, searching)
    }
    fn set_show_context_menu(&self, shown: bool) {
        MainWindow::set_show_context_menu(self, shown)
    }
    fn set_show_power_menu(&self, shown: bool) {
        MainWindow::set_show_power_menu(self, shown)
    }
    fn invoke_reset_scroll(&self) {
        MainWindow::invoke_reset_scroll(self)
    }
    fn release_input(&self) {
        // The destroyed window never sees the pointer leave or held keys go
        // up. Without this the last hovered button stays highlighted, and a
        // Ctrl still held from Ctrl+W/Ctrl+Y would keep Slint's modifiers set
        // (refocusing an already active window does not reset them), turning
        // typed letters into shortcuts on the next show.
        let window = self.window();
        let _ = window.try_dispatch_event(slint::platform::WindowEvent::PointerExited);
        let _ = window.try_dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(false));
    }
    fn invoke_focus_search(&self) {
        MainWindow::invoke_focus_search(self)
    }
}

/// Return to the startup view: no search, no category, empty list, nothing
/// selected, menus closed, list scrolled to the top, search focused.
fn reset_view(view: &impl MenuView, apps: &Rc<VecModel<AppItem>>) {
    view.set_show_context_menu(false);
    view.set_show_power_menu(false);
    view.set_search_text(SharedString::new());
    view.set_active_category(-1);
    apps.set_vec(Vec::new());
    view.set_apps(ModelRc::from(apps.clone()));
    view.set_app_count(0);
    view.set_selected_app(-1);
    view.set_is_searching(false);
    view.invoke_reset_scroll();
    view.release_input();
    view.invoke_focus_search();
}

// ── Launcher state shared by the UI callbacks ──

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifetime {
    /// Plain `start-menu`: closing or launching exits the process.
    ExitOnClose,
    /// `start-menu --resident`: closing or launching hides and resets.
    Resident,
}

/// Stamps of the files behind the loaded data, compared on every show.
struct LoadedStamps {
    app_index: Option<FileStamp>,
    pinned: Option<FileStamp>,
    usage: Option<FileStamp>,
}

/// Commands started by a resident menu. A plain launch exits right after
/// spawning; a resident one must reap its children so they do not linger as
/// zombies.
#[derive(Default)]
struct Children(RefCell<Vec<std::process::Child>>);

impl Children {
    fn adopt(&self, child: std::process::Child) {
        self.0.borrow_mut().push(child);
    }

    /// Reap exited children; returns how many are still running.
    fn reap(&self) -> usize {
        let mut children = self.0.borrow_mut();
        children.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
        children.len()
    }
}

struct Launcher {
    ui: slint::Weak<MainWindow>,
    lifetime: Cell<Lifetime>,
    apps: RefCell<Vec<AppEntry>>,
    icon_resolver: RefCell<IconResolver>,
    path_cache: RefCell<HashMap<String, PathBuf>>,
    path_cache_primed: Cell<bool>,
    img_cache: RefCell<HashMap<String, Image>>,
    model: Rc<VecModel<AppItem>>,
    pinned_model: Rc<VecModel<AppItem>>,
    cat_model: Rc<VecModel<CategoryItem>>,
    pinned: RefCell<Vec<String>>,
    usage: RefCell<Usage>,
    loaded: RefCell<LoadedStamps>,
    children: Children,
}

impl Launcher {
    fn new(ui: &MainWindow, lifetime: Lifetime) -> Rc<Self> {
        // ── Load all apps from cache ──
        let (apps, app_index) = read_app_index(&app_index_path(), true).unwrap_or_default();
        let icon_resolver = IconResolver::from_env();
        // Defer full icon path scan to first category/search interaction.
        // Startup only pre-resolves icons for the pinned row (typically ≤5 entries)
        // instead of running stat() against 10+ dirs for every app in the index.
        let pinned_stamp = FileStamp::of(&pinned_path());
        let pinned = load_pinned();
        let path_cache = build_icon_path_cache(pinned_apps(&apps, &pinned), &icon_resolver);
        let usage_stamp = usage::state_path().and_then(|path| FileStamp::of(&path));
        let usage = Usage::load();
        let launcher = Rc::new(Self {
            ui: ui.as_weak(),
            lifetime: Cell::new(lifetime),
            apps: RefCell::new(apps),
            icon_resolver: RefCell::new(icon_resolver),
            path_cache: RefCell::new(path_cache),
            path_cache_primed: Cell::new(false),
            img_cache: RefCell::new(HashMap::new()),
            model: Rc::new(VecModel::default()),
            pinned_model: Rc::new(VecModel::default()),
            cat_model: Rc::new(VecModel::default()),
            pinned: RefCell::new(pinned),
            usage: RefCell::new(usage),
            loaded: RefCell::new(LoadedStamps {
                app_index,
                pinned: pinned_stamp,
                usage: usage_stamp,
            }),
            children: Children::default(),
        });

        // ── Build category sidebar ──
        ui.set_categories(ModelRc::from(launcher.cat_model.clone()));
        launcher.rebuild_categories(ui);

        // ── Initial view: empty (no category selected, no search) ──
        ui.set_active_category(-1);
        ui.set_app_count(0);

        // ── Load pinned apps ──
        launcher.update_pinned(ui);
        launcher
    }

    fn rebuild_categories(&self, ui: &MainWindow) {
        let apps = self.apps.borrow();
        let mut items = Vec::new();
        for (key, display, icon) in CATEGORIES {
            let count = apps.iter().filter(|a| a.category == *key).count();
            // Skip empty categories (but always keep settings)
            if count == 0 && *key != "settings" {
                continue;
            }
            items.push(CategoryItem {
                key: SharedString::from(*key),
                label: SharedString::from(*display),
                icon: SharedString::from(*icon),
            });
        }
        self.cat_model.set_vec(items);
        ui.set_category_count(self.cat_model.row_count() as i32);
    }

    fn update_pinned(&self, ui: &MainWindow) {
        let cache = self.path_cache.borrow();
        update_pinned_model(
            ui,
            &self.apps.borrow(),
            &self.pinned_model,
            &cache,
            &self.img_cache,
            &self.pinned.borrow(),
        );
    }

    /// Resolve every app's icon path once, on first interactive use.
    fn prime_path_cache(&self) {
        if !self.path_cache_primed.get() {
            let cache = build_icon_path_cache(self.apps.borrow().iter(), &self.icon_resolver.borrow());
            *self.path_cache.borrow_mut() = cache;
            self.path_cache_primed.set(true);
        }
    }

    fn category_key(&self, index: usize) -> String {
        self.cat_model
            .row_data(index)
            .map(|c| c.key.to_string())
            .unwrap_or_else(|| "all".to_string())
    }

    fn show_matches(&self, ui: &MainWindow, category_key: &str, query: &str) {
        let cache = self.path_cache.borrow();
        update_view(
            ui,
            &self.apps.borrow(),
            &self.model,
            &cache,
            &self.img_cache,
            category_key,
            query,
            &self.pinned.borrow(),
            &self.usage.borrow(),
        );
    }

    /// Search text or category changed.
    fn filter_changed(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        ui.set_show_context_menu(false);
        let search = ui.get_search_text().to_string();
        let cat_idx = ui.get_active_category() as usize;

        // No category selected and no search => show nothing
        if search.is_empty() && cat_idx >= self.cat_model.row_count() {
            self.model.set_vec(Vec::new());
            ui.set_apps(ModelRc::from(self.model.clone()));
            ui.set_app_count(0);
            ui.set_selected_app(-1);
            ui.set_is_searching(false);
            return;
        }

        let cat_key = self.category_key(cat_idx);
        self.prime_path_cache();
        self.show_matches(&ui, &cat_key, &search);
    }

    fn toggle_pin(&self, exec: &str) {
        let Some(ui) = self.ui.upgrade() else { return; };
        {
            let mut p = self.pinned.borrow_mut();
            commands::toggle_pin(&mut p, exec);
            save_pinned(&p);
        }
        // Prime full icon cache if not done yet (user may pin before opening any category)
        self.prime_path_cache();
        self.update_pinned(&ui);
        // Refresh current view to update is_pinned flags
        let search = ui.get_search_text().to_string();
        let cat_idx = ui.get_active_category() as usize;
        if !search.is_empty() || cat_idx < self.cat_model.row_count() {
            let cat_key = self.category_key(cat_idx);
            self.show_matches(&ui, &cat_key, &search);
        }
    }

    /// Record the launch for frecency ranking, run it, then close.
    fn launch(&self, exec: &str) {
        {
            let mut u = self.usage.borrow_mut();
            u.record(exec, usage::now_unix());
            u.save();
        }
        self.spawn_shell(exec);
        self.close();
    }

    fn spawn_shell(&self, command: &str) {
        match std::process::Command::new("sh").arg("-c").arg(command).spawn() {
            Ok(child) if self.lifetime.get() == Lifetime::Resident => self.children.adopt(child),
            Ok(_) => {}
            Err(error) => eprintln!("start-menu: cannot run {command:?}: {error}"),
        }
    }

    /// Escape, compositor close, launching and the toolbar actions end here.
    fn close(&self) {
        match self.lifetime.get() {
            Lifetime::ExitOnClose => std::process::exit(0),
            Lifetime::Resident => self.hide(),
        }
    }

    fn is_shown(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| ui.window().is_visible())
    }

    fn show(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        self.children.reap();
        if !ui.window().is_visible() {
            trace("show");
            self.refresh_sources(&ui);
            if let Err(error) = ui.show() {
                eprintln!("start-menu: cannot show the menu: {error}");
                return;
            }
            // After `--hidden`, Slint has already created the (unmapped)
            // Wayland window and skipped its first frame while hidden; showing
            // an existing window requests no new frame, and without a frame
            // the compositor never maps it. Recreated windows draw anyway.
            ui.window().request_redraw();
        }
        ui.invoke_focus_search();
    }

    /// Idempotent: a hidden menu was already reset when it was hidden.
    fn hide(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if !ui.window().is_visible() {
            return;
        }
        trace("hide");
        // Wayland cannot unmap a window: Slint destroys it and recreates it on
        // show, keeping the component, fonts and loaded GL libraries.
        if let Err(error) = ui.hide() {
            eprintln!("start-menu: cannot hide the menu: {error}");
        }
        reset_view(&ui, &self.model);
        self.children.reap();
    }

    fn handle(&self, request: Request) {
        match request {
            Request::Toggle if self.is_shown() => self.close(),
            Request::Toggle | Request::Show => self.show(),
            Request::Hide => self.close(),
            Request::Quit => {
                let _ = slint::quit_event_loop();
            }
            Request::Reap => {
                self.children.reap();
            }
        }
    }

    /// Re-read the app index, pins and usage if their files changed since
    /// they were loaded; unchanged files cost one stat each.
    fn refresh_sources(&self, ui: &MainWindow) {
        let index_path = app_index_path();
        let mut apps_changed = false;
        // A missing index is being rebuilt; keep the last good list.
        if stamp::should_reload(self.loaded.borrow().app_index, FileStamp::of(&index_path), false) {
            if let Some((apps, read_at)) = read_app_index(&index_path, false) {
                *self.apps.borrow_mut() = apps;
                self.loaded.borrow_mut().app_index = read_at;
                apps_changed = true;
            }
        }
        let pinned_now = FileStamp::of(&pinned_path());
        let pins_changed = stamp::should_reload(self.loaded.borrow().pinned, pinned_now, true);
        if pins_changed {
            *self.pinned.borrow_mut() = load_pinned();
            self.loaded.borrow_mut().pinned = pinned_now;
        }
        if let Some(path) = usage::state_path() {
            let usage_now = FileStamp::of(&path);
            if stamp::should_reload(self.loaded.borrow().usage, usage_now, true) {
                *self.usage.borrow_mut() = Usage::load();
                self.loaded.borrow_mut().usage = usage_now;
            }
        }
        if apps_changed {
            trace("app index reloaded");
            self.rebuild_categories(ui);
            // New installs may add icon directories, so rescan the roots too.
            *self.icon_resolver.borrow_mut() = IconResolver::from_env();
            self.img_cache.borrow_mut().clear();
            self.path_cache.borrow_mut().clear();
            self.path_cache_primed.set(false);
        }
        if apps_changed || pins_changed {
            if !self.path_cache_primed.get() {
                let pinned_paths = build_icon_path_cache(
                    pinned_apps(&self.apps.borrow(), &self.pinned.borrow()),
                    &self.icon_resolver.borrow(),
                );
                self.path_cache.borrow_mut().extend(pinned_paths);
            }
            self.update_pinned(ui);
        }
    }
}

fn bind_callbacks(launcher: &Rc<Launcher>, ui: &MainWindow) {
    // ── Search pop-char callback (Backspace from FocusScope) ──
    {
        let ui_weak = ui.as_weak();
        ui.on_search_pop_char(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = ui.get_search_text().to_string();
            if !s.is_empty() {
                s.pop();
                ui.set_search_text(s.into());
            }
        });
    }

    // ── Filter callback (search text or category changed) ──
    let l = launcher.clone();
    ui.on_filter_changed(move || l.filter_changed());

    // ── Launch app ──
    let l = launcher.clone();
    ui.on_launch_app(move |index| {
        if let Some(item) = l.model.row_data(index as usize) {
            l.launch(item.exec.as_str());
        }
    });

    // ── Launch pinned app ──
    let l = launcher.clone();
    ui.on_launch_pinned(move |index| {
        if let Some(item) = l.pinned_model.row_data(index as usize) {
            l.launch(item.exec.as_str());
        }
    });

    // ── Toggle pin ──
    let l = launcher.clone();
    ui.on_toggle_pin(move |exec| l.toggle_pin(exec.as_str()));

    // ── Close ──
    let l = launcher.clone();
    ui.on_close(move || {
        trace("close: Escape");
        l.close();
    });

    // ── Open Web App Center ──
    let l = launcher.clone();
    ui.on_open_webapp_center(move || {
        l.spawn_shell("webapp-center");
        l.close();
    });

    // ── Open Sync Center ──
    let l = launcher.clone();
    ui.on_open_sync_center(move || {
        l.spawn_shell("sync-center-gui");
        l.close();
    });

    // ── Power actions ──
    let l = launcher.clone();
    ui.on_power_action(move |action| {
        let cmd = match action.as_str() {
            "lock" => "lock-screen",
            "sleep" => "systemctl suspend",
            "restart" => "systemctl reboot",
            "shutdown" => "systemctl poweroff",
            _ => return,
        };
        l.spawn_shell(cmd);
        l.close();
    });

    // ── Window drag ──
    {
        let ui_weak = ui.as_weak();
        ui.on_start_drag(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.window().with_winit_window(
                    |winit_win: &i_slint_backend_winit::winit::window::Window| {
                        let _ = winit_win.drag_window();
                    },
                );
            }
        });
    }
}

/// The window, its launcher, and the theme timer that must live as long.
struct Menu {
    ui: MainWindow,
    launcher: Rc<Launcher>,
    _theme_timer: slint::Timer,
}

impl Menu {
    fn new(lifetime: Lifetime) -> Result<Self, slint::PlatformError> {
        let ui = MainWindow::new()?;
        let ui_weak = ui.as_weak();
        let _theme_timer = theme::watch(ThemeRole::Popup, move |palette| {
            if let Some(ui) = ui_weak.upgrade() {
                apply_theme(&ui, palette);
            }
        });
        let launcher = Launcher::new(&ui, lifetime);
        bind_callbacks(&launcher, &ui);
        ui.set_version(format!("v{}", env!("CARGO_PKG_VERSION")).into());
        Ok(Self {
            ui,
            launcher,
            _theme_timer,
        })
    }
}

// ── Entry point ──

#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Version,
    /// No flags: show once, exit on close or launch.
    Legacy,
    Resident { hidden: bool },
}

fn parse_invocation(args: impl IntoIterator<Item = OsString>) -> Invocation {
    let (mut resident, mut hidden) = (false, false);
    for arg in args {
        match arg.to_str() {
            Some("-v" | "--version") => return Invocation::Version,
            Some("--resident") => resident = true,
            Some("--hidden") => hidden = true,
            // Earlier releases ignored other arguments; keep doing so.
            _ => {}
        }
    }
    if resident {
        Invocation::Resident { hidden }
    } else {
        Invocation::Legacy
    }
}

/// OS scripts can only probe safely with `--version` (older builds open the
/// menu for any other argument); they print just the first line.
fn version_text() -> String {
    format!(
        "start-menu v{}\nfeatures: resident\n",
        env!("CARGO_PKG_VERSION")
    )
}

fn main() -> Result<(), slint::PlatformError> {
    match parse_invocation(std::env::args_os().skip(1)) {
        Invocation::Version => {
            print!("{}", version_text());
            Ok(())
        }
        Invocation::Legacy => run_once(),
        Invocation::Resident { hidden } => run_resident(hidden),
    }
}

fn run_once() -> Result<(), slint::PlatformError> {
    smpl_common::init("start-menu", 520.0, 580.0)?;
    let menu = Menu::new(Lifetime::ExitOnClose)?;
    menu.ui.invoke_focus_search();
    menu.ui.run()
}

thread_local! {
    /// The resident launcher, for requests forwarded by the signal thread.
    static RESIDENT: RefCell<Option<Rc<Launcher>>> = const { RefCell::new(None) };
}

fn run_resident(hidden: bool) -> Result<(), slint::PlatformError> {
    // Before any thread exists, so that every later thread inherits the mask.
    let blocked = signals::block().map_err(|error| {
        slint::PlatformError::Other(format!("cannot block signals: {error}"))
    })?;
    init_trace();
    let pid = std::process::id();
    let pidfile = instance::pidfile_path();
    let _instance_lock = match instance::start(&pidfile, pid, !hidden) {
        Ok(instance::Startup::Resident(lock)) => lock,
        Ok(instance::Startup::HandedOff(existing)) => {
            trace(&format!("resident {existing} is running; exiting"));
            return Ok(());
        }
        Err(error) => {
            eprintln!("start-menu: {error}");
            std::process::exit(1);
        }
    };

    smpl_common::init("start-menu", 520.0, 580.0)?;
    // Hiding destroys the Wayland window and showing recreates it without the
    // init hook's attributes; the backend reapplies this app ID every time.
    slint::set_xdg_app_id("start-menu")?;
    let menu = Menu::new(Lifetime::Resident)?;
    let launcher = menu.launcher.clone();
    {
        let launcher = launcher.clone();
        menu.ui.window().on_close_requested(move || {
            trace("close: compositor request");
            launcher.close();
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    trace_rendering(&menu.ui);
    RESIDENT.with(|resident| *resident.borrow_mut() = Some(launcher.clone()));

    let signal_pidfile = pidfile.clone();
    let progress = Arc::new(Progress::default());
    exit_when_stalled(progress.clone(), pidfile.clone(), pid).map_err(|error| {
        slint::PlatformError::Other(format!("cannot start UI watchdog: {error}"))
    })?;
    signals::listen(blocked, move |signal, request| {
        trace(&format!("signal {signal}: {request:?}"));
        if request == Request::Quit {
            // Stop advertising this process at once: a launch racing the
            // shutdown must wait on the lock and become the next resident,
            // not hand off to a process that is about to exit.
            QUITTING.store(true, Ordering::SeqCst);
            let _ = instance::remove_pidfile_if_ours(&signal_pidfile, pid);
            exit_if_stuck(signal_pidfile.clone(), pid);
        }
        let sequence = progress.forwarded();
        let ui_progress = progress.clone();
        let forwarded = slint::invoke_from_event_loop(move || {
            if let Some(launcher) = RESIDENT.with(|resident| resident.borrow().clone()) {
                launcher.handle(request);
            }
            ui_progress.handled(sequence);
        });
        if forwarded.is_err() {
            // The loop has stopped; nothing is left to wait for.
            progress.handled(sequence);
        }
    })
    .map_err(|error| slint::PlatformError::Other(format!("cannot listen for signals: {error}")))?;

    if !hidden {
        launcher.show();
    }
    // Signals sent to this PID are handled from here on.
    if let Err(error) = instance::write_pidfile(&pidfile, pid) {
        eprintln!("start-menu: cannot write {}: {error}", pidfile.display());
        if hidden {
            std::process::exit(1);
        }
        // Nothing can find this process to toggle it: close like a plain launch.
        launcher.lifetime.set(Lifetime::ExitOnClose);
    }
    // A termination request may have raced the write above.
    if QUITTING.load(Ordering::SeqCst) {
        let _ = instance::remove_pidfile_if_ours(&pidfile, pid);
    }
    if hidden {
        // Measuring text loads the system font collection, which otherwise
        // makes the first show after a preload several times slower.
        trace("font warm-up");
        let _ = menu.ui.get_font_warmup();
        trace("font warm-up done");
    }

    let result = slint::run_event_loop_until_quit();
    trace("event loop finished");
    if menu.ui.window().is_visible() {
        let _ = menu.ui.hide();
    }
    if let Err(error) = instance::remove_pidfile_if_ours(&pidfile, pid) {
        eprintln!("start-menu: cannot remove {}: {error}", pidfile.display());
    }
    RESIDENT.with(|resident| resident.borrow_mut().take());
    result
}

const EXIT_GRACE: Duration = Duration::from_secs(2);

/// Longer than an output power-up or display reconfiguration, during which
/// the compositor sends no frame callbacks and a vsync'd swap blocks.
const UI_STALL_LIMIT: Duration = Duration::from_secs(20);

/// Set by the signal thread once SIGTERM/SIGINT arrives.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Requests forwarded to the UI thread and how many of them it has handled.
#[derive(Default)]
struct Progress {
    counts: Mutex<(u64, u64)>,
    changed: Condvar,
}

impl Progress {
    /// Returns the request's sequence number.
    fn forwarded(&self) -> u64 {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.0 += 1;
        self.changed.notify_all();
        counts.0
    }

    fn handled(&self, sequence: u64) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.1 = counts.1.max(sequence);
        self.changed.notify_all();
    }

    /// Return once requests are pending and none was handled for `limit`.
    fn wait_for_stall(&self, limit: Duration) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if counts.1 >= counts.0 {
                counts = self.changed.wait(counts).unwrap_or_else(PoisonError::into_inner);
                continue;
            }
            let handled = counts.1;
            let (guard, wait) = self
                .changed
                .wait_timeout_while(counts, limit, |counts| counts.1 == handled)
                .unwrap_or_else(PoisonError::into_inner);
            if wait.timed_out() {
                return;
            }
            counts = guard;
        }
    }
}

/// A UI thread that stops handling requests (e.g. a frame blocked because the
/// compositor no longer renders the window) would ignore every later toggle.
/// Exit instead, so the next launch replaces this process.
fn exit_when_stalled(progress: Arc<Progress>, pidfile: PathBuf, pid: u32) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("ui-watchdog".into())
        .spawn(move || {
            progress.wait_for_stall(UI_STALL_LIMIT);
            eprintln!("start-menu: no request handled for {UI_STALL_LIMIT:?}; exiting");
            let _ = instance::remove_pidfile_if_ours(&pidfile, pid);
            // SAFETY: terminates immediately without running other threads' code.
            unsafe { libc::_exit(1) }
        })
        .map(drop)
}

/// Termination signals are blocked, so a wedged event loop would otherwise
/// keep the process alive after SIGTERM.
fn exit_if_stuck(pidfile: PathBuf, pid: u32) {
    static ARMED: AtomicBool = AtomicBool::new(false);
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("exit-watchdog".into())
        .spawn(move || {
            std::thread::sleep(EXIT_GRACE);
            eprintln!("start-menu: still running {EXIT_GRACE:?} after termination request");
            let _ = instance::remove_pidfile_if_ours(&pidfile, pid);
            // SAFETY: terminates immediately without running other threads' code.
            unsafe { libc::_exit(1) }
        });
}

// ── Latency trace (SMPL_START_MENU_TRACE=1) ──

static TRACE: AtomicBool = AtomicBool::new(false);

fn init_trace() {
    TRACE.store(
        std::env::var_os("SMPL_START_MENU_TRACE").is_some_and(|value| !value.is_empty() && value != "0"),
        Ordering::Relaxed,
    );
}

/// Prints CLOCK_MONOTONIC seconds, the clock of Python's `time.monotonic()`,
/// so external measurements can be lined up with these events.
fn trace(event: &str) {
    if TRACE.load(Ordering::Relaxed) {
        let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `now` is a valid out-pointer.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
        eprintln!("start-menu: trace {}.{:06} {event}", now.tv_sec, now.tv_nsec / 1000);
    }
}

fn trace_rendering(ui: &MainWindow) {
    if !TRACE.load(Ordering::Relaxed) {
        return;
    }
    let _ = ui.window().set_rendering_notifier(|state, _| match state {
        slint::RenderingState::RenderingSetup => trace("GL context ready"),
        slint::RenderingState::AfterRendering => trace("frame rendered"),
        slint::RenderingState::RenderingTeardown => trace("GL context released"),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app(name: &str, exec: &str, category: &str, search_only: bool) -> AppEntry {
        AppEntry {
            name: name.to_string(),
            exec: exec.to_string(),
            category: category.to_string(),
            icon: String::new(),
            search_only,
        }
    }

    #[test]
    fn pin_quoting_keeps_metadata_preload_order_and_ui_flags() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let data = home.join(".local/share");
        let system = fixture.path().join("system");
        let user_icon = data.join("icons/hicolor/256x256/apps/example.svg");
        let system_icon = system.join("icons/hicolor/scalable/apps/example.svg");
        for path in [&user_icon, &system_icon] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, icons::tests::SVG).unwrap();
        }
        let dirs = std::env::join_paths([&system]).unwrap();
        let resolver = IconResolver::from_paths(
            Some(&home),
            None,
            Some(&dirs),
            &fixture.path().join("flatpak"),
        );
        let mut example = make_app("Example", "\"/home/foo/.local/bin/example\"", "graphics", false);
        example.icon = "example".into();
        let mut unpinned = make_app("Unpinned", "unpinned", "apps", false);
        unpinned.icon = system_icon.to_str().unwrap().into();
        let apps = vec![make_app("Other", "other", "apps", false), example, unpinned];
        let mut pinned = vec![
            "custom \"$HOME\" | another".to_string(),
            "/home/foo/.local/bin/example".to_string(),
            "other".to_string(),
        ];
        let original = pinned.clone();
        let selected = pinned_apps(&apps, &pinned);
        assert_eq!(selected.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(), ["Example", "Other"]);
        let paths = build_icon_path_cache(selected, &resolver);
        assert_eq!(paths.len(), 1, "startup should only preload matching pins");
        assert_eq!(paths.get("example"), Some(&user_icon));
        let images = RefCell::new(HashMap::new());
        let item = to_ui_item(&apps[1], &paths, &images, &pinned);
        assert_eq!(item.name, "Example");
        assert_eq!(item.category_key, "graphics");
        assert_eq!(item.exec, apps[1].exec);
        assert!(item.has_icon);
        assert!(item.is_pinned);
        assert_eq!(pinned, original, "lookup must not migrate or rewrite pin commands");

        commands::toggle_pin(&mut pinned, item.exec.as_str());
        assert!(!to_ui_item(&apps[1], &paths, &images, &pinned).is_pinned);
        assert_eq!(pinned, [original[0].clone(), original[2].clone()]);
        assert_eq!(pinned_apps(&apps, &pinned)[0].name, "Other");
    }

    #[test]
    fn pin_lookup_prefers_exact_matches_and_preserves_shell_distinctions() {
        let apps = vec![
            make_app("Quoted", "'foo' arg", "apps", false),
            make_app("Exact", "foo arg", "apps", false),
            make_app("Literal", "foo '$HOME'", "apps", false),
        ];
        let pins = vec![
            "foo arg".to_string(),
            "foo \"$HOME\"".to_string(),
            "foo different".to_string(),
        ];
        let selected = pinned_apps(&apps, &pins);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "Exact");
    }

    #[test]
    fn missing_and_undecodable_icons_keep_initial_fallback() {
        let fixture = tempfile::tempdir().unwrap();
        let invalid = fixture.path().join("invalid.svg");
        std::fs::write(&invalid, "not an image").unwrap();
        let mut app = make_app("Example", "'example'", "apps", false);
        app.icon = "example".into();
        let images = RefCell::new(HashMap::new());
        let pinned = vec!["example".to_string()];
        for paths in [HashMap::new(), HashMap::from([("example".to_string(), invalid)])] {
            let item = to_ui_item(&app, &paths, &images, &pinned);
            assert!(!item.has_icon);
            assert_eq!(item.initial, "E");
            assert!(item.is_pinned);
            assert!(images.borrow().is_empty());
        }
    }

    // ── parse_app_line: the splitn(5) bug check ──
    // If splitn uses fewer than 5, parts.get(4) always returns None and
    // search_only is always false — all card keywords show in browse.
    fn parse_app_line(line: &str) -> AppEntry {
        let parts: Vec<&str> = line.splitn(5, ';').collect();
        let name = parts[0].to_string();
        let exec = parts.get(1).unwrap_or(&"").to_string();
        let category = parts.get(2).unwrap_or(&"").to_string();
        let icon = parts.get(3).unwrap_or(&"").to_string();
        let search_only = parts.get(4).map(|s| s.trim() == "1").unwrap_or(false);
        AppEntry { name, exec, category, icon, search_only }
    }

    #[test]
    fn search_only_flag_parsed_from_5th_field() {
        let entry = parse_app_line("Bar;smplos-settings taskbar;settings;preferences-system;1");
        assert!(entry.search_only, "5th field '1' must set search_only=true");
    }

    #[test]
    fn normal_entry_not_search_only() {
        let entry = parse_app_line("Taskbar;smplos-settings taskbar;settings;preferences-system");
        assert!(!entry.search_only, "no 5th field means search_only=false");
    }

    #[test]
    fn search_only_hidden_in_browse() {
        let apps = vec![
            make_app("Taskbar", "smplos-settings taskbar", "settings", false),
            make_app("Bar", "smplos-settings taskbar", "settings", true),
            make_app("Workspaces", "smplos-settings taskbar", "settings", true),
        ];
        let visible = filter_apps(&apps, "settings", "");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "Taskbar");
    }

    #[test]
    fn search_only_appears_in_search() {
        let apps = vec![
            make_app("Taskbar", "smplos-settings taskbar", "settings", false),
            make_app("Bar", "smplos-settings taskbar", "settings", true),
            make_app("Workspaces", "smplos-settings taskbar", "settings", true),
        ];
        // "workspac" uniquely matches only "Workspaces" (not "Taskbar" or "Bar")
        let results = filter_apps(&apps, "settings", "workspac");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Workspaces");
        // search_only entry must appear
        assert!(results[0].search_only);
    }

    #[test]
    fn system_utilities_hidden_in_settings_browse() {
        let apps = vec![
            make_app("Bluetooth", "smplos-settings bluetooth", "settings", false),
            make_app("Bluetooth Manager", "blueman-manager", "settings", false),
            make_app("Kvantum Manager", "kvantummanager", "settings", false),
            make_app("Advanced Network Configuration", "nm-connection-editor", "settings", false),
            make_app("Qt5 Settings", "qt5ct", "settings", false),
            make_app("App Center", "toggle-app-center", "settings", false),
            make_app("Web App Center", "webapp-center", "settings", false),
        ];
        let visible = filter_apps(&apps, "settings", "");
        let names: Vec<&str> = visible.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Bluetooth"), "smplos-settings entries must show");
        assert!(names.contains(&"App Center"), "toggle-app-center must show");
        assert!(names.contains(&"Web App Center"), "webapp-center must show");
        assert!(!names.contains(&"Bluetooth Manager"), "blueman-manager must be hidden");
        assert!(!names.contains(&"Kvantum Manager"), "kvantummanager must be hidden");
        assert!(!names.contains(&"Advanced Network Configuration"), "nm-connection-editor must be hidden");
        assert!(!names.contains(&"Qt5 Settings"), "qt5ct must be hidden");
    }

    // ── Frecency-based search ranking ──

    fn app(name: &str, exec: &str) -> AppEntry {
        AppEntry {
            name: name.to_string(),
            exec: exec.to_string(),
            category: "apps".to_string(),
            icon: String::new(),
            search_only: false,
        }
    }

    #[test]
    fn search_match_rank_prefers_word_starts() {
        // Plain alphabetical when no usage data: with fuzzy ranking,
        // prefix/exact matches still beat substring matches because nucleo
        // gives a higher score to contiguous prefix runs and word-starts.
        let apps = vec![
            app("barcoder", "barcoder"),
            app("VS Code",  "code"),
            app("Code OSS", "code-oss"),
            app("Code",     "codebin"),
        ];
        let usage = Usage::default();
        let out = filter_and_rank(&apps, "apps", "code", &usage, 1_000_000);
        let names: Vec<&str> = out.iter().map(|a| a.name.as_str()).collect();
        // "Code" must be first (exact match), all 4 must be present.
        assert_eq!(names[0], "Code");
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn fuzzy_matches_acronym_for_vscode() {
        // Typing "vsc" should find "Visual Studio Code" via subsequence match
        // (v…s…c…) — old substring filter would have missed it entirely.
        let apps = vec![
            app("Visual Studio Code", "code"),
            app("Firefox",            "firefox"),
            app("Calculator",         "gnome-calculator"),
        ];
        let out = filter_and_rank(&apps, "apps", "vsc", &Usage::default(), 0);
        let names: Vec<&str> = out.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Visual Studio Code"),
                "fuzzy match must find Visual Studio Code via 'vsc' acronym; got {:?}", names);
    }

    #[test]
    fn fuzzy_matches_transposition_typo() {
        // "pwoer" is one adjacent transposition away from "power".  Nucleo's
        // subsequence match rejects it (no 'r' after the last 'e'), so this
        // exercises the Damerau-Levenshtein fallback in fuzzy_score.
        let apps = vec![
            app("Power Profile",  "settings --tab power"),
            app("Screen Off",     "settings --tab power"),
            app("Firefox",        "firefox"),
        ];
        let out = filter_and_rank(&apps, "apps", "pwoer", &Usage::default(), 0);
        let names: Vec<&str> = out.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Power Profile"),
                "typo 'pwoer' must still surface Power Profile; got {:?}", names);
        assert!(!names.contains(&"Firefox"),
                "typo fallback must not match unrelated apps; got {:?}", names);
    }

    #[test]
    fn fuzzy_drops_non_matches() {
        // "xyz" matches none of these names — result must be empty, NOT show
        // unrelated apps at the bottom.
        let apps = vec![
            app("Firefox",    "firefox"),
            app("Calculator", "gnome-calculator"),
        ];
        let out = filter_and_rank(&apps, "apps", "xyz", &Usage::default(), 0);
        assert!(out.is_empty(), "non-matching query must return empty list; got {:?}",
                out.iter().map(|a| a.name.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn frecency_still_wins_over_fuzzy_quality() {
        // "Code" gives a higher raw fuzzy score for query "code" than "VS Code",
        // but heavy VS Code usage must still float it to the top — that's the
        // whole point of frecency taking precedence.
        let apps = vec![
            app("Code",    "codebin"),
            app("VS Code", "code"),
        ];
        let now = 10_000_000_u64;
        let mut usage = Usage::default();
        for _ in 0..15 { usage.record("code", now); }
        let out = filter_and_rank(&apps, "apps", "code", &usage, now);
        assert_eq!(out[0].exec, "code", "high-frecency app must win even when raw fuzzy score is lower");
    }

    #[test]
    fn most_used_app_floats_to_top_in_search() {
        // User keeps picking VS Code when they type "code" — it should win
        // over the alphabetically-better "Code" entry once usage diverges.
        let apps = vec![
            app("Code",    "codebin"),
            app("VS Code", "code"),
            app("xcode",   "xcode"),
        ];
        let now = 10_000_000_u64;
        let mut usage = Usage::default();
        for _ in 0..8 { usage.record("code", now); }      // launched 8 times today
        usage.record("codebin", now);                      // once
        let out = filter_and_rank(&apps, "apps", "code", &usage, now);
        assert_eq!(out[0].exec, "code", "high-frecency match must win");
    }

    #[test]
    fn frecency_decays_over_time() {
        // App launched 5 times a year ago must lose to one launched twice today.
        let apps = vec![
            app("Old App", "old"),
            app("New App", "new"),
        ];
        let now = 100_000_000_u64;
        let year_ago = now - (365 * 86_400);
        let mut usage = Usage::default();
        for _ in 0..5 { usage.record("old", year_ago); }
        for _ in 0..2 { usage.record("new", now); }
        let out = filter_and_rank(&apps, "apps", "app", &usage, now);
        assert_eq!(out[0].exec, "new");
    }

    #[test]
    fn unused_apps_still_sort_predictably_by_match_quality() {
        // No usage history at all: fuzzy ranking puts prefix/contiguous
        // matches above scattered-character subsequence matches.
        let apps = vec![
            app("xcoder", "xcoder"),
            app("Codium", "codium"),
        ];
        let out = filter_and_rank(&apps, "apps", "cod", &Usage::default(), 0);
        assert_eq!(out[0].name, "Codium",
                   "prefix match 'Codium' must beat embedded substring 'xcoder'");
    }

    // ── Resident mode ──

    #[test]
    fn version_probe_prints_the_legacy_line_then_the_resident_feature() {
        let text = version_text();
        assert_eq!(
            text,
            format!("start-menu v{}\nfeatures: resident\n", env!("CARGO_PKG_VERSION"))
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "exactly two lines: {text:?}");
        // The first line is unchanged from releases without resident mode.
        let version = lines[0].strip_prefix("start-menu v").unwrap();
        assert_eq!(version.split('.').count(), 3);
        assert!(version.split('.').all(|part| part.parse::<u32>().is_ok()));
        assert_eq!(lines[1], "features: resident");
    }

    #[test]
    fn only_resident_flag_changes_the_legacy_launch() {
        let parse = |args: &[&str]| parse_invocation(args.iter().map(OsString::from));
        assert_eq!(parse(&[]), Invocation::Legacy);
        assert_eq!(parse(&["--hidden"]), Invocation::Legacy);
        assert_eq!(parse(&["--unknown"]), Invocation::Legacy);
        assert_eq!(parse(&["--resident"]), Invocation::Resident { hidden: false });
        assert_eq!(parse(&["--resident", "--hidden"]), Invocation::Resident { hidden: true });
        assert_eq!(parse(&["--hidden", "--resident"]), Invocation::Resident { hidden: true });
        for version in [&["-v"][..], &["--version"], &["--resident", "--version"], &["-v", "--resident"]] {
            assert_eq!(parse(version), Invocation::Version, "{version:?}");
        }
    }

    #[derive(Default)]
    struct RecordedView {
        search: RefCell<String>,
        category: Cell<i32>,
        apps: RefCell<Option<ModelRc<AppItem>>>,
        app_count: Cell<i32>,
        selected: Cell<i32>,
        searching: Cell<bool>,
        context_menu: Cell<bool>,
        power_menu: Cell<bool>,
        scrolled: Cell<bool>,
        input_held: Cell<bool>,
        focus_zone: Cell<i32>,
    }

    impl MenuView for RecordedView {
        fn set_search_text(&self, text: SharedString) {
            *self.search.borrow_mut() = text.into();
        }
        fn set_active_category(&self, index: i32) {
            self.category.set(index);
        }
        fn set_apps(&self, apps: ModelRc<AppItem>) {
            *self.apps.borrow_mut() = Some(apps);
        }
        fn set_app_count(&self, count: i32) {
            self.app_count.set(count);
        }
        fn set_selected_app(&self, index: i32) {
            self.selected.set(index);
        }
        fn set_is_searching(&self, searching: bool) {
            self.searching.set(searching);
        }
        fn set_show_context_menu(&self, shown: bool) {
            self.context_menu.set(shown);
        }
        fn set_show_power_menu(&self, shown: bool) {
            self.power_menu.set(shown);
        }
        fn invoke_reset_scroll(&self) {
            self.scrolled.set(false);
        }
        fn release_input(&self) {
            self.input_held.set(false);
        }
        fn invoke_focus_search(&self) {
            self.focus_zone.set(0);
        }
    }

    #[test]
    fn hiding_resets_the_view_to_its_startup_state() {
        let items = [app("Firefox", "firefox"), app("Files", "nemo")]
            .iter()
            .map(|entry| to_ui_item(entry, &HashMap::new(), &RefCell::new(HashMap::new()), &[]))
            .collect::<Vec<_>>();
        let model = Rc::new(VecModel::from(items));
        let view = RecordedView {
            search: RefCell::new("fi".into()),
            category: Cell::new(3),
            apps: RefCell::new(Some(ModelRc::from(model.clone()))),
            app_count: Cell::new(2),
            selected: Cell::new(1),
            searching: Cell::new(true),
            context_menu: Cell::new(true),
            power_menu: Cell::new(true),
            scrolled: Cell::new(true),
            input_held: Cell::new(true),
            focus_zone: Cell::new(2),
        };

        reset_view(&view, &model);

        assert_eq!(view.search.borrow().as_str(), "");
        assert_eq!(view.category.get(), -1, "no category selected");
        assert_eq!(model.row_count(), 0, "app list empty as at startup");
        let bound = view.apps.borrow().clone().unwrap();
        assert_eq!(bound.row_count(), 0);
        assert_eq!(view.app_count.get(), 0);
        assert_eq!(view.selected.get(), -1);
        assert!(!view.searching.get());
        assert!(!view.context_menu.get(), "context menu closed");
        assert!(!view.power_menu.get(), "power menu closed");
        assert!(!view.scrolled.get(), "list scrolled to the top");
        assert!(!view.input_held.get(), "stale hover and modifiers cleared");
        assert_eq!(view.focus_zone.get(), 0, "typing goes to the search field");

        // The window keeps showing the shared model, so later searches appear.
        model.push(to_ui_item(&app("Gimp", "gimp"), &HashMap::new(), &RefCell::new(HashMap::new()), &[]));
        assert_eq!(bound.row_count(), 1);

        // Resetting an already reset view changes nothing.
        model.set_vec(Vec::new());
        reset_view(&view, &model);
        assert_eq!(view.category.get(), -1);
        assert_eq!(view.search.borrow().as_str(), "");
    }

    #[test]
    fn hidden_reset_matches_the_declared_startup_state() {
        let ui = include_str!("../ui/main.slint");
        for declaration in [
            "in-out property <int> active-category: -1;",
            "in-out property <int> app-count: 0;",
            "in-out property <bool> is-searching: false;",
            "in-out property <int> selected-app: -1;",
            "in-out property <bool> show-context-menu: false;",
            "in-out property <bool> show-power-menu: false;",
            "in-out property <string> search-text <=> search-input.text;",
        ] {
            assert!(ui.contains(declaration), "missing startup default: {declaration}");
        }
        let reset_scroll = ui
            .split("public function reset-scroll() {")
            .nth(1)
            .expect("reset-scroll function")
            .split('}')
            .next()
            .unwrap();
        assert!(reset_scroll.contains("app-flick.viewport-x = 0;"));
        assert!(reset_scroll.contains("app-flick.viewport-y = 0;"));
        assert!(ui.contains("app-flick := Flickable {"));
    }

    #[test]
    fn hidden_preload_measures_text_without_drawing_it() {
        let ui = include_str!("../ui/main.slint");
        assert!(ui.contains("out property <length> font-warmup: font-warmup-text.preferred-width;"));
        let text = ui
            .split("font-warmup-text := Text {")
            .nth(1)
            .expect("font warm-up text")
            .split("\n    }")
            .next()
            .unwrap();
        assert!(text.contains("visible: false;"));
        // Plain text plus an icon glyph, so the icon-font fallback loads too.
        assert!(text.contains("\\u{f0ac3}"));
    }

    #[test]
    fn app_index_is_read_with_the_stamp_taken_before_reading() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("app_index");
        assert!(read_app_index(&path, false).is_none(), "missing index keeps the old list");

        std::fs::write(
            &path,
            "Firefox;firefox;internet;firefox\n\
             firefox;firefox-dup;internet\n\
             broken line\n\
             Display;smplos-settings display;settings;preferences;1\n",
        )
        .unwrap();
        let (apps, stamp) = read_app_index(&path, false).unwrap();
        assert_eq!(stamp, FileStamp::of(&path));
        let names: Vec<&str> = apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(names, ["Firefox", "Display"], "deduplicated by lowercase name");
        assert!(apps[1].search_only);
        assert_eq!(apps[0].icon, "firefox");

        std::fs::write(&path, "Gimp;gimp;graphics;gimp\n").unwrap();
        assert!(stamp::should_reload(stamp, FileStamp::of(&path), false));
    }

    #[test]
    fn exited_launches_are_reaped_and_running_ones_kept() {
        let wait_until = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !done() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let children = Children::default();
        let exited = std::process::Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let exited_pid = exited.id();
        children.adopt(exited);
        children.adopt(std::process::Command::new("sleep").arg("30").spawn().unwrap());

        wait_until(&|| children.reap() == 1);
        assert_eq!(children.reap(), 1, "the running launch is kept");
        assert!(
            !Path::new(&format!("/proc/{exited_pid}")).exists(),
            "an exited launch must not stay a zombie"
        );

        for child in children.0.borrow_mut().iter_mut() {
            child.kill().unwrap();
        }
        wait_until(&|| children.reap() == 0);
        assert_eq!(children.reap(), 0);
    }

    #[test]
    fn ui_stall_is_detected_only_while_requests_wait() {
        let limit = Duration::from_millis(200);
        let progress = Arc::new(Progress::default());
        let (stalled, detected) = std::sync::mpsc::channel();
        let watcher = {
            let progress = progress.clone();
            std::thread::spawn(move || {
                progress.wait_for_stall(limit);
                stalled.send(std::time::Instant::now()).unwrap();
            })
        };

        // Idle (nothing pending) is never a stall.
        assert!(detected.recv_timeout(limit * 2).is_err());

        // Requests handled one by one keep resetting the limit, even when
        // the queue as a whole waits longer than the limit.
        let first = progress.forwarded();
        let second = progress.forwarded();
        std::thread::sleep(limit * 3 / 4);
        progress.handled(first);
        std::thread::sleep(limit * 3 / 4);
        progress.handled(second);
        assert!(detected.recv_timeout(limit).is_err());

        // A request the UI thread never handles is a stall.
        let started = std::time::Instant::now();
        progress.forwarded();
        let at = detected.recv_timeout(Duration::from_secs(5)).expect("stall detected");
        assert!(at.duration_since(started) >= limit);
        watcher.join().unwrap();
    }
}
