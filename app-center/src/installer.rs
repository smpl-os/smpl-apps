use crate::catalog::Source;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;

/// A running process with streaming output and interactive input.
pub struct StreamingProcess {
    child: Child,
    output_rx: mpsc::Receiver<String>,
    stdin: Option<ChildStdin>,
}

/// Result for operations that complete instantly (no streaming needed).
pub struct ImmediateResult {
    pub success: bool,
    pub message: String,
}

/// Either a live streaming process or an instant result.
pub enum SpawnResult {
    Streaming(StreamingProcess),
    Immediate(ImmediateResult),
}

impl StreamingProcess {
    /// Drain all available output lines without blocking.
    pub fn poll_output(&self) -> Vec<String> {
        let mut lines = Vec::new();
        while let Ok(line) = self.output_rx.try_recv() {
            lines.push(line);
        }
        lines
    }

    /// Send a line of text to the process stdin.
    pub fn send_input(&mut self, text: &str) {
        if let Some(ref mut stdin) = self.stdin {
            let _ = writeln!(stdin, "{}", text);
            let _ = stdin.flush();
        }
    }

    /// Check if the process exited. Returns None if still running.
    pub fn try_wait(&mut self) -> Option<bool> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.success()),
            _ => None,
        }
    }

    /// Kill the process and all its children.
    pub fn kill(&mut self) {
        // Drop stdin first to unblock any reads
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Strip ANSI escape sequences for clean display.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn spawn_process(cmd: &str, args: &[&str]) -> Result<StreamingProcess, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run {}: {}", cmd, e))?;

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel::<String>();

    if let Some(out) = stdout {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(strip_ansi(&line));
            }
        });
    }

    if let Some(err) = stderr {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(strip_ansi(&line));
            }
        });
    }

    Ok(StreamingProcess {
        child,
        output_rx: rx,
        stdin,
    })
}

/// Install an official-repo package. Falls back to pkexec pacman if Yay is broken.
///
/// Uses `-Syu` (full system sync + upgrade) rather than `-S` so the install
/// can never be a partial upgrade. On Arch, installing a package against an
/// out-of-date system can pull in a binary that links a newer soname than the
/// installed libraries (e.g. blender 5.1 needs libopenjph.so.0.28 but the
/// frozen offline mirror shipped libopenjph.so.0.27) -- the package installs
/// "successfully" but the binary aborts on startup with a missing-library
/// error. `-Syu` keeps the whole system consistent so this can't happen.
fn spawn_pacman_install(id: &str) -> SpawnResult {
    let (cmd, args): (&str, Vec<&str>) = if yay_works() {
        ("yay", vec!["-Syu", "--noconfirm", id])
    } else {
        ("pkexec", vec!["pacman", "-Syu", "--noconfirm", id])
    };
    match spawn_process(cmd, &args) {
        Ok(p) => SpawnResult::Streaming(p),
        Err(e) => SpawnResult::Immediate(ImmediateResult { success: false, message: e }),
    }
}

/// Install an AUR package with the OS-provided Yay.
///
/// Uses `-Syu` for the same reason as `spawn_pacman_install`: AUR packages
/// are compiled locally against currently-installed system libraries, so
/// installing on top of a stale system can produce a binary that breaks the
/// moment a dependency soname changes. Syncing first keeps the install
/// reproducible.
fn spawn_aur_install(id: &str) -> SpawnResult {
    let script = yay_upgrade_script(&[id]);
    match spawn_process("bash", &["-c", &script]) {
        Ok(p) => SpawnResult::Streaming(p),
        Err(e) => SpawnResult::Immediate(ImmediateResult { success: false, message: e }),
    }
}

