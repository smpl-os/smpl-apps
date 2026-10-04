//! Bounded command execution and provider-aware Hyprland dispatches.
//!
//! Under the Lua config provider (Hyprland ≥ 0.55) `hyprctl dispatch X` is
//! evaluated as `hl.dispatch(X)`: legacy strings fail with an error reply
//! while hyprctl still exits 0. Every dispatch therefore uses the provider's
//! syntax and only an exact `ok` reply counts as success.

use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs external commands; injected so tests never touch the live desktop.
pub trait Runner: Send + Sync {
    /// `Err` only when the command could not run or timed out.
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String>;
}

/// Real commands, bounded like the Taskbar runner.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String> {
        let output = Command::new("timeout")
            .args(["--signal=TERM", "--kill-after=1s", "8s", program])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("couldn't run {program}: {error}"))?;
        match output.status.code() {
            Some(124) | Some(137) | None => {
                return Err(format!("{program} did not finish in time"))
            }
            Some(125..=127) => return Err(format!("couldn't run {program}")),
            _ => {}
        }
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

/// `hyprctl <args>`; fails on a failed exit status.
pub fn hyprctl(runner: &dyn Runner, args: &[&str]) -> Result<String, String> {
    let output = runner.run("hyprctl", args)?;
    if !output.success {
        let detail = match first_line(&output.stderr) {
            "" => first_line(&output.stdout),
            line => line,
        };
        return Err(format!("hyprctl {} failed: {detail}", args.join(" ")));
    }
    Ok(output.stdout)
}

/// `hyprctl <args>` whose reply must be exactly `ok`.
pub fn hyprctl_ok(runner: &dyn Runner, args: &[&str]) -> Result<(), String> {
    let reply = hyprctl(runner, args)?;
    match reply.trim() {
        "ok" => Ok(()),
        "" => Err(format!("Hyprland gave no reply to {}", args.join(" "))),
        other => Err(format!(
            "Hyprland refused {}: {}",
            args.join(" "),
            first_line(other)
        )),
    }
}

/// Hyprland's config provider, which decides the dispatch syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Lua,
    /// hyprlang, or Hyprland before 0.55 (no `configProvider` line).
    Legacy,
}

/// Reads `configProvider:` from `hyprctl systeminfo`.
pub fn parse_provider(systeminfo: &str) -> Result<Provider, String> {
    for line in systeminfo.lines() {
        if let Some(value) = line.trim().strip_prefix("configProvider:") {
            return match value.trim() {
                "lua" => Ok(Provider::Lua),
                "hyprlang" => Ok(Provider::Legacy),
                other => Err(format!("unsupported Hyprland config provider \"{other}\"")),
            };
        }
    }
    Ok(Provider::Legacy)
}

/// A Lua string literal.
fn lua_string(text: &str) -> String {
    let mut literal = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            c if c.is_control() => literal.push_str(&format!("\\{:03}", c as u32)),
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
}

/// The display actions Settings dispatches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    MoveWorkspaceToMonitor {
        workspace: String,
        monitor: String,
    },
    /// Centers a floating window on its display. Legacy `centerwindow` only
    /// acts on the focused window, so the legacy form moves the window to the
    /// precomputed centered position `x`, `y` instead.
    CenterWindow {
        address: String,
        x: i32,
        y: i32,
    },
}

impl Dispatch {
    pub fn argument(&self, provider: Provider) -> String {
        match (self, provider) {
            (Self::MoveWorkspaceToMonitor { workspace, monitor }, Provider::Lua) => format!(
                "hl.dsp.workspace.move({{workspace={}, monitor={}}})",
                lua_string(workspace),
                lua_string(monitor)
            ),
            (Self::MoveWorkspaceToMonitor { workspace, monitor }, Provider::Legacy) => {
                format!("moveworkspacetomonitor {workspace} {monitor}")
            }
            (Self::CenterWindow { address, .. }, Provider::Lua) => format!(
                "hl.dsp.window.center({{window={}}})",
                lua_string(&format!("address:{address}"))
            ),
            (Self::CenterWindow { address, x, y }, Provider::Legacy) => {
                format!("movewindowpixel exact {x} {y},address:{address}")
            }
        }
    }
}

