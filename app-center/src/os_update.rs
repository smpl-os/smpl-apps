use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

pub fn command(log_path: &Path) -> Command {
    let mut command = Command::new("smplos-update");
    clean_coordinator_state(&mut command);
    command
        .args(["--mode", "full"])
        .env("SMPLOS_UPDATE_LOG_FILE", log_path);
    command
}

pub fn new_log() -> Result<(PathBuf, File), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("Cannot launch Update OS without an absolute HOME directory")?;
    let directory = home.join(".local/state/smplos/update");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("Could not timestamp the OS update: {error}"))?
        .as_nanos();
    let path = directory.join(format!(
        "update-app-center-{}-{stamp}.log",
        std::process::id()
    ));
    let file = OpenOptions::new()
        .create_new(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
    Ok((path, file))
}

/// Reap only the launcher. Server-style terminals can detach before the OS update ends.
pub fn wait_for_launcher(mut child: Child) -> Result<(), String> {
    let status = child
        .wait()
        .map_err(|error| format!("Could not wait for the OS update launcher: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "OS update launcher exited with {status}. Check the update terminal and OS update log before retrying."
        ))
    }
}

#[derive(Debug, PartialEq)]
pub enum Completion {
    Verified { warnings: bool },
    Failed,
    Interrupted,
    Unverified,
}

pub fn read_completion(log_path: &Path) -> Result<Option<Completion>, String> {
    let mut receipt = log_path.as_os_str().to_os_string();
    receipt.push(".result");
    let file = match File::open(Path::new(&receipt)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read the OS completion receipt: {error}")),
    };
    let mut text = String::new();
    file.take(8193)
        .read_to_string(&mut text)
        .map_err(|error| format!("Could not read the OS completion receipt: {error}"))?;
    if text.len() > 8192 {
        return Err("OS completion receipt is too large; completion is unverified".into());
    }
    parse_completion(&text, log_path).map(Some)
}

fn parse_completion(text: &str, log_path: &Path) -> Result<Completion, String> {
    let invalid = || "Invalid OS completion receipt; completion is unverified".to_string();
    let mut fields = HashMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once('=').ok_or_else(invalid)?;
        if fields.insert(key, value).is_some() {
            return Err(invalid());
        }
    }
    if fields.get("protocol") != Some(&"1")
        || fields.get("log").copied() != log_path.to_str()
        || fields.get("mode") != Some(&"full")
        || fields.get("lock_released") != Some(&"1")
        || !matches!(fields.get("core_verified"), Some(&"0" | &"1"))
        || !fields.contains_key("stage")
    {
        return Err(invalid());
    }
    match fields.get("status").copied() {
        Some("complete" | "complete-with-warnings") => {
            if fields.get("stage") != Some(&"finished") {
                return Err(invalid());
            }
            if fields.get("core_verified") != Some(&"1") {
                return Ok(Completion::Unverified);
            }
            Ok(Completion::Verified {
                warnings: fields.get("status") == Some(&"complete-with-warnings"),
            })
        }
        Some("failed") => Ok(Completion::Failed),
        Some("interrupted") => Ok(Completion::Interrupted),
        _ => Err(invalid()),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

pub struct Executable {
    path: PathBuf,
    running: Identity,
}

pub struct RestartTarget {
    path: PathBuf,
    verified: Identity,
}

impl Executable {
    pub fn current() -> Result<Self, String> {
        let path = std::env::current_exe()
            .map_err(|error| format!("Could not locate App Center's executable: {error}"))?;
        let running = fs::metadata("/proc/self/exe")
            .map_err(|error| format!("Could not identify running App Center: {error}"))?;
        Ok(Self {
            path,
            running: Identity::of(&running),
        })
    }

    pub fn replacement(&self) -> Result<Option<RestartTarget>, String> {
        let metadata = fs::metadata(&self.path)
            .map_err(|error| format!("Could not inspect installed App Center: {error}"))?;
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            return Err("Installed App Center is not an executable file".into());
        }
        let verified = Identity::of(&metadata);
        Ok((verified != self.running).then(|| RestartTarget {
            path: self.path.clone(),
            verified,
        }))
    }
}

