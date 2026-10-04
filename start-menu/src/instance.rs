//! Single-instance bookkeeping for `start-menu --resident`.
//!
//! The pidfile is the contract with the smplOS scripts: it names the resident
//! process once that process can receive signals. A neighbouring `.lock` file
//! serialises startup, so launches racing at login cannot both become resident.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `/proc/<pid>/comm` of a menu process.
pub const PROCESS_NAME: &str = "start-menu";

/// How long a new launch waits for a starting resident to publish its pidfile.
const STARTUP_PATIENCE: Duration = Duration::from_secs(5);

pub enum Startup {
    /// This process is the resident. Keep the lock (when it could be taken)
    /// open for the whole process lifetime.
    Resident(Option<File>),
    /// Another resident runs with this PID; it was asked to show if requested.
    HandedOff(u32),
}

/// `${SMPL_START_MENU_PIDFILE:-${XDG_RUNTIME_DIR:-/run/user/<uid>}/smplos/start-menu.pid}`
pub fn pidfile_path() -> PathBuf {
    pidfile_path_from(
        std::env::var_os("SMPL_START_MENU_PIDFILE"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() },
    )
}

fn pidfile_path_from(
    explicit: Option<OsString>,
    runtime_dir: Option<OsString>,
    uid: u32,
) -> PathBuf {
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    runtime_dir
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")))
        .join("smplos/start-menu.pid")
}

fn lock_path(pidfile: &Path) -> PathBuf {
    let mut name = pidfile
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("start-menu.pid"));
    name.push(".lock");
    pidfile.with_file_name(name)
}

/// Create the pidfile's directory with mode 0700 if it is missing.
fn ensure_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
        }
        _ => Ok(()),
    }
}

/// Become the resident, or hand off to the one that is already running.
pub fn start(pidfile: &Path, own_pid: u32, show_existing: bool) -> io::Result<Startup> {
    // A resident found between our check and the signal may exit; look again.
    for _ in 0..3 {
        match claim(pidfile, Path::new("/proc"), own_pid, STARTUP_PATIENCE)? {
            Claim::Primary(lock) => return Ok(Startup::Resident(lock)),
            Claim::Existing(pid) if !show_existing => return Ok(Startup::HandedOff(pid)),
            Claim::Existing(pid) => match send_signal(pid, crate::signals::show_signal()) {
                Ok(()) => return Ok(Startup::HandedOff(pid)),
                Err(error) if error.raw_os_error() == Some(libc::ESRCH) => continue,
                Err(error) => return Err(error),
            },
        }
    }
    Err(io::Error::other("the running start-menu kept exiting during handoff"))
}

enum Claim {
    Primary(Option<File>),
    Existing(u32),
}

fn claim(pidfile: &Path, proc_root: &Path, own_pid: u32, patience: Duration) -> io::Result<Claim> {
    let lock = lock_path(pidfile);
    let mut directory_error = ensure_parent(&lock).err();
    let deadline = Instant::now() + patience;
    loop {
        if let Some(pid) = running_instance(pidfile, proc_root, own_pid) {
            return Ok(Claim::Existing(pid));
        }
        let locked = match directory_error.take() {
            Some(error) => Err(error),
            None => try_lock(&lock),
        };
        match locked {
            Ok(Some(file)) => return Ok(Claim::Primary(Some(file))),
            // Another resident is starting; wait for its pidfile or its exit.
            Ok(None) => {}
            Err(error) => {
                eprintln!(
                    "start-menu: cannot use instance lock {}: {error}",
                    lock.display()
                );
                return Ok(Claim::Primary(None));
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "another start-menu holds {} but did not write {}",
                    lock.display(),
                    pidfile.display()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn try_lock(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("instance lock is not a regular file"));
    }
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(error)
    }
}

fn send_signal(pid: u32, signal: libc::c_int) -> io::Result<()> {
    // Never 0 or negative: those address process groups.
    let pid = libc::pid_t::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: plain syscall on a validated positive PID.
    if unsafe { libc::kill(pid, signal) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The PID of a live resident named by `pidfile`, other than `own_pid`.
fn running_instance(pidfile: &Path, proc_root: &Path, own_pid: u32) -> Option<u32> {
    let pid = read_pid(pidfile)?;
    (pid != own_pid && is_live_menu(proc_root, pid)).then_some(pid)
}

fn is_live_menu(proc_root: &Path, pid: u32) -> bool {
    let process = proc_root.join(pid.to_string());
    let is_menu = fs::read_to_string(process.join("comm"))
        .is_ok_and(|comm| comm.trim_end_matches('\n') == PROCESS_NAME);
    // Zombies keep their comm until they are reaped.
    is_menu && fs::read_to_string(process.join("stat")).is_ok_and(|stat| !is_dead(&stat))
}

fn is_dead(stat: &str) -> bool {
    stat.rsplit_once(')')
        .and_then(|(_, fields)| fields.trim_start().chars().next())
        .is_none_or(|state| matches!(state, 'Z' | 'X' | 'x'))
}

pub fn read_pid(path: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(path).ok()?.take(64).read_to_string(&mut text).ok()?;
    parse_pid(&text)
}

fn parse_pid(text: &str) -> Option<u32> {
    let digits = text.trim();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let pid = digits.parse::<libc::pid_t>().ok().filter(|pid| *pid > 0)?;
    u32::try_from(pid).ok()
}

/// Atomically publish `pid` (temp file + rename in the same directory).
pub fn write_pidfile(path: &Path, pid: u32) -> io::Result<()> {
    ensure_parent(path)?;
    let mut name = OsString::from(".");
    name.push(path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "pidfile path has no file name")
    })?);
    name.push(format!(".{pid}.tmp"));
    let temp = path.with_file_name(name);
    let result = write_new(&temp, pid).and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn write_new(temp: &Path, pid: u32) -> io::Result<()> {
    let open = || {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(temp)
    };
    let mut file = match open() {
        // Left behind by an earlier process that had our PID and crashed.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(temp)?;
            open()?
        }
        other => other?,
    };
    file.write_all(format!("{pid}\n").as_bytes())
}