fn yay_upgrade_script(ids: &[&str]) -> String {
    let list = ids
        .iter()
        .map(|id| format!("'{}'", id.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "set -euo pipefail\n\
         if ! yay --version || ! yay -Q pacman; then\n\
           echo 'ERROR: Yay is missing or cannot access libalpm. Run Update OS in App Center, then retry.' >&2\n\
           exit 1\n\
         fi\n\
         yay -Syu --noconfirm {list}\n",
    )
}

/// Remove a package. Falls back to pkexec pacman if Yay is broken.
fn spawn_pacman_remove(id: &str) -> SpawnResult {
    let (cmd, args): (&str, Vec<&str>) = if yay_works() {
        ("yay", vec!["-Rns", "--noconfirm", id])
    } else {
        ("pkexec", vec!["pacman", "-R", "--noconfirm", id])
    };
    match spawn_process(cmd, &args) {
        Ok(p) => SpawnResult::Streaming(p),
        Err(e) => SpawnResult::Immediate(ImmediateResult { success: false, message: e }),
    }
}

/// Spawn an install process with streaming output.
pub fn spawn_install(source: &Source, id: &str) -> SpawnResult {
    match source {
        Source::Aur => spawn_aur_install(id),
        Source::Pacman => spawn_pacman_install(id),
        Source::Flatpak => {
            if !which_exists("flatpak") {
                return SpawnResult::Immediate(ImmediateResult {
                    success: false,
                    message: "flatpak is not installed. Run: sudo pacman -S flatpak".into(),
                });
            }
            let _ = Command::new("flatpak")
                .args([
                    "remote-add", "--if-not-exists", "--user", "flathub",
                    "https://dl.flathub.org/repo/flathub.flatpakrepo",
                ])
                .output();
            match spawn_process("flatpak", &["install", "-y", "--user", "flathub", id]) {
                Ok(p) => SpawnResult::Streaming(p),
                Err(e) => SpawnResult::Immediate(ImmediateResult {
                    success: false,
                    message: e,
                }),
            }
        }
        Source::Script => match spawn_process(id, &["install"]) {
            Ok(p) => SpawnResult::Streaming(p),
            Err(e) => SpawnResult::Immediate(ImmediateResult {
                success: false,
                message: e,
            }),
        },
        Source::AppImage => SpawnResult::Immediate(ImmediateResult {
            success: false,
            message: format!(
                "Visit appimage.github.io to download {}.AppImage, then place it in ~/.local/bin/",
                id
            ),
        }),
    }
}

/// Spawn an update for a specific set of pacman/AUR and flatpak packages.
/// AUR selections use Yay with a full upgrade, never targeted pacman -Sy.
/// Repo-only selections retain their existing path; empty lists are skipped.
pub fn spawn_update(pacman_ids: &[String], aur_ids: &[String], flatpak_ids: &[String]) -> SpawnResult {
    if !has_internet() {
        return SpawnResult::Immediate(ImmediateResult {
            success: false,
            message: "No internet connection. Updates require an internet connection — please connect and try again.".into(),
        });
    }
    let mut steps: Vec<String> = Vec::new();
    if let Some(step) = native_update_step(pacman_ids, aur_ids) {
        steps.push(step);
    }
    if !flatpak_ids.is_empty() && which_exists("flatpak") {
        let list = flatpak_ids
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        steps.push(format!("flatpak update -y --user {}", list));
    }
    if steps.is_empty() {
        return SpawnResult::Immediate(ImmediateResult {
            success: true,
            message: "Nothing selected to update".into(),
        });
    }
    let script = format!("set -e\n{}\n", steps.join("\n"));
    match spawn_process("bash", &["-c", &script]) {
        Ok(p) => SpawnResult::Streaming(p),
        Err(e) => SpawnResult::Immediate(ImmediateResult { success: false, message: e }),
    }
}

fn native_update_step(pacman_ids: &[String], aur_ids: &[String]) -> Option<String> {
    if !aur_ids.is_empty() {
        let ids = pacman_ids
            .iter()
            .chain(aur_ids)
            .map(String::as_str)
            .collect::<Vec<_>>();
        Some(yay_upgrade_script(&ids))
    } else if !pacman_ids.is_empty() {
        let list = pacman_ids
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join(" ");
        // pkexec shows a graphical polkit password dialog (same as smplos-update).
        // Refresh mirrors first (skip if reflector is missing) to dodge expired/dead
        // mirrors, then upgrade ONLY the selected packages. We deliberately do not
        // run a full -Syu: the hypr stack is pinned, so a system upgrade can't be
        // satisfied. Targeted -Sy upgrades just the chosen apps + their deps.
        Some(format!(
            "pkexec sh -c \"command -v reflector >/dev/null 2>&1 && timeout 12 reflector --latest 20 --sort rate --protocol https --connection-timeout 2 --download-timeout 2 --save /etc/pacman.d/mirrorlist; pacman -Sy --needed --noconfirm {}\"",
            list
        ))
    } else {
        None
    }
}

/// Spawn an uninstall process with streaming output.
pub fn spawn_uninstall(source: &Source, id: &str, name: &str) -> SpawnResult {
    match source {
        Source::Aur | Source::Pacman => spawn_pacman_remove(id),
        Source::Flatpak => {
            match spawn_process("flatpak", &["uninstall", "-y", "--user", id]) {
                Ok(p) => SpawnResult::Streaming(p),
                Err(e) => SpawnResult::Immediate(ImmediateResult {
                    success: false,
                    message: e,
                }),
            }
        }
        Source::AppImage => {
            let home = std::env::var("HOME").unwrap_or_default();
            for path in &[
                format!("/opt/appimages/{}.AppImage", name),
                format!("{}/.local/bin/{}.AppImage", home, name),
            ] {
                let _ = std::fs::remove_file(path);
            }
            let desktop = format!(
                "{}/.local/share/applications/{}-appimage.desktop",
                home,
                name.to_lowercase()
            );
            let _ = std::fs::remove_file(&desktop);
            SpawnResult::Immediate(ImmediateResult {
                success: true,
                message: format!("Removed {}", name),
            })
        }
        Source::Script => match spawn_process(id, &["uninstall"]) {
            Ok(p) => SpawnResult::Streaming(p),
            Err(e) => SpawnResult::Immediate(ImmediateResult {
                success: false,
                message: e,
            }),
        },
    }
}

fn which_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Quick connectivity check so we can fail with a clear message instead of
/// cryptic mirror errors. Short-timeout HTTPS request to a reliable host.
fn has_internet() -> bool {
    Command::new("curl")
        .args(["-sS", "--max-time", "5", "-I", "https://archlinux.org"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Yay dynamically loads libalpm, so --version alone cannot establish readiness.
fn yay_works() -> bool {
    yay_works_with(|| Command::new("yay"))
}

fn yay_works_with(mut command: impl FnMut() -> Command) -> bool {
    [["--version"].as_slice(), ["-Q", "pacman"].as_slice()]
        .iter()
        .all(|args| {
            command()
                .args(*args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
}

pub fn aur_updates() -> Result<Vec<String>, String> {
    aur_updates_with(Command::new("yay"))
}

fn aur_updates_with(mut command: Command) -> Result<Vec<String>, String> {
    let output = command.args(["-Qua"]).output().map_err(|e| {
        format!("Cannot check AUR updates: {e}. Run Update OS in App Center, then retry.")
    })?;
    // Like pacman -Qu, an empty query may exit 1 when nothing needs upgrading.
    if !output.status.success()
        && !(output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty())
    {
        return Err(format!(
            "Cannot check AUR updates ({}): {}. Run Update OS in App Center, then retry.",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if !output.stderr.is_empty() {
        eprintln!(
            "[app-center] Yay: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "app-center-yay-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("bin")).unwrap();
            Self(path)
        }

        fn yay(&self, body: &str) {
            let path = self.0.join("bin/yay");
            fs::write(&path, format!(
                "#!/bin/bash\nprintf '<%s>' \"$@\" >> \"$CALLS\"\nprintf '\\n' >> \"$CALLS\"\n{body}\n"
            )).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn command(&self) -> Command {
            // Interpret fixtures directly to avoid ETXTBSY during parallel spawns.
            let mut command = Command::new("/bin/bash");
            command.arg(self.0.join("bin/yay"));
            command.env("CALLS", self.0.join("calls"));
            command
        }

        fn install(&self, id: &str) -> std::process::Output {
            self.script(&yay_upgrade_script(&[id]))
        }

        fn script(&self, script: &str) -> std::process::Output {
            Command::new("/bin/bash")
                .args(["-c", script])
                .env("PATH", self.0.join("bin"))
                .env("HOME", &self.0)
                .env("CALLS", self.0.join("calls"))
                .stdin(Stdio::null())
                .output()
                .unwrap()
        }

        fn calls(&self) -> String {
            fs::read_to_string(self.0.join("calls")).unwrap_or_default()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn yay_readiness_requires_an_actual_libalpm_query() {
        let fx = Fixture::new();
        fx.yay("case \"$1\" in --version) exit 0;; -Q) exit 1;; esac");
        assert!(!yay_works_with(|| fx.command()));
        assert_eq!(fx.calls(), "<--version>\n<-Q><pacman>\n");

        let fx = Fixture::new();
        fx.yay("exit 0");
        assert!(yay_works_with(|| fx.command()));
        assert_eq!(fx.calls(), "<--version>\n<-Q><pacman>\n");

        let fx = Fixture::new();
        assert!(!yay_works_with(|| fx.command()));
    }

    #[test]
    fn aur_install_requires_working_yay_without_bootstrapping_it() {
        for body in [
            None,
            Some("exit 1"),
            Some("case \"$1\" in --version) exit 0;; *) echo 'libalpm unavailable' >&2; exit 1;; esac"),
        ] {
            let fx = Fixture::new();
            if let Some(body) = body {
                fx.yay(body);
            }
            let output = fx.install("voxtype-bin");
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("Run Update OS"));
            assert!(!fx.calls().contains("<-Syu>"));
        }
    }

    #[test]
    fn aur_install_keeps_full_upgrade_and_literal_package_argument() {
        let fx = Fixture::new();
        fx.yay("exit 0");
        assert!(fx.install("pkg' literal").status.success());
        assert_eq!(
            fx.calls(),
            "<--version>\n<-Q><pacman>\n<-Syu><--noconfirm><pkg' literal>\n"
        );

        let fx = Fixture::new();
        fx.yay("case \"$1\" in -Syu) echo 'install failed' >&2; exit 42;; esac");
        let output = fx.install("voxtype-bin");
        assert_eq!(output.status.code(), Some(42));
        assert!(String::from_utf8_lossy(&output.stderr).contains("install failed"));
    }

    #[test]
    fn selected_aur_updates_use_yay_with_a_full_base_upgrade() {
        for repo in [vec![], vec!["blender".to_string()]] {
            let fx = Fixture::new();
            fx.yay("exit 0");
            let step = native_update_step(&repo, &["brave-bin".to_string()]).unwrap();
            assert!(!step.contains("pkexec"));
            let output = fx.script(&format!("{step}printf 'next-step\\n'\n"));
            assert!(output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains("next-step"));
            let targets = if repo.is_empty() { "" } else { "<blender>" };
            assert_eq!(
                fx.calls(),
                format!("<--version>\n<-Q><pacman>\n<-Syu><--noconfirm>{targets}<brave-bin>\n")
            );
        }

        let fx = Fixture::new();
        fx.yay("case \"$1\" in -Q) exit 1;; esac");
        let step = native_update_step(&[], &["brave-bin".to_string()]).unwrap();
        let output = fx.script(&step);
        assert!(!output.status.success());
        assert!(!fx.calls().contains("<-Syu>"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Run Update OS"));
    }

    #[test]
    fn repo_only_update_path_is_preserved_and_empty_native_selection_is_skipped() {
        let step = native_update_step(&["blender".to_string()], &[]).unwrap();
        assert!(step.starts_with("pkexec sh -c "));
        assert!(step.contains("pacman -Sy --needed --noconfirm 'blender'"));
        assert!(!step.contains("yay"));
        assert!(native_update_step(&[], &[]).is_none());

        let main = include_str!("main.rs");
        assert_eq!(main.matches("\"AUR\" => aur_ids.push(item.id.to_string())").count(), 2);
        assert!(main.contains("installer::spawn_update(&pacman_ids, &aur_ids, &flatpak_ids)"));
    }

    #[test]
    fn aur_update_query_reports_failures_and_accepts_empty_results() {
        let fx = Fixture::new();
        fx.yay("printf 'voxtype-bin 1.0 -> 2.0\\nother-app 2.0 -> 3.0\\n'");
        assert_eq!(aur_updates_with(fx.command()).unwrap(), ["voxtype-bin", "other-app"]);
        assert_eq!(fx.calls(), "<-Qua>\n");

        for code in [0, 1] {
            let fx = Fixture::new();
            fx.yay(&format!("exit {code}"));
            assert!(aur_updates_with(fx.command()).unwrap().is_empty());
        }

        for body in [
            "echo 'libalpm unavailable' >&2; exit 1",
            "echo 'incomplete output'; exit 1",
            "exit 2",
        ] {
            let fx = Fixture::new();
            fx.yay(body);
            assert!(aur_updates_with(fx.command()).unwrap_err().contains("Run Update OS"));
        }
        let fx = Fixture::new();
        assert!(aur_updates_with(fx.command()).unwrap_err().contains("Run Update OS"));
    }

    #[test]
    fn installed_tab_exposes_the_aur_query_error() {
        let main = include_str!("main.rs");
        let ui = include_str!("../ui/main.slint");
        assert!(main.contains("installer::aur_updates()"));
        assert_eq!(main.matches("ui.set_installed_update_error(update_error.into())").count(), 4);
        assert!(ui.contains("text: root.installed-update-error;"));
        assert!(ui.contains("root.current-tab == 0 && !root.show-detail && !root.checking-for-updates && root.installed-update-error != \"\""));
    }
}
