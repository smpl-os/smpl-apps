use std::process::Command;

pub fn command() -> Command {
    let mut command = Command::new("smplos-update");
    command.args(["--mode", "full"]);
    command
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
        fs::write(&updater, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&updater, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = command();
        assert_eq!(command.get_program(), "smplos-update");
        let output = command
            .env("PATH", &fixture.0)
            .env("HOME", &fixture.0)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
            .wait_with_output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "--mode\nfull\n");
    }

    #[test]
    fn missing_updater_is_a_spawn_error_not_a_success() {
        let fixture = Fixture::new();
        assert!(command()
            .env("PATH", &fixture.0)
            .env("HOME", &fixture.0)
            .spawn()
            .is_err());
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
        assert!(callback.contains("os_update::command().spawn()"));
        assert!(!callback.contains("Command::new"));
    }
}