/// Runs one dispatch in the provider's syntax; only an `ok` reply succeeds.
pub fn dispatch(runner: &dyn Runner, provider: Provider, action: &Dispatch) -> Result<(), String> {
    hyprctl_ok(runner, &["dispatch", &action.argument(provider)])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    type Handler = dyn Fn(&str, &[&str]) -> Result<CommandOutput, String> + Send + Sync;

    /// Records every command; replies come from a closure.
    pub struct MockRunner {
        pub calls: Mutex<Vec<String>>,
        handler: Box<Handler>,
    }

    impl MockRunner {
        pub fn new(
            handler: impl Fn(&str, &[&str]) -> Result<CommandOutput, String> + Send + Sync + 'static,
        ) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                handler: Box::new(handler),
            }
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Runner for MockRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String> {
            self.calls.lock().unwrap().push(
                std::iter::once(program)
                    .chain(args.iter().copied())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            (self.handler)(program, args)
        }
    }

    pub fn reply(stdout: &str) -> Result<CommandOutput, String> {
        Ok(CommandOutput {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    pub fn failure(stderr: &str) -> Result<CommandOutput, String> {
        Ok(CommandOutput {
            success: false,
            stdout: String::new(),
            stderr: stderr.into(),
        })
    }

    #[test]
    fn provider_comes_from_systeminfo() {
        assert_eq!(
            parse_provider("State:\n\nconfigProvider: lua\nbackend: drm\n"),
            Ok(Provider::Lua)
        );
        assert_eq!(
            parse_provider("configProvider: hyprlang"),
            Ok(Provider::Legacy)
        );
        assert_eq!(parse_provider("Hyprland 0.54.0\n"), Ok(Provider::Legacy));
        assert!(parse_provider("configProvider: toml").is_err());
    }

    #[test]
    fn dispatch_strings_follow_the_provider() {
        let workspace = Dispatch::MoveWorkspaceToMonitor {
            workspace: "1".into(),
            monitor: "DP-3".into(),
        };
        assert_eq!(
            workspace.argument(Provider::Lua),
            r#"hl.dsp.workspace.move({workspace="1", monitor="DP-3"})"#
        );
        assert_eq!(
            workspace.argument(Provider::Legacy),
            "moveworkspacetomonitor 1 DP-3"
        );
        let center = Dispatch::CenterWindow {
            address: "0x55aa".into(),
            x: 100,
            y: 200,
        };
        assert_eq!(
            center.argument(Provider::Lua),
            r#"hl.dsp.window.center({window="address:0x55aa"})"#
        );
        assert_eq!(
            center.argument(Provider::Legacy),
            "movewindowpixel exact 100 200,address:0x55aa"
        );
        assert_eq!(lua_string("a\"b\\c\u{1}"), "\"a\\\"b\\\\c\\001\"");
    }

    #[test]
    fn only_an_exact_ok_reply_counts_as_success() {
        let action = Dispatch::MoveWorkspaceToMonitor {
            workspace: "1".into(),
            monitor: "DP-3".into(),
        };
        let ok = MockRunner::new(|_, _| reply("ok\n"));
        assert_eq!(dispatch(&ok, Provider::Lua, &action), Ok(()));
        assert_eq!(
            ok.calls(),
            [r#"hyprctl dispatch hl.dsp.workspace.move({workspace="1", monitor="DP-3"})"#]
        );
        // Hyprland exits 0 but replies with an error for legacy syntax under Lua.
        let refused = MockRunner::new(|_, _| {
            reply("[string \"return hl.dispatch(moveworkspacetomonitor 1 DP-3)\"]:1: ')' expected\n\n → Note: dispatch in lua")
        });
        let error = dispatch(&refused, Provider::Legacy, &action).unwrap_err();
        assert!(error.contains("')' expected"), "{error}");
        assert!(dispatch(&MockRunner::new(|_, _| reply("")), Provider::Lua, &action).is_err());
        assert!(dispatch(
            &MockRunner::new(|_, _| reply("okay")),
            Provider::Lua,
            &action
        )
        .is_err());
        assert!(dispatch(
            &MockRunner::new(|_, _| failure("Couldn't connect (4)")),
            Provider::Lua,
            &action
        )
        .is_err());
        assert!(dispatch(
            &MockRunner::new(|_, _| Err("couldn't run hyprctl".into())),
            Provider::Lua,
            &action
        )
        .is_err());
    }

    #[test]
    fn system_runner_reports_missing_commands_and_exit_status() {
        assert!(SystemRunner
            .run("/nonexistent/settings-display-test", &[])
            .is_err());
        let failed = SystemRunner
            .run("sh", &["-c", "printf out; printf err >&2; exit 3"])
            .unwrap();
        assert_eq!(
            failed,
            CommandOutput {
                success: false,
                stdout: "out".into(),
                stderr: "err".into()
            }
        );
        assert!(SystemRunner.run("sh", &["-c", "exit 0"]).unwrap().success);
    }
}