/// Remove the pidfile only while it still names `pid`.
pub fn remove_pidfile_if_ours(path: &Path, pid: u32) -> io::Result<bool> {
    if read_pid(path) != Some(pid) {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_process(proc_root: &Path, pid: u32, comm: &str, state: char) {
        let dir = proc_root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(dir.join("stat"), format!("{pid} ({comm}) {state} 1 {pid} {pid} 0 -1")).unwrap();
    }

    #[test]
    fn process_state_is_read_after_the_last_parenthesis() {
        assert!(!is_dead("600 (start-menu) S 1 600"));
        assert!(!is_dead("600 (odd) name) R 1 600"));
        assert!(is_dead("600 (odd) S name) Z 1 600"));
        assert!(is_dead("600 (start-menu) X 1"));
        assert!(is_dead("garbage"));
    }

    #[test]
    fn pidfile_path_prefers_override_then_runtime_dir_then_uid() {
        assert_eq!(
            pidfile_path_from(Some("/tmp/x/menu.pid".into()), Some("/run/user/7".into()), 7),
            PathBuf::from("/tmp/x/menu.pid")
        );
        for empty in [None, Some(OsString::new())] {
            assert_eq!(
                pidfile_path_from(empty.clone(), Some("/xdg".into()), 7),
                PathBuf::from("/xdg/smplos/start-menu.pid")
            );
            assert_eq!(
                pidfile_path_from(empty.clone(), Some(OsString::new()), 7),
                PathBuf::from("/run/user/7/smplos/start-menu.pid")
            );
            assert_eq!(
                pidfile_path_from(empty, None, 1000),
                PathBuf::from("/run/user/1000/smplos/start-menu.pid")
            );
        }
        assert_eq!(
            lock_path(Path::new("/run/user/7/smplos/start-menu.pid")),
            PathBuf::from("/run/user/7/smplos/start-menu.pid.lock")
        );
    }

    #[test]
    fn pid_parsing_rejects_values_that_could_signal_groups() {
        assert_eq!(parse_pid("1234\n"), Some(1234));
        assert_eq!(parse_pid("  42  "), Some(42));
        for invalid in ["", "0", "-1", "+5", "12a", "1 2", "99999999999", "2147483648"] {
            assert_eq!(parse_pid(invalid), None, "{invalid:?}");
        }
        assert_eq!(parse_pid("2147483647"), Some(i32::MAX as u32));
    }

    #[test]
    fn pidfile_is_written_atomically_in_a_private_directory() {
        let fixture = tempfile::tempdir().unwrap();
        let dir = fixture.path().join("runtime/smplos");
        let pidfile = dir.join("start-menu.pid");
        write_pidfile(&pidfile, 4321).unwrap();
        assert_eq!(fs::read_to_string(&pidfile).unwrap(), "4321\n");
        assert_eq!(read_pid(&pidfile), Some(4321));
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "created directory must be private");

        // Replacement goes through a temp file; a stale temp of ours is replaced too.
        fs::write(dir.join(".start-menu.pid.99.tmp"), "junk").unwrap();
        write_pidfile(&pidfile, 99).unwrap();
        assert_eq!(read_pid(&pidfile), Some(99));
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["start-menu.pid"]);

        // An existing directory keeps its mode.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o750)).unwrap();
        write_pidfile(&pidfile, 7).unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o750);
    }

    #[test]
    fn pidfile_is_removed_only_while_it_names_us() {
        let fixture = tempfile::tempdir().unwrap();
        let pidfile = fixture.path().join("start-menu.pid");
        assert!(!remove_pidfile_if_ours(&pidfile, 10).unwrap(), "missing file");
        write_pidfile(&pidfile, 11).unwrap();
        assert!(!remove_pidfile_if_ours(&pidfile, 10).unwrap());
        assert_eq!(read_pid(&pidfile), Some(11), "another resident's pidfile stays");
        fs::write(&pidfile, "garbage").unwrap();
        assert!(!remove_pidfile_if_ours(&pidfile, 10).unwrap());
        assert!(pidfile.exists());
        write_pidfile(&pidfile, 10).unwrap();
        assert!(remove_pidfile_if_ours(&pidfile, 10).unwrap());
        assert!(!pidfile.exists());
    }

    #[test]
    fn only_a_live_start_menu_other_than_us_is_an_instance() {
        let fixture = tempfile::tempdir().unwrap();
        let proc_root = fixture.path().join("proc");
        let pidfile = fixture.path().join("start-menu.pid");
        let own = 500;
        assert_eq!(running_instance(&pidfile, &proc_root, own), None, "no pidfile");

        write_pidfile(&pidfile, 600).unwrap();
        assert_eq!(running_instance(&pidfile, &proc_root, own), None, "dead PID");

        fake_process(&proc_root, 600, "firefox", 'S');
        assert_eq!(running_instance(&pidfile, &proc_root, own), None, "wrong comm");

        fake_process(&proc_root, 600, "start-menu", 'Z');
        assert_eq!(running_instance(&pidfile, &proc_root, own), None, "zombie");

        fake_process(&proc_root, 600, "start-menu", 'S');
        assert_eq!(running_instance(&pidfile, &proc_root, own), Some(600));

        fake_process(&proc_root, own, "start-menu", 'R');
        write_pidfile(&pidfile, own).unwrap();
        assert_eq!(running_instance(&pidfile, &proc_root, own), None, "ourselves");

        fs::write(&pidfile, "0\n").unwrap();
        assert_eq!(running_instance(&pidfile, &proc_root, own), None, "invalid PID");
    }

    #[test]
    fn claim_hands_off_to_a_live_instance_or_becomes_primary() {
        let fixture = tempfile::tempdir().unwrap();
        let proc_root = fixture.path().join("proc");
        let pidfile = fixture.path().join("run/smplos/start-menu.pid");
        let patience = Duration::from_millis(50);

        // Stale pidfile: become primary and take the lock.
        write_pidfile(&pidfile, 600).unwrap();
        let Claim::Primary(Some(lock)) = claim(&pidfile, &proc_root, 500, patience).unwrap()
        else {
            panic!("a stale pidfile must not block startup");
        };

        // While the lock is held and no pidfile names a live menu, a second
        // launch waits and then gives up instead of starting another resident.
        let started = Instant::now();
        let error = claim(&pidfile, &proc_root, 501, patience).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= patience);

        // Once the resident publishes its pidfile, later launches hand off.
        fake_process(&proc_root, 500, "start-menu", 'S');
        write_pidfile(&pidfile, 500).unwrap();
        assert!(matches!(
            claim(&pidfile, &proc_root, 501, patience).unwrap(),
            Claim::Existing(500)
        ));

        // After the resident exits (lock released, pidfile removed), a new
        // launch becomes primary again.
        assert!(remove_pidfile_if_ours(&pidfile, 500).unwrap());
        drop(lock);
        assert!(matches!(
            claim(&pidfile, &proc_root, 501, patience).unwrap(),
            Claim::Primary(Some(_))
        ));
    }

    #[test]
    fn send_signal_refuses_group_addresses() {
        for pid in [0, u32::MAX, i32::MAX as u32 + 1] {
            assert_eq!(
                send_signal(pid, 0).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        // Signal 0 only checks that our own process exists.
        send_signal(std::process::id(), 0).unwrap();
    }
}
