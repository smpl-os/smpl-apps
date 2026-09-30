/// Compare literal argv without evaluating shell syntax or expanding variables.
/// Commands outside this subset retain the old exact-string matching behavior.
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
        pinned.push(exec.to_string());
    }
}

fn literal_argv(command: &str) -> Option<Vec<String>> {
    if command.chars().any(|c| c.is_control() && c != '\t') {
        return None;
    }

    let mut args = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => word.push(c),
            (Some('"'), '$' | '`') => return None,
            (Some('"'), '\\') => {
                let escaped = chars.next()?;
                // POSIX double quotes only remove backslashes before these
                // characters (newlines were excluded above).
                if !matches!(escaped, '$' | '`' | '"' | '\\') {
                    word.push('\\');
                }
                word.push(escaped);
            }
            (Some('"'), _) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
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
                word.push(c);
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
    // Assignment prefixes and shell reserved words aren't ordinary argv.
    // Reject their quoted forms too rather than guessing their shell role.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_quoting_preserves_argument_boundaries() {
        for (left, right) in [
            (
                "/home/foo/.local/bin/example",
                "\"/home/foo/.local/bin/example\"",
            ),
            (
                "'/path with spaces/app' --flag",
                "/path\\ with\\ spaces/app '--flag'",
            ),
            ("foo 'two words'", "foo two\\ words"),
            ("foo a\"b\"'c'", "'foo' abc"),
            ("foo '' \"\"", "foo \"\" ''"),
            ("foo 'a|b'", "foo a\\|b"),
            ("foo '$HOME'", "foo \"\\$HOME\""),
            ("foo 'a\\b'", "foo \"a\\b\""),
            ("foo \"a\\\\b\"", "foo a\\\\b"),
            ("foo 'a\"b'", "foo a\\\"b"),
            ("foo \"a'b\"", "foo a\\'b"),
            ("foo '\u{e9} space'", "foo \u{e9}\\ space"),
            ("  foo\targ  ", "\"foo\" arg"),
        ] {
            assert!(equivalent(left, right), "{left:?} != {right:?}");
            assert!(equivalent(right, left), "{right:?} != {left:?}");
        }
    }

    #[test]
    fn real_argument_differences_do_not_match() {
        for (left, right) in [
            ("foo 'two words'", "foo two words"),
            ("foo ''", "foo"),
            ("foo '' ''", "foo ''"),
            ("foo --one", "foo --two"),
            ("foo one two", "foo two one"),
            ("foo --arg=value", "foo --arg value"),
            ("foo \"a\\b\"", "foo ab"),
            ("foo a", "Foo a"),
        ] {
            assert!(!equivalent(left, right), "{left:?} == {right:?}");
        }
    }

    #[test]
    fn shell_constructs_only_match_exactly() {
        for (left, right) in [
            ("foo \"$HOME\"", "foo '$HOME'"),
            ("foo $HOME", "foo '$HOME'"),
            ("foo 'a|b'", "foo a|b"),
            ("foo a&&b", "foo 'a&&b'"),
            ("foo a;b", "foo 'a;b'"),
            ("foo a&b", "foo 'a&b'"),
            ("foo $(bar)", "foo '$(bar)'"),
            ("foo `bar`", "foo '`bar`'"),
            ("foo \"$((1+2))\"", "foo '$((1+2))'"),
            ("foo ${HOME}", "foo '${HOME}'"),
            ("foo ~", "foo '~'"),
            ("foo *.png", "foo '*.png'"),
            ("foo a?", "foo 'a?'"),
            ("foo [ab]", "foo '[ab]'"),
            ("foo {a,b}", "foo '{a,b}'"),
            ("foo >file", "foo '>file'"),
            ("foo <(bar)", "foo '<(bar)'"),
            ("foo # comment", "foo '#' comment"),
            ("foo\nbar", "foo bar"),
            ("foo \\\nbar", "foo bar"),
            ("VAR=value foo", "'VAR=value' foo"),
            ("VAR='value' foo", "VAR=value foo"),
            ("if true", "'if' true"),
            ("foo 'unterminated", "foo unterminated"),
            ("foo trailing\\", "foo trailing"),
            ("foo\0", "foo"),
        ] {
            assert!(!equivalent(left, right), "{left:?} == {right:?}");
            assert!(!equivalent(right, left), "{right:?} == {left:?}");
            assert!(equivalent(left, left), "exact custom command must match");
        }
    }

    #[test]
    fn toggle_keeps_order_and_custom_commands_without_rewriting_them() {
        let original = vec![
            "before --flag".to_string(),
            "/path/example".to_string(),
            "foo \"$HOME\" | bar".to_string(),
            "'/path/example'".to_string(),
            "after".to_string(),
        ];
        let mut pins = original.clone();
        assert!(is_pinned(&pins, "\"/path/example\""));
        assert!(!is_pinned(&pins, "/path/example --different"));
        toggle_pin(&mut pins, "\"/path/example\"");
        assert_eq!(
            pins,
            [
                original[0].clone(),
                original[2].clone(),
                original[4].clone()
            ]
        );
        toggle_pin(&mut pins, "\"/path/example\"");
        assert_eq!(pins.last().unwrap(), "\"/path/example\"");
        assert_eq!(
            &pins[..3],
            &[
                original[0].clone(),
                original[2].clone(),
                original[4].clone()
            ]
        );
    }
}
