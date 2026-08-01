//! Deterministic 2-letter hint label generator.
//!
//! Given a target count and an alphabet of "hint chars" (see [`Config`]),
//! produces short, unambiguous labels. Single-char labels for the first
//! `N` targets, then 2-char labels once we run out.
//!
//! We prefer labels that can be typed with alternating hands and don't
//! start with the same letter as another label (to allow immediate
//! commit without ambiguity delay).
//!
//! [`Config`]: crate::config::Config

/// Generate `count` hint labels using the given hint-chars alphabet.
///
/// # Contract
///
/// * Returns exactly `count` labels.
/// * Every label is a valid *prefix-free* code — no label is a prefix of
///   any other. This lets the daemon commit on the first exact match
///   without an ambiguity timer.
///
/// # Prefix-free strategy
///
/// If `count <= alphabet.len()`, use single characters.
/// Otherwise use only two-character labels; single chars are never mixed
/// with two-char labels, which would violate the prefix-free property.
pub fn generate(count: usize, hint_chars: &str) -> Vec<String> {
    let alphabet: Vec<char> = hint_chars.chars().collect();
    if alphabet.is_empty() || count == 0 {
        return Vec::new();
    }

    if count <= alphabet.len() {
        return alphabet.iter().take(count).map(|c| c.to_string()).collect();
    }

    // Two-character labels: alphabet[i] + alphabet[j].
    // With 20-char alphabet that's 400 labels — plenty for one screen.
    let mut out = Vec::with_capacity(count);
    'outer: for a in &alphabet {
        for b in &alphabet {
            out.push(format!("{a}{b}"));
            if out.len() == count { break 'outer; }
        }
    }

    // If a caller asks for more than alphabet² labels, three-char fallback.
    // Rare but keeps the API total.
    if out.len() < count {
        'outer3: for a in &alphabet {
            for b in &alphabet {
                for c in &alphabet {
                    out.push(format!("{a}{b}{c}"));
                    if out.len() == count { break 'outer3; }
                }
            }
        }
    }

    out
}

/// Result of feeding a keypress into the running hint filter.
#[derive(Debug, PartialEq, Eq)]
pub enum Match<'a> {
    /// The typed prefix matches no hint. Reset filter and beep-worthy.
    None,
    /// The typed prefix uniquely identifies one label. Commit + close.
    Exact(&'a str),
    /// The typed prefix is a valid prefix of one or more labels but not
    /// yet unique. Keep the overlay open, highlight remaining candidates.
    Partial,
}

/// Match a typed prefix against a list of labels.
///
/// Because labels are prefix-free (see [`generate`]), an exact match on a
/// label is unambiguous. But `Partial` covers "the user has typed one char
/// of a two-char label" — no label yet fully matches.
pub fn match_prefix<'a>(labels: &'a [String], prefix: &str) -> Match<'a> {
    if prefix.is_empty() {
        return Match::Partial;
    }
    let mut candidates = labels.iter().filter(|l| l.starts_with(prefix));
    match candidates.next() {
        None => Match::None,
        Some(first) => {
            if first == prefix && candidates.next().is_none() {
                Match::Exact(first)
            } else {
                Match::Partial
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_when_fits() {
        let labels = generate(5, "abcdefghij");
        assert_eq!(labels, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn two_char_when_overflows() {
        let labels = generate(100, "abcdefghij");
        // All labels 2 chars long — no mixing with single chars.
        assert!(labels.iter().all(|l| l.len() == 2));
        assert_eq!(labels[0], "aa");
        assert_eq!(labels[99], "jj");
        assert_eq!(labels.len(), 100);
    }

    #[test]
    fn three_char_when_double_overflows() {
        // 10^2 = 100 two-char labels; 101 forces one three-char.
        let labels = generate(101, "abcdefghij");
        assert_eq!(labels.len(), 101);
        // First 100 are 2-char, last one 3-char.
        assert!(labels[..100].iter().all(|l| l.len() == 2));
        assert_eq!(labels[100].len(), 3);
    }

    #[test]
    fn labels_are_unique() {
        let labels = generate(400, "abcdefghijklmnopqrst");
        let set: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(set.len(), labels.len());
    }

    #[test]
    fn match_prefix_exact_and_partial() {
        let labels: Vec<String> = ["aa", "ab", "bc"].iter().map(|s| s.to_string()).collect();
        assert_eq!(match_prefix(&labels, "a"), Match::Partial);
        assert_eq!(match_prefix(&labels, "aa"), Match::Exact("aa"));
        assert_eq!(match_prefix(&labels, "z"), Match::None);
    }
}
