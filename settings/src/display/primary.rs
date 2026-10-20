//! The primary monitor under the OS workspace policy.
//!
//! With the stable workspace policy on, the OS owns where workspace 1 and the
//! taskbar live. `workspace-ctl primary` names the primary monitor and
//! `workspace-ctl set-primary <CONNECTOR>` changes it: the policy saves the
//! choice, applies it at once and moves the taskbar itself, so Settings never
//! touches the bar. Older installs have neither subcommand: `read` then comes
//! back empty, the card keeps naming the display that shows workspace 1 and
//! offers no button (`Snapshot::can_set_primary`), and `set` is never called.

use super::hyprctl::{CommandOutput, Runner};

/// The monitor the policy names as primary, or `None` when it can't say
/// (policy off, older OS, a failed command or an unusable answer).
pub fn read(runner: &dyn Runner) -> Option<String> {
    let output = runner.run("workspace-ctl", &["primary"]).ok()?;
    if !output.success {
        return None;
    }
    connector(&output.stdout)
}

/// The connector on the first line, if it is a single token.
fn connector(stdout: &str) -> Option<String> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    line.chars()
        .all(|c| !c.is_whitespace() && !c.is_control())
        .then(|| line.to_string())
}

/// Makes `connector` the primary monitor. On failure the error is the OS's own
/// message when it gives one.
pub fn set(runner: &dyn Runner, connector: &str) -> Result<(), String> {
    // A leading dash would be parsed as an option.
    if connector.is_empty() || connector.starts_with('-') {
        return Err(format!("\"{connector}\" isn't a monitor name"));
    }
    let output = runner.run("workspace-ctl", &["set-primary", connector])?;
    if output.success {
        return Ok(());
    }
    Err(message(&output)
        .unwrap_or_else(|| "workspace-ctl set-primary failed without a message".into()))
}

/// The last non-empty line of stderr (else stdout): the reason for a failure,
/// also when the tool prints a usage block or a traceback before it.
fn message(output: &CommandOutput) -> Option<String> {
    [&output.stderr, &output.stdout]
        .into_iter()
        .find_map(|text| {
            text.lines()
                .map(str::trim)
                .rev()
                .find(|line| !line.is_empty())
                .map(str::to_string)
        })
}

/// What the Primary Monitor card says about the selected display. `primary`
/// is the display holding workspace 1 (under the policy, also the taskbar) and
/// `here` says whether the selected display is that one.
pub fn card_text(policy: bool, settable: bool, primary: Option<&str>, here: bool) -> String {
    match (policy, settable, primary, here) {
        (true, true, _, true) => {
            "This is the primary monitor: the taskbar and workspace 1 are on this display.".into()
        }
        (true, true, Some(name), false) => format!(
            "The taskbar and workspace 1 are on {name}. Making this display primary moves them here, and the choice is saved."
        ),
        (true, true, None, false) => {
            "Making this display primary moves the taskbar and workspace 1 here, and the choice is saved.".into()
        }
        (true, false, _, true) => format!(
            "Workspace 1 is on this display. {OLD_OS}"
        ),
        (true, false, Some(name), false) => format!("Workspace 1 is on {name}. {OLD_OS}"),
        (true, false, None, false) => {
            format!("Workspace 1 is not open on any display. {OLD_OS}")
        }
        (false, _, _, true) => "Workspace 1 is on this display.".into(),
        (false, _, Some(name), false) => {
            format!("Workspace 1 is on {name}. Moving it takes effect now and is not saved.")
        }
        (false, _, None, false) => "Workspace 1 is not open on any display.".into(),
    }
}

