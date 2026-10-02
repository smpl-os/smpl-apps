//! Explicit city selection and offline, DST-aware world clocks.
//!
//! Preferences live in `$XDG_CONFIG_HOME/smplos/calendar.json`, with the
//! platform config-directory fallback. Loading preferences never performs a
//! network request. Only an explicit `search` contacts Open-Meteo geocoding.

use crate::weather::{Config, TemperatureUnit};
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

const ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
const MAX_BYTES: u64 = 128 * 1024;
const MAX_CITIES: usize = 3;
const MAX_SEARCH_RESULTS: usize = 100;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
static WRITE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct City {
    pub name: String,
    /// Name plus available region/country, suitable for disambiguating results.
    pub label: String,
    pub latitude: f64,
    pub longitude: f64,
    /// An IANA identifier, not a fixed offset, so world clocks follow DST.
    pub timezone: String,
}

impl City {
    pub fn validate(&self) -> Result<()> {
        validate_text(&self.name, 80, "City name")?;
        Config {
            latitude: self.latitude,
            longitude: self.longitude,
            location: self.label.clone(),
            timezone: self.timezone.clone(),
            temperature_unit: TemperatureUnit::Celsius,
        }
        .validate()?;
        self.timezone
            .parse::<Tz>()
            .context("City timezone must be a known IANA timezone")?;
        Ok(())
    }

    pub fn local_datetime(&self, now: DateTime<Utc>) -> Result<DateTime<Tz>> {
        let timezone = self
            .timezone
            .parse::<Tz>()
            .context("City timezone must be a known IANA timezone")?;
        Ok(now.with_timezone(&timezone))
    }

    fn same_location(&self, other: &Self) -> bool {
        // Provider rounding or an edited display label must not add a second
        // clock for the same location. Same-named distant cities remain valid.
        (self.latitude - other.latitude).abs() < 0.001
            && (self.longitude - other.longitude).abs() < 0.001
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub cities: Vec<City>,
    /// Index of the weather city. Empty settings always use zero.
    pub primary: usize,
    pub temperature_unit: TemperatureUnit,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cities: Vec::new(),
            primary: 0,
            temperature_unit: TemperatureUnit::Celsius,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.cities.len() <= MAX_CITIES,
            "Select at most three cities"
        );
        ensure!(
            (self.cities.is_empty() && self.primary == 0) || self.primary < self.cities.len(),
            "Primary weather city index is invalid"
        );
        for (index, city) in self.cities.iter().enumerate() {
            city.validate()?;
            ensure!(
                !self.cities[..index]
                    .iter()
                    .any(|existing| existing.same_location(city)),
                "City is already selected"
            );
        }
        Ok(())
    }

    /// Mutators validate before changing anything, including on failure.
    pub fn add_city(&mut self, city: City) -> Result<()> {
        self.validate()?;
        city.validate()?;
        ensure!(
            self.cities.len() < MAX_CITIES,
            "Select at most three cities"
        );
        ensure!(
            !self
                .cities
                .iter()
                .any(|existing| existing.same_location(&city)),
            "City is already selected"
        );
        self.cities.push(city);
        Ok(())
    }

    pub fn remove_city(&mut self, index: usize) -> Result<()> {
        self.validate()?;
        ensure!(index < self.cities.len(), "City index is invalid");
        self.cities.remove(index);
        if self.cities.is_empty() {
            self.primary = 0;
        } else if index < self.primary {
            self.primary -= 1;
        } else if self.primary >= self.cities.len() {
            self.primary = self.cities.len() - 1;
        }
        Ok(())
    }

    pub fn set_primary(&mut self, index: usize) -> Result<()> {
        self.validate()?;
        ensure!(index < self.cities.len(), "City index is invalid");
        self.primary = index;
        Ok(())
    }
}

fn preferences_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Could not determine calendar preferences directory")?
        .join("smplos/calendar.json"))
}

/// Absence is distinct from invalid/unreadable preferences; never overwrites
/// corrupt preferences with defaults.
pub fn load_settings() -> Result<Option<Settings>> {
    load_settings_at(&preferences_path()?)
}

