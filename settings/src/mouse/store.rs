//! Reading and writing the Mouse tab's files.
//!
//! A write goes to a temporary file beside the target and is renamed over
//! it, so a reader (Hyprland, a daemon) never sees half a file. When asked,
//! the previous content is kept in a `backups` folder next to the file.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Previous versions kept per file.
const BACKUPS_KEPT: usize = 10;

/// Whether `save` keeps the content it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backups {
    Keep,
    Skip,
}

/// The file's text; `None` when it doesn't exist yet.
pub fn read(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error("read", path, e)),
    }
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> String {
    format!("couldn't {action} {}: {error}", path.display())
}

/// The file a write lands in: a symlink's target, so a linked file (a
/// dotfiles checkout) stays a link.
fn resolve(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

/// Removes the temporary file if the write fails half way.
struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Replaces `path` with `text`.
///
/// `expected` is what Settings last read (`None`: no file). If the file is
/// different now, nothing is written, so an edit made by hand or by another
/// program is not overwritten. Writing the text the file already has does
/// nothing.
pub fn save(path: &Path, text: &str, expected: Option<&str>, backups: Backups) -> Result<(), String> {
    write(path, text, expected, backups).map_err(|error| {
        if error.starts_with("Not saved") {
            error
        } else {
            format!("Not saved: {error}")
        }
    })
}

fn write(path: &Path, text: &str, expected: Option<&str>, backups: Backups) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let target = resolve(path);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{} is not a file path", path.display()))?;
    let current = read(&target)?;
    if current.as_deref() != expected {
        return Err(format!("Not saved: {name} changed on disk since Settings read it."));
    }
    if current.as_deref() == Some(text) {
        return Ok(());
    }
    let dir = target.parent().ok_or_else(|| format!("{} has no directory", target.display()))?;
    fs::create_dir_all(dir).map_err(|e| io_error("create", dir, e))?;
    if let (Some(old), Backups::Keep) = (&current, backups) {
        back_up(dir, &name, old)?;
    }
    let temporary = Temporary(dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temporary.0)
        .map_err(|e| io_error("create", &temporary.0, e))?;
    if current.is_some() {
        if let Ok(meta) = fs::metadata(&target) {
            let _ = file.set_permissions(meta.permissions());
        }
    }
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| io_error("write", &temporary.0, e))?;
    drop(file);
    fs::rename(&temporary.0, &target).map_err(|e| io_error("replace", &target, e))?;
    if let Ok(directory) = fs::File::open(dir) {
        let _ = directory.sync_all();
    }
    if backups == Backups::Keep {
        prune_backups(dir, &name);
    }
    Ok(())
}

/// `mouse.json` -> (`mouse`, `.json`).
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// `<stem>-YYYYMMDD-HHMMSS-NNN<ext>`: the names `back_up` makes, and only those.
/// Returns the part between the stem and the extension.
fn backup_tail<'a>(file: &'a str, stem: &str, ext: &str) -> Option<&'a str> {
    let tail = file.strip_prefix(stem)?.strip_prefix('-')?.strip_suffix(ext)?;
    let digits = |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let mut parts = tail.split('-');
    let shaped = matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(date), Some(time), Some(number), None) if digits(date, 8) && digits(time, 6) && digits(number, 3)
    );
    shaped.then_some(tail)
}

/// Copies the old content to `backups/<name>-<time>-<n>.<ext>` beside the file.
fn back_up(dir: &Path, name: &str, old: &str) -> Result<(), String> {
    let backups = dir.join("backups");
    fs::create_dir_all(&backups).map_err(|e| io_error("create", &backups, e))?;
    let (stem, ext) = split_name(name);
    let stamp = crate::display::conf::local_stamp();
    let head = format!("{stem}-{stamp}-");
    // Count on from the newest copy of this second rather than reusing a gap
    // that pruning left, so names always sort oldest to newest.
    let next = fs::read_dir(&backups)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            backup_tail(&file, stem, ext)?;
            file.strip_prefix(&head)?.strip_suffix(ext)?.parse::<u32>().ok()
        })
        .max()
        .map_or(0, |highest| highest + 1);
    if next > 999 {
        return Err("Not saved: too many backups in one second".to_string());
    }
    let target = backups.join(format!("{head}{next:03}{ext}"));
    fs::write(&target, old).map_err(|e| format!("Not saved: couldn't back up {name} ({e})"))
}