impl RestartTarget {
    fn command(&self) -> Result<(File, Command), String> {
        let file = File::open(&self.path)
            .map_err(|error| format!("Could not open installed App Center: {error}"))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("Could not verify installed App Center: {error}"))?;
        if Identity::of(&metadata) != self.verified {
            return Err("Installed App Center changed again. Close and reopen it after the current OS update finishes.".into());
        }
        // Execute the verified open inode, not a path another update could replace.
        let mut command = Command::new(format!("/proc/self/fd/{}", file.as_raw_fd()));
        command.arg0(&self.path);
        clean_coordinator_state(&mut command);
        Ok((file, command))
    }

    pub fn restart(&self) -> Result<(), String> {
        let (file, mut command) = self.command()?;
        let image = ExecImage::new(&self.path)?;
        let fd = file.as_raw_fd();
        // fexecve has no execvp-style shell fallback for malformed images.
        // Rust's spawn error pipe keeps the old UI alive on any exec failure.
        unsafe {
            command.pre_exec(move || Err(image.execute(fd)));
        }
        command
            .spawn()
            .map_err(|error| format!("Could not restart App Center: {error}"))?;
        // Exiting, rather than self-exec, reparents any launcher still holding
        // its terminal open. Its waiter thread must not be lost across exec.
        std::process::exit(0);
    }
}

struct ExecImage {
    _arguments: Vec<CString>,
    _environment: Vec<CString>,
    arguments: Box<[*const libc::c_char]>,
    environment: Box<[*const libc::c_char]>,
}

// The pointers refer only to our owned, immutable CString allocations, which
// remain valid when the enclosing image moves into Command's pre_exec closure.
unsafe impl Send for ExecImage {}
unsafe impl Sync for ExecImage {}

impl ExecImage {
    fn new(path: &Path) -> Result<Self, String> {
        let string = |value: &std::ffi::OsStr| {
            CString::new(value.as_bytes()).map_err(|error| {
                format!("Invalid App Center restart argument/environment: {error}")
            })
        };
        let arguments = std::iter::once(path.as_os_str().to_owned())
            .chain(std::env::args_os().skip(1))
            .map(|value| string(&value))
            .collect::<Result<Vec<_>, _>>()?;
        let environment = std::env::vars_os()
            .filter(|(key, _)| !is_coordinator_key(key))
            .map(|(mut key, value)| {
                key.push("=");
                key.push(value);
                string(&key)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pointers = |strings: &[CString]| {
            strings
                .iter()
                .map(|value| value.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect::<Vec<_>>()
                .into_boxed_slice()
        };
        Ok(Self {
            arguments: pointers(&arguments),
            environment: pointers(&environment),
            _arguments: arguments,
            _environment: environment,
        })
    }

    fn execute(&self, fd: i32) -> std::io::Error {
        unsafe {
            libc::fexecve(fd, self.arguments.as_ptr(), self.environment.as_ptr());
        }
        std::io::Error::last_os_error()
    }
}

fn is_coordinator_key(key: &std::ffi::OsStr) -> bool {
    key.to_str().is_some_and(|key| {
        key.starts_with("SMPLOS_UPDATE")
            || key.starts_with("SMPLOS_COORDINATOR")
            || key == "SMPLOS_SUDO_READY"
            || key == "SMPLOS_SUDO_KEEPALIVE_PID"
            || key == "SMPLOS_SYSTEM_UPGRADED"
            || key == "SMPLOS_INVOKER_UID"
            || key == "SMPLOS_INVOKER_USER"
            || key == "BASH_FUNC_sudo%%"
    })
}

fn clean_coordinator_state(command: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if is_coordinator_key(&key) {
            command.env_remove(key);
        }
    }
    // Successful launches cannot retain the updater's FD9.
    // Only async-signal-safe calls run in the forked child.
    unsafe {
        command.pre_exec(|| {
            let flags = libc::fcntl(9, libc::F_GETFD);
            if flags == -1 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::EBADF) {
                    return Err(error);
                }
            } else if libc::fcntl(9, libc::F_SETFD, flags | libc::FD_CLOEXEC) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "app-center-update-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn confirmed_os_update_launches_full_mode_through_path_without_real_updater() {
        let fixture = Fixture::new();
        let updater = fixture.0.join("smplos-update");
        std::os::unix::fs::symlink("/bin/echo", &updater).unwrap();
        let logpath = fixture.0.join("run.log");
        let mut command = command(&logpath);
        assert_eq!(command.get_program(), "smplos-update");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["--mode", "full"]);
        assert!(command.get_envs().any(|(key, value)| {
            key == "SMPLOS_UPDATE_LOG_FILE" && value == Some(logpath.as_os_str())
        }));
        let output = command
            .env("PATH", &fixture.0)
            .env("HOME", &fixture.0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
            .wait_with_output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "--mode full\n");
    }

    #[test]
    fn missing_updater_is_a_spawn_error_not_a_success() {
        let fixture = Fixture::new();
        assert!(command(&fixture.0.join("run.log"))
            .env("PATH", &fixture.0)
            .env("HOME", &fixture.0)
            .spawn()
            .is_err());
    }

    fn receipt(logpath: &Path, status: &str) -> String {
        format!(
            "protocol=1\nstatus={status}\nstage=finished\nlog={}\nmode=full\ncore_verified=1\nlock_released=1\noptional_phase_warnings=0\n",
            logpath.display()
        )
    }

    #[test]
    fn completion_requires_the_exact_invocation_verified_core_and_released_lock() {
        let logpath = Path::new("/tmp/app-center-unit.log");
        let good = receipt(logpath, "complete");
        assert_eq!(
            parse_completion(&good, logpath).unwrap(),
            Completion::Verified { warnings: false }
        );
        let warnings = receipt(logpath, "complete-with-warnings");
        assert_eq!(
            parse_completion(&warnings, logpath).unwrap(),
            Completion::Verified { warnings: true }
        );
        for (before, after) in [
            ("protocol=1", "protocol=2"),
            ("mode=full", "mode=apps"),
            ("stage=finished", "stage=installing"),
            ("lock_released=1", "lock_released=0"),
            ("core_verified=1", "core_verified=yes"),
            ("log=/tmp/app-center-unit.log", "log=/tmp/other-update.log"),
            ("status=complete", "status=running"),
            ("lock_released=1\n", ""),
        ] {
            assert!(
                parse_completion(&good.replace(before, after), logpath).is_err(),
                "{before}"
            );
        }
        assert!(parse_completion(&format!("{good}protocol=1\n"), logpath).is_err());
        assert_eq!(
            parse_completion(&good.replace("core_verified=1", "core_verified=0"), logpath).unwrap(),
            Completion::Unverified
        );
        assert_eq!(
            parse_completion(&receipt(logpath, "failed"), logpath).unwrap(),
            Completion::Failed
        );
        assert_eq!(
            parse_completion(&receipt(logpath, "interrupted"), logpath).unwrap(),
            Completion::Interrupted
        );
    }

    #[test]
    fn no_receipt_or_global_status_does_not_verify_launcher_success() {
        let fixture = Fixture::new();
        let logpath = fixture.0.join("run.log");
        fs::write(fixture.0.join("status"), receipt(&logpath, "complete")).unwrap();
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        assert!(wait_for_launcher(child).is_ok());
        assert_eq!(read_completion(&logpath).unwrap(), None);
        fs::write(fixture.0.join("run.log.result"), "x".repeat(8193)).unwrap();
        assert!(read_completion(&logpath).unwrap_err().contains("too large"));
    }

    #[test]
    fn receipt_is_observed_while_terminal_still_waits_for_acknowledgement() {
        use std::io::Write;
        let fixture = Fixture::new();
        let logpath = fixture.0.join("run.log");
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read answer"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        fs::write(
            fixture.0.join("run.log.result"),
            receipt(&logpath, "complete"),
        )
        .unwrap();
        assert_eq!(child.try_wait().unwrap(), None);
        assert_eq!(
            read_completion(&logpath).unwrap(),
            Some(Completion::Verified { warnings: false })
        );
        writeln!(child.stdin.take().unwrap(), "done").unwrap();
        wait_for_launcher(child).unwrap();
    }