fn load_settings_at(path: &Path) -> Result<Option<Settings>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Could not read calendar preferences"),
    };
    let settings: Settings =
        serde_json::from_slice(&read_bounded(file)?).context("Invalid calendar preferences")?;
    settings.validate()?;
    Ok(Some(settings))
}

/// Validates before writing, then atomically replaces the complete settings.
pub fn save_settings(settings: &Settings) -> Result<()> {
    save_settings_at(settings, &preferences_path()?)
}

fn save_settings_at(settings: &Settings, path: &Path) -> Result<()> {
    settings.validate()?;
    let bytes = serde_json::to_vec_pretty(settings)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Calendar preferences are too large"
    );
    let parent = path
        .parent()
        .context("Calendar preferences have no parent directory")?;
    fs::create_dir_all(parent).context("Could not create calendar preferences directory")?;
    let staging = parent.join(format!(
        ".calendar-settings.{}.{}.json",
        std::process::id(),
        WRITE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&staging)
        .context("Could not create calendar preferences staging file")?;
    let result = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result.context("Could not save calendar preferences")
}

#[derive(Debug)]
pub struct SearchResults {
    pub cities: Vec<City>,
    pub suggested: bool,
    pub warning: Option<String>,
}

/// Explicit search: one original query and at most two prefix candidate
/// requests, sharing a ten-second deadline. Only provider records are returned.
pub fn search(query: &str, cancelled: &AtomicBool) -> Result<SearchResults> {
    let deadline = Instant::now() + SEARCH_TIMEOUT;
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    search_with(query, cancelled, |query| {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("City search timed out")?;
        let response = agent
            .get(ENDPOINT)
            .timeout(remaining)
            .query("name", query)
            .query("count", "100")
            .query("language", "en")
            .query("format", "json")
            .call()
            .context("Could not search Open-Meteo cities")?;
        ensure!(
            response.status() == 200,
            "Unexpected geocoding response status"
        );
        parse_response(&read_bounded(response.into_reader())?)
    })
}

fn search_with(
    query: &str,
    cancelled: &AtomicBool,
    mut fetch: impl FnMut(&str) -> Result<Vec<City>>,
) -> Result<SearchResults> {
    let query = query.trim();
    validate_query(query)?;
    ensure!(!cancelled.load(Ordering::Relaxed), "City search cancelled");
    let mut cities = fetch(query)?;
    let location = query.split(',').next().unwrap_or(query).trim();
    let key = match_key(location);
    let mut suggested = false;
    let mut warning = None;
    let exact = cities.iter().any(|city| match_key(&city.name) == key);
    if !exact && key.chars().count() >= 5 {
        let distance_limit = if key.chars().count() >= 8 { 2 } else { 1 };
        for candidate in prefix_queries(query) {
            ensure!(!cancelled.load(Ordering::Relaxed), "City search cancelled");
            let found = match fetch(&candidate) {
                Ok(found) => found,
                Err(error) if !cities.is_empty() => {
                    warning = Some(format!("Broader search unavailable: {error:#}"));
                    break;
                }
                Err(error) => return Err(error),
            };
            for city in found {
                if edit_distance(&key, &match_key(&city.name)) <= distance_limit
                    && !cities.iter().any(|existing| existing.same_location(&city))
                {
                    cities.push(city);
                    suggested = true;
                }
            }
            if suggested {
                break;
            }
        }
    }
    ensure!(!cancelled.load(Ordering::Relaxed), "City search cancelled");
    // Stable sorting retains provider ordering for equally close, distinct places.
    cities.sort_by_key(|city| edit_distance(&key, &match_key(&city.name)));
    cities.truncate(10);
    Ok(SearchResults {
        cities,
        suggested,
        warning,
    })
}

