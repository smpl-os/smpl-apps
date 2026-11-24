use serde_json::{json, Map, Value};
use std::fs::{OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone)]
pub struct Telemetry {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    started: Instant,
    pid: u32,
    sink: Mutex<Option<std::fs::File>>,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::from_env("smpl-shell")
    }
}

impl Telemetry {
    pub fn from_env(service: &str) -> Self {
        let enabled = std::env::var("SMPL_BAR_TELEMETRY")
            .ok()
            .is_some_and(|value| !matches!(value.as_str(), "" | "0" | "false" | "off"));
        if !enabled {
            return Self { inner: None };
        }
        let sink = std::env::var_os("SMPL_BAR_TELEMETRY_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .and_then(|path| {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match OpenOptions::new().create(true).append(true).open(&path) {
                    Ok(file) => {
                        let _ = std::fs::set_permissions(&path, Permissions::from_mode(0o600));
                        Some(file)
                    }
                    Err(error) => {
                        eprintln!(
                            "{service}: cannot open telemetry sink {}: {error}",
                            path.display()
                        );
                        None
                    }
                }
            });
        let telemetry = Self {
            inner: Some(Arc::new(Inner {
                started: Instant::now(),
                pid: std::process::id(),
                sink: Mutex::new(sink),
            })),
        };
        telemetry.emit("process_start", []);
        telemetry
    }

    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    pub fn now(&self) -> Option<Instant> {
        self.inner.as_ref().map(|_| Instant::now())
    }

    pub fn emit<I>(&self, event: &str, fields: I)
    where
        I: IntoIterator<Item = (&'static str, Value)>,
    {
        let Some(inner) = &self.inner else {
            return;
        };
        let mut object = Map::new();
        object.insert("service".to_owned(), json!("smpl-bar"));
        object.insert("event".to_owned(), json!(event));
        object.insert("pid".to_owned(), json!(inner.pid));
        object.insert(
            "elapsed_us".to_owned(),
            json!(inner.started.elapsed().as_micros() as u64),
        );
        for (key, value) in fields {
            object.insert(key.to_owned(), value);
        }
        let line = match serde_json::to_string(&Value::Object(object)) {
            Ok(mut line) => {
                line.push('\n');
                line
            }
            Err(_) => return,
        };
        if let Ok(mut sink) = inner.sink.lock() {
            if let Some(file) = sink.as_mut() {
                if file.write_all(line.as_bytes()).is_ok() {
                    let _ = file.flush();
                }
            } else {
                eprint!("{line}");
            }
        }
    }

    pub fn duration_since(&self, started: Instant, event: &str, fields: &[(&'static str, Value)]) {
        let elapsed = started.elapsed().as_micros() as u64;
        let mut values = vec![("duration_us", json!(elapsed))];
        values.extend_from_slice(fields);
        self.emit(event, values);
    }

    pub fn emit_process_sample(&self, reason: &str) {
        if !self.enabled() {
            return;
        }
        let Some(sample) = ProcessSample::read() else {
            return;
        };
        self.emit(
            "process_sample",
            [
                ("reason", json!(reason)),
                ("rss_kib", json!(sample.rss_kib)),
                ("pss_kib", json!(sample.pss_kib)),
                ("hwm_kib", json!(sample.hwm_kib)),
                ("threads", json!(sample.threads)),
            ],
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSample {
    pub rss_kib: u64,
    pub pss_kib: u64,
    pub hwm_kib: u64,
    pub threads: u64,
}

impl ProcessSample {
    pub fn read() -> Option<Self> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let mut sample = Self {
            rss_kib: 0,
            pss_kib: 0,
            hwm_kib: 0,
            threads: 0,
        };
        for line in status.lines() {
            let (key, value) = line.split_once(':')?;
            let numeric = value
                .split_whitespace()
                .next()
                .and_then(|number| number.parse().ok())
                .unwrap_or(0);
            match key {
                "VmRSS" => sample.rss_kib = numeric,
                "VmHWM" => sample.hwm_kib = numeric,
                "Threads" => sample.threads = numeric,
                _ => {}
            }
        }
        if let Ok(rollup) = std::fs::read_to_string("/proc/self/smaps_rollup") {
            sample.pss_kib = rollup
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    (key == "Pss").then(|| {
                        value
                            .split_whitespace()
                            .next()
                            .and_then(|number| number.parse().ok())
                            .unwrap_or(0)
                    })
                })
                .unwrap_or(0);
        }
        Some(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_telemetry_has_no_sink_and_zero_overhead_events() {
        std::env::remove_var("SMPL_BAR_TELEMETRY");
        let telemetry = Telemetry::from_env("smpl-bar-test");
        assert!(!telemetry.enabled());
        telemetry.emit("ignored", [("value", json!(1))]);
    }

    #[test]
    fn process_sample_reads_the_live_contract() {
        let sample = ProcessSample::read().unwrap();
        assert!(sample.rss_kib > 0);
        assert!(sample.hwm_kib >= sample.rss_kib);
        assert!(sample.threads >= 1);
    }
}