/// The policy owns placement and this OS can't be asked to change the primary.
const OLD_OS: &str =
    "Choosing the primary monitor here needs a newer smplOS; update it to enable this.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::hyprctl::tests::{failure, reply, MockRunner};

    #[test]
    fn read_returns_the_connector_the_os_names() {
        let runner = MockRunner::new(|program, args| match (program, args) {
            ("workspace-ctl", ["primary"]) => reply("HDMI-A-1\n"),
            _ => Err("unexpected".into()),
        });
        assert_eq!(read(&runner).as_deref(), Some("HDMI-A-1"));
        assert_eq!(runner.calls(), ["workspace-ctl primary"]);
    }

    #[test]
    fn read_tolerates_blank_lines_and_extra_output() {
        for (stdout, expected) in [
            ("DP-3", Some("DP-3")),
            ("\n  eDP-1  \r\n", Some("eDP-1")),
            ("DP-3\nsecond line\n", Some("DP-3")),
            ("", None),
            ("\n\n", None),
            ("HDMI-A-1 (Lenovo)\n", None),
            ("DP\u{7}3\n", None),
        ] {
            let runner = MockRunner::new(move |_, _| reply(stdout));
            assert_eq!(read(&runner).as_deref(), expected, "{stdout:?}");
        }
    }

    #[test]
    fn read_is_none_when_the_os_cannot_say() {
        // Policy off or an older OS: non-zero exit, empty stdout.
        let runner = MockRunner::new(|_, _| failure(""));
        assert_eq!(read(&runner), None);
        // A failing command's stdout is never trusted.
        let runner = MockRunner::new(|_, _| {
            Ok(CommandOutput {
                success: false,
                stdout: "HDMI-A-1\n".into(),
                stderr: String::new(),
            })
        });
        assert_eq!(read(&runner), None);
        // A missing or hung workspace-ctl.
        let runner = MockRunner::new(|_, _| Err("couldn't run workspace-ctl".into()));
        assert_eq!(read(&runner), None);
    }

    #[test]
    fn set_runs_only_workspace_ctl_set_primary() {
        let runner = MockRunner::new(|program, args| match (program, args) {
            ("workspace-ctl", ["set-primary", "DP-3"]) => reply(""),
            _ => Err("unexpected".into()),
        });
        assert_eq!(set(&runner, "DP-3"), Ok(()));
        // The OS moves the taskbar; Settings must not touch the bar or eww.
        assert_eq!(runner.calls(), ["workspace-ctl set-primary DP-3"]);
    }

    #[test]
    fn set_reports_the_os_message() {
        let runner = MockRunner::new(|_, _| failure("workspace-ctl: unknown monitor DP-9\n"));
        assert_eq!(
            set(&runner, "DP-9"),
            Err("workspace-ctl: unknown monitor DP-9".into())
        );
        // Usage blocks and tracebacks end with the actual reason.
        let runner = MockRunner::new(|_, _| {
            failure("usage: workspace-ctl [-h] command\nworkspace-ctl: error: invalid choice: 'set-primary'\n")
        });
        assert_eq!(
            set(&runner, "DP-3"),
            Err("workspace-ctl: error: invalid choice: 'set-primary'".into())
        );
        // Without stderr the stdout line is used, then a generic sentence.
        let runner = MockRunner::new(|_, _| {
            Ok(CommandOutput {
                success: false,
                stdout: "policy is disabled\n".into(),
                stderr: String::new(),
            })
        });
        assert_eq!(set(&runner, "DP-3"), Err("policy is disabled".into()));
        let runner = MockRunner::new(|_, _| failure(""));
        assert_eq!(
            set(&runner, "DP-3"),
            Err("workspace-ctl set-primary failed without a message".into())
        );
    }

    #[test]
    fn set_passes_runner_errors_through() {
        let runner = MockRunner::new(|_, _| Err("workspace-ctl did not finish in time".into()));
        assert_eq!(
            set(&runner, "DP-3"),
            Err("workspace-ctl did not finish in time".into())
        );
    }

    #[test]
    fn set_rejects_names_that_would_parse_as_options() {
        let runner = MockRunner::new(|_, _| reply(""));
        assert!(set(&runner, "").is_err());
        assert!(set(&runner, "--help").is_err());
        assert!(runner.calls().is_empty(), "nothing is run for a bad name");
    }

    #[test]
    fn card_text_tells_where_the_taskbar_and_workspace_one_live() {
        let text = card_text(true, true, Some("HDMI-A-1"), false);
        assert_eq!(
            text,
            "The taskbar and workspace 1 are on HDMI-A-1. Making this display primary moves them here, and the choice is saved."
        );
        assert_eq!(
            card_text(true, true, Some("HDMI-A-1"), true),
            "This is the primary monitor: the taskbar and workspace 1 are on this display."
        );
        assert!(card_text(true, true, None, false).contains("the choice is saved"));
    }

    #[test]
    fn card_text_for_an_os_without_set_primary_is_honest_not_a_dead_end() {
        for here in [true, false] {
            for primary in [Some("DP-3"), None] {
                let text = card_text(true, false, primary, here);
                assert!(text.contains("needs a newer smplOS"), "{text}");
                assert!(!text.contains("Taskbar settings"), "{text}");
            }
        }
        assert!(card_text(true, false, Some("DP-3"), false).starts_with("Workspace 1 is on DP-3."));
    }

    #[test]
    fn card_text_without_the_policy_keeps_the_existing_wording() {
        assert_eq!(
            card_text(false, true, Some("HDMI-A-1"), true),
            "Workspace 1 is on this display."
        );
        assert_eq!(
            card_text(false, true, Some("HDMI-A-1"), false),
            "Workspace 1 is on HDMI-A-1. Moving it takes effect now and is not saved."
        );
        assert_eq!(
            card_text(false, true, None, false),
            "Workspace 1 is not open on any display."
        );
    }

    #[test]
    fn no_card_text_sends_the_user_to_the_taskbar_tab() {
        for policy in [true, false] {
            for settable in [true, false] {
                for primary in [Some("DP-3"), None] {
                    for here in [true, false] {
                        let text = card_text(policy, settable, primary, here);
                        assert!(!text.contains("Taskbar settings"), "{text}");
                    }
                }
            }
        }
    }
}