fn fold_latin(value: &str) -> String {
    let mut latin = false;
    value
        .nfd()
        .filter(|&c| {
            if is_combining_mark(c) {
                return !latin;
            }
            latin = c.is_ascii_alphabetic();
            true
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn match_key(value: &str) -> String {
    fold_latin(value)
        .chars()
        .filter(|c| c.is_alphanumeric() || is_combining_mark(*c))
        .collect()
}

fn prefix_queries(query: &str) -> Vec<String> {
    let (location, qualifier) = query
        .split_once(',')
        .map_or((query, ""), |(name, qualifier)| (name, qualifier));
    let folded = fold_latin(location.trim());
    let words: Vec<_> = folded
        .split(|c: char| c.is_whitespace() || c == '-')
        .filter(|word| !word.is_empty())
        .collect();
    let Some(last) = words.last() else {
        return vec![];
    };
    let stem = words[..words.len() - 1].join("-");
    let mut candidates = Vec::new();
    for count in [5, 3] {
        let suffix: String = last.chars().take(count).collect();
        let name = if stem.is_empty() {
            suffix
        } else {
            format!("{stem}-{suffix}")
        };
        let candidate = if qualifier.is_empty() {
            name
        } else {
            format!("{name},{}", qualifier)
        };
        if candidate != fold_latin(query) && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// Optimal-string-alignment distance: insertion, deletion, substitution and
/// adjacent transposition. Inputs are bounded city names, not arbitrary text.
fn edit_distance(left: &str, right: &str) -> usize {
    let a: Vec<_> = left.chars().collect();
    let b: Vec<_> = right.chars().collect();
    let mut previous_previous = vec![0; b.len() + 1];
    let mut previous: Vec<_> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut current = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                current[j] = current[j].min(previous_previous[j - 2] + 1);
            }
        }
        previous_previous = previous;
        previous = current;
    }
    previous[b.len()]
}

pub(crate) fn validate_query(query: &str) -> Result<()> {
    validate_text(query, 100, "City search")?;
    ensure!(
        query
            .split(',')
            .next()
            .unwrap_or(query)
            .trim()
            .chars()
            .count()
            >= 2,
        "Enter at least two characters to search"
    );
    Ok(())
}

fn validate_text(value: &str, limit: usize, field: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty()
            && value.chars().count() <= limit
            && !value.chars().any(char::is_control),
        "{field} must contain 1–{limit} characters and no control characters"
    );
    Ok(())
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "City data exceeds 128 KiB");
    Ok(bytes)
}

#[derive(Deserialize)]
struct SearchResponse {
    generationtime_ms: f64,
    results: Option<Vec<SearchCity>>,
}

#[derive(Deserialize)]
struct SearchCity {
    name: String,
    latitude: f64,
    longitude: f64,
    timezone: String,
    admin1: Option<String>,
    country: Option<String>,
}

