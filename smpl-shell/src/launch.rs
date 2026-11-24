use std::ffi::OsStr;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const RESIDENT_SIGNALS: &[libc::c_int] = &[
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGCHLD,
];

pub fn command_with_signals(program: impl AsRef<OsStr>, signals: &[libc::c_int]) -> Command {
    let signals = signals.to_vec();
    let mut command = Command::new(program);
    // SAFETY: only async-signal-safe functions run between fork and exec.
    unsafe {
        command.pre_exec(move || {
            for &signal in &signals {
                if libc::signal(signal, libc::SIG_DFL) == libc::SIG_ERR {
                    return Err(io::Error::last_os_error());
                }
            }
            let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
            libc::sigemptyset(empty.as_mut_ptr());
            match libc::pthread_sigmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut()) {
                0 => Ok(()),
                error => Err(io::Error::from_raw_os_error(error)),
            }
        });
    }
    command
}

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut signals = RESIDENT_SIGNALS.to_vec();
    signals.push(libc::SIGRTMIN());
    command_with_signals(program, &signals)
}

pub fn shell(command: &str) -> Command {
    let mut process = self::command("sh");
    process.arg("-c").arg(command);
    process
}

pub fn literal_argv(command: &str) -> Option<Vec<String>> {
    if command
        .chars()
        .any(|character| character.is_control() && character != '\t')
    {
        return None;
    }
    let mut args = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(character) = chars.next() {
        match (quote, character) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => word.push(character),
            (Some('"'), '$' | '`') => return None,
            (Some('"'), '\\') => {
                let escaped = chars.next()?;
                if !matches!(escaped, '$' | '`' | '"' | '\\') {
                    word.push('\\');
                }
                word.push(escaped);
            }
            (Some('"'), _) => word.push(character),
            (None, '\'' | '"') => {
                quote = Some(character);
                started = true;
            }
            (None, '\\') => {
                word.push(chars.next()?);
                started = true;
            }
            (None, ' ' | '\t') => {
                if started {
                    args.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (
                None,
                '$' | '`' | '|' | '&' | ';' | '<' | '>' | '(' | ')' | '{' | '}' | '*' | '?' | '['
                | ']' | '~' | '#' | '!',
            ) => return None,
            (None, _) => {
                word.push(character);
                started = true;
            }
            _ => unreachable!(),
        }
    }
    if quote.is_some() {
        return None;
    }
    if started {
        args.push(word);
    }
    let executable = args.first()?;
    if executable.is_empty()
        || executable.contains('=')
        || matches!(
            executable.as_str(),
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "while"
                | "until"
                | "for"
                | "in"
                | "function"
                | "select"
                | "time"
                | "coproc"
                | "!"
                | "{"
                | "}"
                | "[["
                | "]]"
        )
    {
        return None;
    }
    Some(args)
}

pub fn equivalent(left: &str, right: &str) -> bool {
    left == right
        || match (literal_argv(left), literal_argv(right)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
}

pub fn is_pinned(pinned: &[String], exec: &str) -> bool {
    pinned.iter().any(|pin| equivalent(pin, exec))
}

pub fn toggle_pin(pinned: &mut Vec<String>, exec: &str) {
    if is_pinned(pinned, exec) {
        pinned.retain(|pin| !equivalent(pin, exec));
    } else {
        pinned.push(exec.to_owned());
    }
}

pub fn bounded_status(mut command: Command, timeout: Duration) -> Option<std::process::ExitStatus> {
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

pub fn close_popups() -> io::Result<()> {
    let mut command = self::command("popup-toggle");
    command
        .arg("--close")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match bounded_status(command, Duration::from_secs(2)) {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(io::Error::other(format!(
            "popup-toggle --close failed: {status}"
        ))),
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "popup-toggle --close timed out",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_argv_preserves_boundaries_but_rejects_shell_expansion() {
        assert_eq!(
            literal_argv(r#"'two words' "a b" plain\ word"#),
            Some(vec![
                "two words".to_owned(),
                "a b".to_owned(),
                "plain word".to_owned()
            ])
        );
        for command in [
            "echo $HOME",
            "a && b",
            "a | b",
            "VAR=x a",
            "$(id)",
            "x `id`",
        ] {
            assert!(literal_argv(command).is_none(), "{command}");
        }
    }

    #[test]
    fn equivalent_quoting_keeps_custom_commands_exact() {
        assert!(equivalent("/path/app", r#""/path/app""#));
        assert!(!equivalent("app one two", "app 'one two'"));
        let custom = "foo \"$HOME\" | bar";
        assert!(equivalent(custom, custom));
        assert!(!equivalent(custom, "foo '$HOME' | bar"));
    }
}
