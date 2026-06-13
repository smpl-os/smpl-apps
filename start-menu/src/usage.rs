// Per-user app usage tracking for frecency-based ranking.
//
// Stored as TSV at: $XDG_STATE_HOME/smplos/app-usage.tsv
//   (defaults to ~/.local/state/smplos/app-usage.tsv)
//
// Line format: "<count>\t<last_unix_ts>\t<exec>\n"
//
// The frecency score is `count * 0.5^(days_since_last_use / HALF_LIFE_DAYS)`,
// so an app launched 10 times two weeks ago scores the same as one launched
// 5 times today (HALF_LIFE_DAYS = 14). This makes the top match in search
// results converge to what the user actually picks, without permanently
// locking in apps they used once and forgot.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const HALF_LIFE_DAYS: f64 = 14.0;
const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Default, Debug, Clone)]
pub struct Usage {
    // exec → (count, last_unix_ts)
    entries: HashMap<String, (u32, u64)>,
}

fn state_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("smplos").join("app-usage.tsv"))
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Usage {
    pub fn load() -> Self {
        let Some(path) = state_path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Self::default() };
        Self::from_str(&text)
    }

    /// Parse TSV content. Malformed lines are skipped silently so a corrupted
    /// file never prevents the start menu from opening.
    pub fn from_str(text: &str) -> Self {
        let mut entries = HashMap::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\n');
            if line.is_empty() { continue; }
            let mut parts = line.splitn(3, '\t');
            let count = parts.next().and_then(|s| s.parse::<u32>().ok());
            let ts    = parts.next().and_then(|s| s.parse::<u64>().ok());
            let exec  = parts.next().map(str::to_string);
            if let (Some(c), Some(t), Some(e)) = (count, ts, exec) {
                if !e.is_empty() {
                    entries.insert(e, (c, t));
                }
            }
        }
        Self { entries }
    }

    pub fn to_tsv(&self) -> String {
        // Sort by exec so the file is stable on disk (helps debugging / git diffs).
        let mut rows: Vec<(&String, &(u32, u64))> = self.entries.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = String::new();
        for (exec, (count, ts)) in rows {
            s.push_str(&format!("{}\t{}\t{}\n", count, ts, exec));
        }
        s
    }

    pub fn record(&mut self, exec: &str, now: u64) {
        let entry = self.entries.entry(exec.to_string()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = now;
    }

    pub fn save(&self) {
        let Some(path) = state_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Atomic write: tmp + rename, so a crash mid-write can't corrupt the file.
        let tmp = path.with_extension("tsv.tmp");
        if std::fs::write(&tmp, self.to_tsv()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Frecency: launch count weighted by exponential decay since last use.
    /// Returns 0.0 for never-launched apps, so they fall through to the
    /// alphabetical tiebreaker in the caller.
    pub fn score(&self, exec: &str, now: u64) -> f64 {
        let Some(&(count, last)) = self.entries.get(exec) else { return 0.0 };
        if count == 0 { return 0.0 }
        let dt_days = (now.saturating_sub(last) as f64) / SECONDS_PER_DAY;
        let decay = 0.5_f64.powf(dt_days / HALF_LIFE_DAYS);
        count as f64 * decay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_tsv() {
        let mut u = Usage::default();
        u.record("code", 1_000_000);
        u.record("code", 1_000_100);
        u.record("firefox", 1_000_050);
        let parsed = Usage::from_str(&u.to_tsv());
        assert_eq!(parsed.entries.get("code"),    Some(&(2, 1_000_100)));
        assert_eq!(parsed.entries.get("firefox"), Some(&(1, 1_000_050)));
    }

    #[test]
    fn malformed_lines_skipped() {
        let text = "not_a_number\t0\tbad\n\n3\t100\tgood\nonly_one_field\n";
        let u = Usage::from_str(text);
        assert_eq!(u.entries.len(), 1);
        assert_eq!(u.entries.get("good"), Some(&(3, 100)));
    }

    #[test]
    fn score_zero_for_unknown_exec() {
        let u = Usage::default();
        assert_eq!(u.score("never-launched", 1_000_000), 0.0);
    }

    #[test]
    fn recent_use_outscores_old_use_of_same_count() {
        let mut u = Usage::default();
        let now = 10_000_000_u64;
        // "old" was launched 28 days ago (2 half-lives → score *= 0.25)
        let old_ts = now - (28 * 86_400);
        u.entries.insert("old".into(),    (4, old_ts));
        u.entries.insert("recent".into(), (4, now));
        assert!(u.score("recent", now) > u.score("old", now));
    }

    #[test]
    fn higher_count_outscores_lower_count_at_same_time() {
        let mut u = Usage::default();
        let now = 10_000_000_u64;
        u.entries.insert("a".into(), (10, now));
        u.entries.insert("b".into(), (2,  now));
        assert!(u.score("a", now) > u.score("b", now));
    }

    #[test]
    fn record_increments_and_updates_timestamp() {
        let mut u = Usage::default();
        u.record("x", 100);
        u.record("x", 200);
        u.record("x", 300);
        assert_eq!(u.entries.get("x"), Some(&(3, 300)));
    }
}
