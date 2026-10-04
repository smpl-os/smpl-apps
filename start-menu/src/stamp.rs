//! Change detection for the files a resident menu re-reads when shown.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Identity and modification state of a file. Atomic replacement changes the
/// inode even when size and modification time are preserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    device: u64,
    inode: u64,
    len: u64,
    modified_sec: i64,
    modified_nsec: i64,
}

impl FileStamp {
    /// `None` when the file is missing or cannot be inspected.
    pub fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            len: metadata.len(),
            modified_sec: metadata.mtime(),
            modified_nsec: metadata.mtime_nsec(),
        })
    }
}

/// Whether a source loaded at `loaded` must be read again now that it is at
/// `current`. A source whose absence is meaningful (no pins, no usage yet) is
/// reloaded when it disappears; otherwise the last good copy is kept.
pub fn should_reload(
    loaded: Option<FileStamp>,
    current: Option<FileStamp>,
    reload_when_missing: bool,
) -> bool {
    if current.is_none() && !reload_when_missing {
        return false;
    }
    loaded != current
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::time::{Duration, SystemTime};

    fn set_mtime(path: &Path, time: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn stamps_change_with_size_mtime_and_replacement_only() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("app_index");
        assert_eq!(FileStamp::of(&path), None);

        let fixed = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::write(&path, "Firefox;firefox;internet;firefox\n").unwrap();
        set_mtime(&path, fixed);
        let original = FileStamp::of(&path).unwrap();
        assert_eq!(FileStamp::of(&path), Some(original), "unchanged file");

        // Reading or rewriting identical metadata is not a change.
        let _ = fs::read_to_string(&path).unwrap();
        assert_eq!(FileStamp::of(&path), Some(original));

        // Same size, new mtime.
        fs::write(&path, "Firefox;firefox;internet;FIREFOX\n").unwrap();
        set_mtime(&path, fixed + Duration::from_secs(1));
        assert_ne!(FileStamp::of(&path), Some(original));

        // New size, same mtime.
        fs::write(&path, "Firefox;firefox;internet;firefox\nGimp;gimp;graphics;gimp\n").unwrap();
        set_mtime(&path, fixed);
        let grown = FileStamp::of(&path).unwrap();
        assert_ne!(grown, original);

        // Atomic replacement with identical size and mtime (new inode).
        let replacement = fixture.path().join("app_index.tmp");
        fs::write(&replacement, fs::read(&path).unwrap()).unwrap();
        set_mtime(&replacement, fixed);
        fs::rename(&replacement, &path).unwrap();
        assert_ne!(FileStamp::of(&path), Some(grown));
    }

    #[test]
    fn reload_decision_keeps_or_clears_missing_sources() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("source");
        fs::write(&path, "a").unwrap();
        let a = FileStamp::of(&path);
        fs::write(&path, "ab").unwrap();
        let b = FileStamp::of(&path);
        assert!(a.is_some() && b.is_some() && a != b);

        for reload_when_missing in [false, true] {
            assert!(!should_reload(a, a, reload_when_missing), "unchanged");
            assert!(should_reload(a, b, reload_when_missing), "changed");
            assert!(should_reload(None, a, reload_when_missing), "appeared");
            assert!(!should_reload(None, None, reload_when_missing), "still missing");
        }
        // The app index keeps its last good copy while it is being rebuilt;
        // removing the pin or usage file means "none" and is applied.
        assert!(!should_reload(a, None, false));
        assert!(should_reload(a, None, true));
    }
}