    #[test]
    fn restart_requires_a_changed_executable_and_rechecks_the_verified_inode() {
        let fixture = Fixture::new();
        let path = fixture.0.join("app-center");
        fs::copy("/bin/true", &path).unwrap();
        let executable = Executable {
            path: path.clone(),
            running: Identity::of(&fs::metadata(&path).unwrap()),
        };
        assert!(executable.replacement().unwrap().is_none());
        let staged = fixture.0.join("new-app-center");
        fs::copy("/bin/true", &staged).unwrap();
        fs::rename(&staged, &path).unwrap();
        let target = executable.replacement().unwrap().unwrap();
        let (_file, _command) = target.command().unwrap();
        fs::copy("/bin/true", &staged).unwrap();
        fs::rename(&staged, &path).unwrap();
        assert!(target.command().unwrap_err().contains("changed again"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(executable.replacement().is_err());
    }

    #[test]
    fn restart_spawns_verified_binary_without_coordinator_state_or_fd9() {
        const PROBE: &str = "APP_CENTER_RESTART_TEST";
        const FLAGS: &[&str] = &[
            "SMPLOS_UPDATE_LOCKED",
            "SMPLOS_UPDATER_REFRESHED",
            "SMPLOS_UPDATE_LOG_FILE",
            "SMPLOS_COORDINATOR_TEST",
            "SMPLOS_SUDO_READY",
            "SMPLOS_SUDO_KEEPALIVE_PID",
            "SMPLOS_SYSTEM_UPGRADED",
            "SMPLOS_INVOKER_UID",
            "SMPLOS_INVOKER_USER",
            "BASH_FUNC_sudo%%",
        ];
        match std::env::var(PROBE).as_deref() {
            Ok("after") => {
                for key in FLAGS {
                    assert!(std::env::var_os(key).is_none(), "{key}");
                }
                assert_eq!(
                    std::env::var("SMPLOS_TERMINAL").unwrap(),
                    "preserved-terminal"
                );
                assert_eq!(unsafe { libc::fcntl(9, libc::F_GETFD) }, -1);
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EBADF)
                );
                assert_ne!(
                    std::process::id().to_string(),
                    std::env::var("APP_CENTER_ORIGINAL_PID").unwrap()
                );
                println!("verified restart adopted executable without coordinator state");
            }
            Ok("before") => {
                // This runs only in the isolated test subprocess, never the test runner.
                let input = File::open("/dev/null").unwrap();
                assert_eq!(unsafe { libc::dup2(input.as_raw_fd(), 9) }, 9);
                assert_eq!(unsafe { libc::fcntl(9, libc::F_SETFD, 0) }, 0);
                std::env::set_var(PROBE, "after");
                std::env::set_var("APP_CENTER_ORIGINAL_PID", std::process::id().to_string());
                let path = std::env::current_exe().unwrap();
                let target = RestartTarget {
                    verified: Identity::of(&fs::metadata(&path).unwrap()),
                    path,
                };
                panic!("{}", target.restart().unwrap_err());
            }
            _ => {
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "os_update::tests::restart_spawns_verified_binary_without_coordinator_state_or_fd9",
                        "--test-threads=1",
                        "--nocapture",
                    ])
                    .env(PROBE, "before")
                    .env("SMPLOS_TERMINAL", "preserved-terminal");
                for key in FLAGS {
                    command.env(key, "test-value");
                }
                let output = command.output().unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(String::from_utf8_lossy(&output.stdout)
                    .contains("verified restart adopted executable"));
            }
        }
    }

    #[test]
    fn failed_replacement_exec_returns_an_error_without_exiting_the_old_process() {
        let fixture = Fixture::new();
        let path = fixture.0.join("app-center");
        fs::write(&path, "not an executable image\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        let target = RestartTarget {
            verified: Identity::of(&fs::metadata(&path).unwrap()),
            path,
        };
        assert!(target
            .restart()
            .unwrap_err()
            .contains("Could not restart App Center"));
    }

    #[test]
    fn launcher_is_waited_for_and_reaped_without_running_an_update() {
        use std::io::Write;
        use std::sync::mpsc;
        use std::time::Duration;

        let mut child = Command::new("/bin/sh")
            .args(["-c", "read answer"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut stdin = child.stdin.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || tx.send(wait_for_launcher(child)).unwrap());
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        writeln!(stdin, "done").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(()));
        worker.join().unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn failed_launcher_exit_is_not_reported_as_success() {
        let child = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let error = wait_for_launcher(child).unwrap_err();
        assert!(error.contains("exit status: 7"));
        assert!(error.contains("Check the update terminal and OS update log"));
    }

    #[test]
    fn ui_confirmation_and_rust_callback_use_the_tested_command() {
        let ui = include_str!("../ui/main.slint");
        let main = include_str!("main.rs");
        assert!(ui.contains("clicked => { root.show-update-os-confirm = true; }"));
        assert!(ui.contains(
            "root.show-update-os-confirm = false;\n                            root.update-os();"
        ));
        let callback = main
            .split("ui.on_update_os")
            .nth(1)
            .unwrap()
            .split("// -- Copy log")
            .next()
            .unwrap();
        assert!(callback.contains("os_update::command(&logpath).spawn()"));
        assert!(!callback.contains("Command::new"));
        assert!(callback.contains("std::thread::spawn"));
        assert!(callback.contains("os_update::wait_for_launcher(child)"));
        assert!(callback.contains("set_os_update_status"));
        assert!(!callback.contains("set_installing"));
        assert!(callback.contains("os_update::read_completion(logpath)"));
        assert!(callback.contains("executable.replacement()"));
        let restart = callback.split("ui.on_restart_app_center").nth(1).unwrap();
        assert!(restart.contains("if ui.get_installing()"));
        assert!(restart.contains("target.restart()"));
        assert!(ui.contains(
            "busy: root.installing;\n                clicked => { root.restart-app-center(); }"
        ));
        assert!(ui.contains("text: root.os-update-status;"));
        assert!(ui.contains("close and reopen App Center"));
    }
}