/// Keeps the newest backups of this file; other files are never touched.
fn prune_backups(dir: &Path, name: &str) {
    let (stem, ext) = split_name(name);
    let Ok(entries) = fs::read_dir(dir.join("backups")) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| backup_tail(n, stem, ext).is_some()))
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(BACKUPS_KEPT);
    for old in files.into_iter().take(excess) {
        let _ = fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mouse::testing::temp_dir;

    fn backup_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.join("backups"))
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn a_missing_file_reads_as_none_and_is_created_with_its_folder() {
        let dir = temp_dir("store-new");
        let path = dir.join("smplos/mouse.json");
        assert_eq!(read(&path), Ok(None));

        save(&path, "{}\n", None, Backups::Keep).unwrap();
        assert_eq!(read(&path), Ok(Some("{}\n".to_string())));
        assert!(backup_names(&dir.join("smplos")).is_empty(), "nothing to back up the first time");
        let leftovers: Vec<_> = fs::read_dir(dir.join("smplos")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers, ["mouse.json"], "no temporary file is left behind");
    }

    #[test]
    fn replacing_a_file_keeps_what_it_held_in_backups() {
        let dir = temp_dir("store-backup");
        let path = dir.join("mouse.json");
        save(&path, "one", None, Backups::Keep).unwrap();
        save(&path, "two", Some("one"), Backups::Keep).unwrap();
        assert_eq!(read(&path), Ok(Some("two".to_string())));
        let names = backup_names(&dir);
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("mouse-") && names[0].ends_with(".json"), "{names:?}");
        assert_eq!(fs::read_to_string(dir.join("backups").join(&names[0])).unwrap(), "one");
    }

    #[test]
    fn a_file_changed_behind_settings_back_is_not_overwritten() {
        let dir = temp_dir("store-changed");
        let path = dir.join("mouse.json");
        save(&path, "mine", None, Backups::Keep).unwrap();
        fs::write(&path, "edited by hand").unwrap();

        let err = save(&path, "theirs", Some("mine"), Backups::Keep).unwrap_err();
        assert!(err.contains("changed on disk"), "{err}");
        assert_eq!(read(&path), Ok(Some("edited by hand".to_string())));

        let created = save(&dir.join("other.json"), "x", Some("stale"), Backups::Keep);
        assert!(created.is_err(), "a file that should exist but doesn't is refused too");
        let appeared = save(&path, "x", None, Backups::Keep);
        assert!(appeared.is_err(), "a file that appeared since is refused");
    }

    #[test]
    fn writing_the_same_text_changes_nothing() {
        let dir = temp_dir("store-same");
        let path = dir.join("mouse.json");
        save(&path, "same", None, Backups::Keep).unwrap();
        save(&path, "same", Some("same"), Backups::Keep).unwrap();
        assert!(backup_names(&dir).is_empty());
    }

    #[test]
    fn only_the_newest_backups_are_kept_and_other_files_are_left_alone() {
        let dir = temp_dir("store-prune");
        let path = dir.join("mouse.json");
        fs::create_dir_all(dir.join("backups")).unwrap();
        fs::write(dir.join("backups/notes.txt"), "mine").unwrap();
        fs::write(dir.join("backups/other-20200101-000000-000.json"), "mine").unwrap();
        fs::write(dir.join("backups/mouse-extra.json"), "mine").unwrap();

        let mut previous = None;
        for n in 0..14 {
            let text = format!("version {n}");
            save(&path, &text, previous.as_deref(), Backups::Keep).unwrap();
            previous = Some(text);
        }
        let ours: Vec<String> = backup_names(&dir).into_iter().filter(|n| backup_tail(n, "mouse", ".json").is_some()).collect();
        assert_eq!(ours.len(), BACKUPS_KEPT);
        assert_eq!(fs::read_to_string(dir.join("backups").join(ours.last().unwrap())).unwrap(), "version 12", "the newest previous version survives");
        assert_eq!(fs::read_to_string(dir.join("backups").join(&ours[0])).unwrap(), "version 3", "and the oldest kept one is the tenth newest");
        assert!(dir.join("backups/notes.txt").exists());
        assert!(dir.join("backups/other-20200101-000000-000.json").exists());
        assert!(dir.join("backups/mouse-extra.json").exists(), "a name that only looks like ours is not ours");
    }

    #[test]
    fn a_file_that_needs_no_history_leaves_no_backups_folder() {
        let dir = temp_dir("store-skip");
        let path = dir.join("pointer.conf");
        save(&path, "a", None, Backups::Skip).unwrap();
        save(&path, "b", Some("a"), Backups::Skip).unwrap();
        assert_eq!(read(&path), Ok(Some("b".to_string())));
        assert!(!dir.join("backups").exists());
    }

    #[test]
    fn a_linked_file_stays_a_link_and_the_target_is_updated() {
        let dir = temp_dir("store-link");
        let dotfiles = dir.join("dotfiles");
        let live = dir.join("live");
        fs::create_dir_all(&dotfiles).unwrap();
        fs::create_dir_all(&live).unwrap();
        fs::write(dotfiles.join("mouse.json"), "old").unwrap();
        std::os::unix::fs::symlink(dotfiles.join("mouse.json"), live.join("mouse.json")).unwrap();

        save(&live.join("mouse.json"), "new", Some("old"), Backups::Keep).unwrap();
        assert!(fs::symlink_metadata(live.join("mouse.json")).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(dotfiles.join("mouse.json")).unwrap(), "new");
        assert_eq!(backup_names(&dotfiles).len(), 1, "the backup sits beside the real file");
    }

    #[test]
    fn a_failed_save_always_says_it_did_not_save() {
        let dir = temp_dir("store-fail");
        fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        let err = save(&dir.join("blocker/mouse.json"), "x", None, Backups::Keep).unwrap_err();
        assert!(err.starts_with("Not saved: couldn't"), "{err}");

        let err = save(&dir.join("mouse.json"), "x", Some("stale"), Backups::Keep).unwrap_err();
        assert!(err.starts_with("Not saved: mouse.json changed"), "{err}");
        assert_eq!(err.matches("Not saved").count(), 1, "{err}");
    }

    #[test]
    fn a_file_name_splits_into_a_stem_and_an_extension() {
        assert_eq!(split_name("mouse.json"), ("mouse", ".json"));
        assert_eq!(split_name("pointer.conf"), ("pointer", ".conf"));
        assert_eq!(split_name("plain"), ("plain", ""));
        assert_eq!(split_name(".hidden"), (".hidden", ""));
    }
}