fn parse_response(bytes: &[u8]) -> Result<Vec<City>> {
    let response: SearchResponse =
        serde_json::from_slice(bytes).context("Invalid Open-Meteo geocoding response")?;
    ensure!(
        response.generationtime_ms.is_finite() && response.generationtime_ms >= 0.0,
        "Invalid geocoding response metadata"
    );
    let results = response.results.unwrap_or_default();
    ensure!(
        results.len() <= MAX_SEARCH_RESULTS,
        "Geocoding returned too many results"
    );
    let mut cities: Vec<City> = Vec::with_capacity(results.len());
    for result in results {
        let mut components = vec![result.name.clone()];
        for component in [result.admin1, result.country].into_iter().flatten() {
            validate_text(&component, 80, "City region/country")?;
            if !components
                .iter()
                .any(|part| part.eq_ignore_ascii_case(&component))
            {
                components.push(component);
            }
        }
        let city = City {
            name: result.name,
            label: components.join(", "),
            latitude: result.latitude,
            longitude: result.longitude,
            timezone: result.timezone,
        };
        city.validate()?;
        if !cities.iter().any(|existing| existing.same_location(&city)) {
            cities.push(city);
        }
    }
    Ok(cities)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    // Selected public Open-Meteo records, captured 2026-10-02; no user location.
    fn geocoding_fixture() -> Vec<City> {
        parse_response(include_bytes!("../tests/geocoding-fixture.json")).unwrap()
    }

    #[test]
    fn normalized_distance_handles_latin_accents_and_typical_edits() {
        assert_eq!(match_key("Saint-Étienne"), match_key("SAINT ETIENNE"));
        assert_eq!(match_key("Évry"), match_key("Evry"));
        assert_eq!(fold_latin("पुणे"), "पुणे");
        for typo in ["Lafayete", "Laffayette", "Lafyaette", "Lafayettte"] {
            assert_eq!(edit_distance(&match_key(typo), "lafayette"), 1);
        }
        assert_eq!(edit_distance("bengalru", "bengaluru"), 1);
        assert_eq!(edit_distance("thiruvanathapuram", "thiruvananthapuram"), 1);
        assert!(edit_distance("london", "lafayette") > 2);
    }

    #[test]
    fn typo_retrieval_uses_real_records_not_just_resorted_exact_results() {
        let fixture = geocoding_fixture();
        for (query, expected, calls) in [
            ("Lafayete", 0, vec!["Lafayete", "lafay"]),
            ("Laffayette", 0, vec!["Laffayette", "laffa", "laf"]),
            ("Lafyaette", 0, vec!["Lafyaette", "lafya", "laf"]),
            ("Saint Étiene", 2, vec!["Saint Étiene", "saint-etien"]),
            ("Bengalru", 3, vec!["Bengalru", "benga"]),
            ("Thiruvanathapuram", 4, vec!["Thiruvanathapuram", "thiru"]),
            ("Puduchery", 5, vec!["Puduchery", "puduc"]),
        ] {
            let mut requested = Vec::new();
            let result = search_with(query, &AtomicBool::new(false), |candidate| {
                requested.push(candidate.to_string());
                if candidate == *calls.last().unwrap() {
                    Ok(fixture.clone())
                } else {
                    Ok(vec![])
                }
            })
            .unwrap();
            assert_eq!(requested, calls, "{query}");
            assert!(result.suggested, "{query}");
            assert!(result.warning.is_none());
            assert!(result.cities.contains(&fixture[expected]), "{query}");
            assert!(result.cities.iter().all(|city| fixture.contains(city)));
            assert!(requested.len() <= 3);
        }
    }

    #[test]
    fn exact_matches_rank_first_and_geographic_qualifiers_are_preserved() {
        let fixture = geocoding_fixture();
        let mut calls = 0;
        let exact = search_with("Lafayette", &AtomicBool::new(false), |_| {
            calls += 1;
            Ok(vec![
                fixture[3].clone(),
                fixture[1].clone(),
                fixture[0].clone(),
            ])
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(!exact.suggested);
        assert_eq!(exact.cities[..2], [fixture[1].clone(), fixture[0].clone()]);
        assert_ne!(exact.cities[0].label, exact.cities[1].label);
        let mut requests = vec![];
        let selected = search_with("Lafayete, Louisiana", &AtomicBool::new(false), |query| {
            requests.push(query.to_string());
            Ok(if query == "lafay, Louisiana" {
                vec![fixture[0].clone()]
            } else {
                vec![]
            })
        })
        .unwrap();
        assert_eq!(requests, ["Lafayete, Louisiana", "lafay, Louisiana"]);
        assert_eq!(selected.cities, vec![fixture[0].clone()]);
        assert_eq!(selected.cities[0].timezone, "America/Chicago");
    }

    #[test]
    fn search_is_bounded_cancellable_and_reports_partial_failures() {
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        assert!(search_with("Lafayete", &cancelled, |_| {
            calls += 1;
            cancelled.store(true, Ordering::Relaxed);
            Ok(vec![])
        })
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
        assert_eq!(calls, 1);
        let mut calls = 0;
        let result = search_with("Nonesuchplace", &AtomicBool::new(false), |_| {
            calls += 1;
            Ok(vec![])
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert!(result.cities.is_empty());
        assert!(search_with("Lafayete", &AtomicBool::new(false), |_| {
            anyhow::bail!("offline")
        })
        .is_err());
        let fixture = geocoding_fixture();
        let partial = search_with("Bangalore", &AtomicBool::new(false), |query| {
            if query == "Bangalore" {
                Ok(vec![fixture[3].clone()])
            } else {
                anyhow::bail!("offline");
            }
        })
        .unwrap();
        assert_eq!(partial.cities, vec![fixture[3].clone()]);
        assert!(partial.warning.unwrap().contains("offline"));
        assert!(search_with(&"a".repeat(101), &AtomicBool::new(false), |_| {
            panic!("Invalid queries must not contact provider")
        })
        .is_err());
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/city-test-fixtures")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    WRITE_ID.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.join("calendar.json")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn city() -> City {
        City {
            name: "New York".into(),
            label: "New York, United States".into(),
            latitude: 40.71,
            longitude: -74.0,
            timezone: "America/New_York".into(),
        }
    }

    fn settings() -> Settings {
        Settings {
            cities: vec![city()],
            ..Settings::default()
        }
    }

    fn search_response() -> Value {
        json!({
            "generationtime_ms": 0.5,
            "results": [{
                "name": "New York", "latitude": 40.71, "longitude": -74.0,
                "timezone": "America/New_York", "admin1": "New York",
                "country": "United States"
            }]
        })
    }

    #[test]
    fn settings_limit_duplicates_and_mutators_preserve_valid_state() {
        let mut settings = Settings::default();
        assert!(settings.validate().is_ok());
        settings.add_city(city()).unwrap();
        let duplicate = City {
            label: "Alternate label".into(),
            latitude: 40.7101,
            ..city()
        };
        assert!(settings.add_city(duplicate).is_err());
        for latitude in [41.0, 42.0] {
            settings.add_city(City { latitude, ..city() }).unwrap();
        }
        assert!(settings
            .add_city(City {
                latitude: 43.0,
                ..city()
            })
            .is_err());
        assert_eq!(settings.cities.len(), 3);
        settings.set_primary(2).unwrap();
        assert!(settings.set_primary(3).is_err());
        assert!(settings.remove_city(3).is_err());
        assert_eq!(settings.primary, 2);
        settings.remove_city(0).unwrap();
        assert_eq!(settings.primary, 1);
        assert_eq!(settings.cities[settings.primary].latitude, 42.0);
        settings.remove_city(1).unwrap();
        assert_eq!(settings.primary, 0);
        settings.remove_city(0).unwrap();
        assert_eq!(settings, Settings::default());
        assert!(settings.set_primary(0).is_err());

        let mut middle = Settings {
            cities: [40.0, 41.0, 42.0]
                .map(|latitude| City { latitude, ..city() })
                .to_vec(),
            primary: 1,
            ..Settings::default()
        };
        middle.remove_city(1).unwrap();
        assert_eq!(middle.primary, 1);
        assert_eq!(middle.cities[middle.primary].latitude, 42.0);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        for invalid in [
            Settings {
                cities: vec![city(); 4],
                ..Settings::default()
            },
            Settings {
                cities: vec![city(); 2],
                ..Settings::default()
            },
            Settings {
                primary: 1,
                ..settings()
            },
            Settings {
                primary: 1,
                ..Settings::default()
            },
        ] {
            assert!(invalid.validate().is_err());
        }
        for invalid in [
            City {
                timezone: "Not/A_Zone".into(),
                ..city()
            },
            City {
                timezone: "+02:00".into(),
                ..city()
            },
            City {
                latitude: f64::NAN,
                ..city()
            },
            City {
                longitude: 181.0,
                ..city()
            },
            City {
                name: "x".repeat(81),
                ..city()
            },
            City {
                name: "\n".into(),
                ..city()
            },
            City {
                label: "".into(),
                ..city()
            },
        ] {
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn city_labels_obey_weather_location_limit() {
        let accepted = City {
            label: "é".repeat(80),
            ..city()
        };
        accepted.validate().unwrap();
        Config {
            latitude: accepted.latitude,
            longitude: accepted.longitude,
            location: accepted.label,
            timezone: accepted.timezone,
            temperature_unit: TemperatureUnit::Celsius,
        }
        .validate()
        .unwrap();
        assert!(City {
            label: "é".repeat(81),
            ..city()
        }
        .validate()
        .is_err());

        let mut response = search_response();
        response["results"][0]["name"] = json!("a".repeat(50));
        response["results"][0]["admin1"] = json!("b".repeat(40));
        assert!(parse_response(&serde_json::to_vec(&response).unwrap()).is_err());
    }

    #[test]
    fn iana_clock_follows_spring_and_autumn_dst_transitions() {
        for (utc, expected) in [
            ("2026-03-08T06:59:00Z", "2026-03-08 01:59 -0500"),
            ("2026-03-08T07:01:00Z", "2026-03-08 03:01 -0400"),
            ("2026-11-01T05:30:00Z", "2026-11-01 01:30 -0400"),
            ("2026-11-01T06:30:00Z", "2026-11-01 01:30 -0500"),
        ] {
            let local = city().local_datetime(utc.parse().unwrap()).unwrap();
            assert_eq!(local.format("%Y-%m-%d %H:%M %z").to_string(), expected);
        }
        let india = City {
            timezone: "Asia/Kolkata".into(),
            ..city()
        };
        let local = india
            .local_datetime("2026-10-02T20:00:00Z".parse().unwrap())
            .unwrap();
        assert_eq!(
            local.format("%Y-%m-%d %H:%M").to_string(),
            "2026-10-03 01:30"
        );
    }

    #[test]
    fn atomic_preferences_roundtrip_and_invalid_save_preserves_old_file() {
        let fixture = Fixture::new();
        assert!(load_settings_at(&fixture.path()).unwrap().is_none());
        save_settings_at(&settings(), &fixture.path()).unwrap();
        assert_eq!(load_settings_at(&fixture.path()).unwrap(), Some(settings()));
        let original = fs::read(fixture.path()).unwrap();
        assert!(save_settings_at(
            &Settings {
                primary: 3,
                ..settings()
            },
            &fixture.path()
        )
        .is_err());
        assert_eq!(fs::read(fixture.path()).unwrap(), original);
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            assert_eq!(
                fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let linked = fixture.0.join("linked-settings.json");
            symlink(fixture.path(), &linked).unwrap();
            save_settings_at(&Settings::default(), &linked).unwrap();
            assert!(!fs::symlink_metadata(&linked)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(fixture.path()).unwrap(), original);
        }
        let fahrenheit = Settings {
            temperature_unit: TemperatureUnit::Fahrenheit,
            ..settings()
        };
        save_settings_at(&fahrenheit, &fixture.path()).unwrap();
        assert_eq!(load_settings_at(&fixture.path()).unwrap(), Some(fahrenheit));
        for bytes in [
            "{",
            "{}",
            r#"{"cities":[],"primary":0,"temperature_unit":"kelvin"}"#,
        ] {
            fs::write(fixture.path(), bytes).unwrap();
            assert!(load_settings_at(&fixture.path()).is_err());
        }
    }

    #[test]
    fn explicit_search_query_validation_and_bounded_json() {
        for query in ["", " ", "a", "x\nx", &"x".repeat(101)] {
            assert!(validate_query(query).is_err());
        }
        assert!(validate_query("東京").is_ok());
        assert!(validate_query("São Paulo").is_ok());
        assert_eq!(
            read_bounded(vec![b' '; MAX_BYTES as usize].as_slice())
                .unwrap()
                .len(),
            MAX_BYTES as usize
        );
        assert!(read_bounded(vec![b' '; MAX_BYTES as usize + 1].as_slice()).is_err());
        let fixture = Fixture::new();
        fs::write(fixture.path(), vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(load_settings_at(&fixture.path()).is_err());
    }

    #[test]
    fn search_response_validation_and_disambiguation() {
        let bytes = serde_json::to_vec(&search_response()).unwrap();
        assert_eq!(parse_response(&bytes).unwrap(), vec![city()]);
        assert!(parse_response(br#"{"generationtime_ms":0.1}"#)
            .unwrap()
            .is_empty());
        assert!(parse_response(b"{}").is_err());
        assert!(parse_response(br#"{"error":true,"reason":"bad request"}"#).is_err());
        assert!(parse_response(b"{").is_err());
        for (field, value) in [
            ("timezone", json!("Invalid/Zone")),
            ("timezone", json!(null)),
            ("latitude", json!(91)),
            ("longitude", json!(null)),
            ("name", json!("")),
            ("country", json!("x".repeat(81))),
        ] {
            let mut response = search_response();
            response["results"][0][field] = value;
            assert!(
                parse_response(&serde_json::to_vec(&response).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut response = search_response();
        response["results"][0]
            .as_object_mut()
            .unwrap()
            .remove("timezone");
        assert!(parse_response(&serde_json::to_vec(&response).unwrap()).is_err());
        let mut response = search_response();
        response["results"] = json!(vec![response["results"][0].clone(); 101]);
        assert!(parse_response(&serde_json::to_vec(&response).unwrap()).is_err());
        response["results"] = json!(vec![response["results"][0].clone(); 2]);
        assert_eq!(
            parse_response(&serde_json::to_vec(&response).unwrap())
                .unwrap()
                .len(),
            1
        );
    }
}
