//! Explicit-location, offline-first Open-Meteo forecasts.
//!
//! The calendar supplies a `Config` from its explicitly selected primary city
//! in `smplos/calendar.json`. No location is inferred.
//! Call `load_cache` before starting `fetch` on a worker thread. Network errors
//! never replace a cached forecast. Cache age is limited to seven days, with
//! forecasts marked stale after one hour or whenever fewer than seven current
//! destination-local dates remain. Refresh scheduling uses the original fetch
//! timestamp, not the time the calendar opens. Future timestamps are rejected
//! without a clock-skew allowance.

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
const MAX_BYTES: u64 = 128 * 1024;
const CACHE_VERSION: u8 = 2;
const FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(3600);
const MAX_AGE_DAYS: i64 = 7;
static WRITE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TemperatureUnit {
    Celsius,
    Fahrenheit,
}

impl TemperatureUnit {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Celsius => "°C",
            Self::Fahrenheit => "°F",
        }
    }

    fn query_value(self) -> &'static str {
        match self {
            Self::Celsius => "celsius",
            Self::Fahrenheit => "fahrenheit",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub latitude: f64,
    pub longitude: f64,
    pub location: String,
    pub timezone: String,
    pub temperature_unit: TemperatureUnit,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.latitude.is_finite() && (-90.0..=90.0).contains(&self.latitude),
            "Weather latitude must be finite and between -90 and 90"
        );
        ensure!(
            self.longitude.is_finite() && (-180.0..=180.0).contains(&self.longitude),
            "Weather longitude must be finite and between -180 and 180"
        );
        ensure!(
            !self.location.trim().is_empty()
                && self.location.chars().count() <= 80
                && !self.location.chars().any(char::is_control),
            "Weather location must contain 1–80 characters and no control characters"
        );
        self.timezone
            .parse::<Tz>()
            .context("Weather timezone must be a known IANA timezone")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Forecast {
    #[serde(with = "timestamp")]
    pub fetched_at: DateTime<Utc>,
    /// Offset returned by Open-Meteo for the configured destination.
    pub utc_offset_seconds: i32,
    pub days: Vec<ForecastDay>,
    /// Derived on load, never trusted from disk.
    #[serde(skip)]
    pub stale: bool,
}

impl Forecast {
    /// Time until the original fetch reaches one hour old. Expired forecasts
    /// need an immediate refresh; reopening never starts a new one-hour TTL.
    /// Future timestamps also request an immediate refresh, but are rejected
    /// outright by cache loading and saving.
    pub fn refresh_after(&self, now: DateTime<Utc>) -> std::time::Duration {
        match (now - self.fetched_at).to_std() {
            Ok(age) => FRESH_FOR.saturating_sub(age),
            Err(_) => std::time::Duration::ZERO,
        }
    }

    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        self.refresh_after(now).is_zero()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastDay {
    #[serde(with = "date")]
    pub date: NaiveDate,
    pub min: f64,
    pub max: f64,
    pub precipitation_probability: u8,
    pub weather_code: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunderstorm,
}

impl Condition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::PartlyCloudy => "Partly cloudy",
            Self::Cloudy => "Cloudy",
            Self::Fog => "Fog",
            Self::Drizzle => "Drizzle",
            Self::Rain => "Rain",
            Self::Snow => "Snow",
            Self::Thunderstorm => "Thunderstorm",
        }
    }
}

impl ForecastDay {
    /// Unknown WMO codes are rejected by both fetch and cache loading.
    pub fn condition(&self) -> Option<Condition> {
        match self.weather_code {
            0 => Some(Condition::Clear),
            1 | 2 => Some(Condition::PartlyCloudy),
            3 => Some(Condition::Cloudy),
            45 | 48 => Some(Condition::Fog),
            51 | 53 | 55 | 56 | 57 => Some(Condition::Drizzle),
            61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => Some(Condition::Rain),
            71 | 73 | 75 | 77 | 85 | 86 => Some(Condition::Snow),
            95 | 96 | 99 => Some(Condition::Thunderstorm),
            _ => None,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cache {
    version: u8,
    config: Config,
    forecast: Forecast,
}

fn cache_path() -> Result<PathBuf> {
    Ok(dirs::cache_dir()
        .context("Could not determine weather cache directory")?
        .join("smplos/calendar-weather.json"))
}

/// Missing, expired, differently configured, or no-longer-useful caches return
/// `None`. Corrupt caches return an error, allowing the caller to report it while
/// still attempting a network refresh.
pub fn load_cache(config: &Config) -> Result<Option<Forecast>> {
    load_cache_at(config, &cache_path()?, Utc::now())
}

fn load_cache_at(config: &Config, path: &Path, now: DateTime<Utc>) -> Result<Option<Forecast>> {
    config.validate()?;
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    let cache: Cache = serde_json::from_slice(&bytes).context("Invalid weather cache")?;
    ensure!(
        cache.version == CACHE_VERSION,
        "Unsupported weather cache version"
    );
    cache
        .config
        .validate()
        .context("Invalid weather cache configuration")?;
    if cache.config != *config {
        return Ok(None);
    }
    let mut forecast = cache.forecast;
    validate_forecast(config, &forecast, now)?;
    let age = now - forecast.fetched_at;
    if age > Duration::days(MAX_AGE_DAYS) {
        return Ok(None);
    }
    let today = destination_date(config, now)?;
    let end = today
        .checked_add_signed(Duration::days(6))
        .context("Weather date out of range")?;
    forecast
        .days
        .retain(|day| day.date >= today && day.date <= end);
    if forecast.days.is_empty() {
        return Ok(None);
    }
    forecast.stale = forecast.needs_refresh(now) || forecast.days.len() != 7;
    Ok(Some(forecast))
}

/// Performs one HTTPS request with no redirects/retries, a ten-second overall
/// timeout, and a 128 KiB decoded-body limit. Must not run on the UI thread.
pub fn fetch(config: &Config) -> Result<Forecast> {
    config.validate()?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .redirects(0)
        .build();
    let response = agent
        .get(ENDPOINT)
        .query("latitude", &config.latitude.to_string())
        .query("longitude", &config.longitude.to_string())
        .query("timezone", &config.timezone)
        .query("forecast_days", "7")
        .query("temperature_unit", config.temperature_unit.query_value())
        .query(
            "daily",
            "temperature_2m_min,temperature_2m_max,precipitation_probability_max,weather_code",
        )
        .call()
        .context("Could not fetch Open-Meteo forecast")?;
    ensure!(
        response.status() == 200,
        "Unexpected Open-Meteo response status"
    );
    let bytes = read_bounded(response.into_reader())?;
    parse_response(config, &bytes, Utc::now())
}

/// Atomically replaces the cache only after validating the complete forecast.
pub fn save_cache(config: &Config, forecast: &Forecast) -> Result<()> {
    save_cache_at(config, forecast, &cache_path()?, Utc::now())
}

fn save_cache_at(
    config: &Config,
    forecast: &Forecast,
    path: &Path,
    now: DateTime<Utc>,
) -> Result<()> {
    config.validate()?;
    validate_forecast(config, forecast, now)?;
    ensure!(
        now - forecast.fetched_at <= Duration::days(MAX_AGE_DAYS),
        "Weather forecast is too old to cache"
    );
    let bytes = serde_json::to_vec(&Cache {
        version: CACHE_VERSION,
        config: config.clone(),
        forecast: forecast.clone(),
    })?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Weather cache is too large"
    );
    let parent = path
        .parent()
        .context("Weather cache has no parent directory")?;
    fs::create_dir_all(parent).context("Could not create weather cache directory")?;
    let staging = parent.join(format!(
        ".calendar-weather.{}.{}.json",
        std::process::id(),
        WRITE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&staging)
        .context("Could not create weather cache staging file")?;
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
    result.context("Could not save weather cache")
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match File::open(path) {
        Ok(file) => read_bounded(file).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
    }
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Weather data exceeds 128 KiB"
    );
    Ok(bytes)
}

fn destination_date(config: &Config, now: DateTime<Utc>) -> Result<NaiveDate> {
    let timezone = config
        .timezone
        .parse::<Tz>()
        .context("Weather timezone must be a known IANA timezone")?;
    Ok(now.with_timezone(&timezone).date_naive())
}

fn validate_forecast(config: &Config, forecast: &Forecast, now: DateTime<Utc>) -> Result<()> {
    ensure!(
        forecast.fetched_at <= now,
        "Weather cache timestamp is in the future"
    );
    ensure!(
        (-43_200..=50_400).contains(&forecast.utc_offset_seconds),
        "Invalid weather UTC offset"
    );
    let first_date = destination_date(config, forecast.fetched_at)?;
    let last_date = first_date
        .checked_add_signed(Duration::days(6))
        .context("Weather date out of range")?;
    ensure!(
        !forecast.days.is_empty() && forecast.days.len() <= 7,
        "Weather forecast must contain 1–7 days"
    );
    for (index, day) in forecast.days.iter().enumerate() {
        ensure!(
            day.date >= first_date && day.date <= last_date,
            "Weather forecast date is outside its original seven-day window"
        );
        if index > 0 {
            ensure!(
                forecast.days[index - 1].date.succ_opt() == Some(day.date),
                "Weather forecast dates must be contiguous and increasing"
            );
        }
        let (min, max) = match config.temperature_unit {
            TemperatureUnit::Celsius => (day.min, day.max),
            TemperatureUnit::Fahrenheit => ((day.min - 32.0) / 1.8, (day.max - 32.0) / 1.8),
        };
        ensure!(
            min.is_finite()
                && max.is_finite()
                && (-100.0..=70.0).contains(&min)
                && (-100.0..=70.0).contains(&max)
                && min <= max,
            "Invalid weather temperature range"
        );
        ensure!(
            day.precipitation_probability <= 100,
            "Invalid precipitation probability"
        );
        ensure!(day.condition().is_some(), "Invalid WMO weather code");
    }
    Ok(())
}

#[derive(Deserialize)]
struct ApiResponse {
    utc_offset_seconds: i32,
    daily_units: ApiUnits,
    daily: ApiDays,
}

#[derive(Deserialize)]
struct ApiUnits {
    time: String,
    temperature_2m_min: String,
    temperature_2m_max: String,
    precipitation_probability_max: String,
    weather_code: String,
}

#[derive(Deserialize)]
struct ApiDays {
    time: Vec<String>,
    temperature_2m_min: Vec<Option<f64>>,
    temperature_2m_max: Vec<Option<f64>>,
    precipitation_probability_max: Vec<Option<u8>>,
    weather_code: Vec<Option<u8>>,
}

fn parse_response(config: &Config, bytes: &[u8], now: DateTime<Utc>) -> Result<Forecast> {
    let response: ApiResponse =
        serde_json::from_slice(bytes).context("Invalid Open-Meteo response")?;
    let units = response.daily_units;
    ensure!(
        units.time == "iso8601"
            && units.temperature_2m_min == config.temperature_unit.symbol()
            && units.temperature_2m_max == config.temperature_unit.symbol()
            && units.precipitation_probability_max == "%"
            && units.weather_code == "wmo code",
        "Unexpected Open-Meteo forecast units"
    );
    let data = response.daily;
    ensure!(
        data.time.len() == 7
            && data.temperature_2m_min.len() == 7
            && data.temperature_2m_max.len() == 7
            && data.precipitation_probability_max.len() == 7
            && data.weather_code.len() == 7,
        "Open-Meteo must return seven complete forecast days"
    );
    let mut days = Vec::with_capacity(7);
    for index in 0..7 {
        days.push(ForecastDay {
            date: parse_date(&data.time[index])?,
            min: data.temperature_2m_min[index].context("Missing minimum temperature")?,
            max: data.temperature_2m_max[index].context("Missing maximum temperature")?,
            precipitation_probability: data.precipitation_probability_max[index]
                .context("Missing precipitation probability")?,
            weather_code: data.weather_code[index].context("Missing weather code")?,
        });
    }
    let forecast = Forecast {
        fetched_at: now,
        utc_offset_seconds: response.utc_offset_seconds,
        days,
        stale: false,
    };
    validate_forecast(config, &forecast, now)?;
    Ok(forecast)
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").context("Invalid weather date")?;
    if date.format("%Y-%m-%d").to_string() != value {
        bail!("Weather date must use YYYY-MM-DD");
    }
    Ok(date)
}

mod date {
    use super::*;

    pub fn serialize<S: Serializer>(value: &NaiveDate, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.format("%Y-%m-%d").to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveDate, D::Error> {
        parse_date(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

mod timestamp {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(value.timestamp())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DateTime<Utc>, D::Error> {
        DateTime::from_timestamp(i64::deserialize(deserializer)?, 0)
            .ok_or_else(|| serde::de::Error::custom("Invalid weather timestamp"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/weather-test-fixtures")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    WRITE_ID.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> PathBuf {
            self.0.join("weather.json")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config() -> Config {
        Config {
            latitude: 51.5,
            longitude: -0.12,
            location: "Test location".into(),
            timezone: "Europe/London".into(),
            temperature_unit: TemperatureUnit::Celsius,
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-10-02T12:00:00Z".parse().unwrap()
    }

    fn response() -> Value {
        json!({
            "utc_offset_seconds": 3600,
            "daily_units": {
                "time": "iso8601", "temperature_2m_min": "°C",
                "temperature_2m_max": "°C", "precipitation_probability_max": "%",
                "weather_code": "wmo code"
            },
            "daily": {
                "time": ["2026-10-02", "2026-10-03", "2026-10-04", "2026-10-05",
                         "2026-10-06", "2026-10-07", "2026-10-08"],
                "temperature_2m_min": [1, 2, 3, 4, 5, 6, 7],
                "temperature_2m_max": [11, 12, 13, 14, 15, 16, 17],
                "precipitation_probability_max": [0, 10, 20, 30, 40, 50, 100],
                "weather_code": [0, 1, 3, 45, 61, 71, 95]
            }
        })
    }

    fn parse(value: Value) -> Result<Forecast> {
        parse_response(&config(), &serde_json::to_vec(&value).unwrap(), now())
    }

    fn forecast() -> Forecast {
        parse(response()).unwrap()
    }

    #[test]
    fn configuration_is_explicit_and_validated() {
        let decoded: Config =
            serde_json::from_slice(&serde_json::to_vec(&config()).unwrap()).unwrap();
        assert_eq!(decoded, config());
        assert!(decoded.validate().is_ok());
        for bytes in [
            "{",
            "{}",
            r#"{"latitude":0,"longitude":0,"location":"x"}"#,
            r#"{"latitude":0,"longitude":0,"location":"x","temperature_unit":"kelvin"}"#,
        ] {
            assert!(serde_json::from_str::<Config>(bytes).is_err());
        }
        for (latitude, longitude) in [
            (91.0, 0.0),
            (0.0, -181.0),
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
        ] {
            assert!(Config {
                latitude,
                longitude,
                ..config()
            }
            .validate()
            .is_err());
        }
        for location in ["".into(), "  ".into(), "x".repeat(81), "A\nB".into()] {
            assert!(Config {
                location,
                ..config()
            }
            .validate()
            .is_err());
        }
        for timezone in ["", "Invalid/Timezone", "+01:00"] {
            assert!(Config {
                timezone: timezone.into(),
                ..config()
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn provider_requires_complete_valid_contiguous_days() {
        let forecast = forecast();
        assert_eq!(forecast.days.len(), 7);
        assert!(!forecast.stale);
        assert_eq!(forecast.days[0].condition(), Some(Condition::Clear));
        assert_eq!(forecast.days[6].condition(), Some(Condition::Thunderstorm));
        for (field, invalid) in [
            ("temperature_2m_min", json!(null)),
            ("temperature_2m_max", json!(-20)),
            ("temperature_2m_max", json!(200)),
            ("precipitation_probability_max", json!(101)),
            ("precipitation_probability_max", json!(-1)),
            ("precipitation_probability_max", json!(0.5)),
            ("weather_code", json!(4)),
            ("weather_code", json!(null)),
            ("time", json!("2026-10-04")),
            ("time", json!("2026-10-2")),
            ("time", json!("2026-02-30")),
        ] {
            let mut value = response();
            value["daily"][field][0] = invalid;
            assert!(parse(value).is_err(), "{field}");
        }
        let mut value = response();
        value["daily"]["time"][1] = json!("2026-10-02");
        assert!(parse(value).is_err());
        let mut value = response();
        value["daily"]["temperature_2m_min"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(parse(value).is_err());
        let mut value = response();
        value["daily"]
            .as_object_mut()
            .unwrap()
            .remove("weather_code");
        assert!(parse(value).is_err());
        assert!(parse(json!({"error": true, "reason": "invalid request"})).is_err());
    }

    #[test]
    fn units_and_destination_date_are_verified() {
        let mut value = response();
        value["daily_units"]["temperature_2m_max"] = json!("°F");
        assert!(parse(value).is_err());
        let mut value = response();
        value["utc_offset_seconds"] = json!(90_000);
        assert!(parse(value).is_err());
        assert!(parse_response(&config(), b"{", now()).is_err());
        let late: DateTime<Utc> = "2026-10-01T23:30:00Z".parse().unwrap();
        assert!(parse_response(&config(), &serde_json::to_vec(&response()).unwrap(), late).is_ok());
        let next_day: DateTime<Utc> = "2026-10-02T23:30:00Z".parse().unwrap();
        assert!(parse_response(
            &config(),
            &serde_json::to_vec(&response()).unwrap(),
            next_day
        )
        .is_err());
        let mut value = response();
        value["daily_units"]["temperature_2m_min"] = json!("°F");
        value["daily_units"]["temperature_2m_max"] = json!("°F");
        let fahrenheit = Config {
            temperature_unit: TemperatureUnit::Fahrenheit,
            ..config()
        };
        assert!(parse_response(&fahrenheit, &serde_json::to_vec(&value).unwrap(), now()).is_ok());
    }

    #[test]
    fn cache_roundtrip_identity_age_and_rollover() {
        let fixture = Fixture::new();
        let path = fixture.path();
        assert!(load_cache_at(&config(), &path, now()).unwrap().is_none());
        save_cache_at(&config(), &forecast(), &path, now()).unwrap();
        let cached = load_cache_at(&config(), &path, now()).unwrap().unwrap();
        assert_eq!(cached.days.len(), 7);
        assert!(!cached.stale);
        assert_eq!(cached.fetched_at, now());
        for changed in [
            Config {
                latitude: 50.0,
                ..config()
            },
            Config {
                longitude: 1.0,
                ..config()
            },
            Config {
                location: "Other".into(),
                ..config()
            },
            Config {
                temperature_unit: TemperatureUnit::Fahrenheit,
                ..config()
            },
            Config {
                timezone: "Europe/Paris".into(),
                ..config()
            },
        ] {
            assert!(load_cache_at(&changed, &path, now()).unwrap().is_none());
        }
        let cached = load_cache_at(&config(), &path, now() + Duration::hours(1))
            .unwrap()
            .unwrap();
        assert!(cached.stale);
        let cached = load_cache_at(&config(), &path, now() + Duration::days(1))
            .unwrap()
            .unwrap();
        assert!(cached.stale);
        assert_eq!(cached.days.len(), 6);
        assert_eq!(cached.days[0].date.to_string(), "2026-10-03");
        assert!(load_cache_at(&config(), &path, now() + Duration::days(8))
            .unwrap()
            .is_none());
        assert!(load_cache_at(&config(), &path, now() + Duration::days(7))
            .unwrap()
            .is_none());
        assert!(load_cache_at(&config(), &path, now() - Duration::seconds(1)).is_err());
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let linked = fixture.0.join("linked-cache.json");
            let original = fs::read(&path).unwrap();
            symlink(&path, &linked).unwrap();
            save_cache_at(&config(), &forecast(), &linked, now()).unwrap();
            assert!(!fs::symlink_metadata(&linked)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn restart_preserves_exact_one_hour_refresh_deadline() {
        let fixture = Fixture::new();
        save_cache_at(&config(), &forecast(), &fixture.path(), now()).unwrap();
        for (age, remaining) in [(0, 3600), (1800, 1800), (3599, 1), (3600, 0), (7200, 0)] {
            let opened_at = now() + Duration::seconds(age);
            let cached = load_cache_at(&config(), &fixture.path(), opened_at)
                .unwrap()
                .unwrap();
            assert_eq!(
                cached.refresh_after(opened_at),
                std::time::Duration::from_secs(remaining)
            );
            assert_eq!(cached.needs_refresh(opened_at), remaining == 0);
            assert_eq!(cached.stale, remaining == 0);
            assert_eq!(cached.days.len(), 7);
        }
        assert_eq!(
            forecast().refresh_after(now() + Duration::milliseconds(3_599_999)),
            std::time::Duration::from_millis(1)
        );
        assert!(forecast().needs_refresh(now() - Duration::nanoseconds(1)));
        assert!(
            load_cache_at(&config(), &fixture.path(), now() - Duration::nanoseconds(1)).is_err()
        );
    }

    #[test]
    fn cache_dates_follow_current_iana_dst_not_stored_offset() {
        for (fetched, opened, stored_offset, expected_date, expected_days) in [
            (
                "2026-03-28T12:00:00Z",
                "2026-03-29T23:30:00Z",
                0,
                "2026-03-30",
                5,
            ),
            (
                "2026-10-24T12:00:00Z",
                "2026-10-25T23:30:00Z",
                3600,
                "2026-10-25",
                6,
            ),
        ] {
            let fixture = Fixture::new();
            let fetched: DateTime<Utc> = fetched.parse().unwrap();
            let opened: DateTime<Utc> = opened.parse().unwrap();
            let mut data = forecast();
            data.fetched_at = fetched;
            data.utc_offset_seconds = stored_offset;
            let first = destination_date(&config(), fetched).unwrap();
            for (index, day) in data.days.iter_mut().enumerate() {
                day.date = first + Duration::days(index as i64);
            }
            save_cache_at(&config(), &data, &fixture.path(), fetched).unwrap();
            let cached = load_cache_at(&config(), &fixture.path(), opened)
                .unwrap()
                .unwrap();
            assert!(cached.stale);
            assert_eq!(cached.days[0].date.to_string(), expected_date);
            assert_eq!(cached.days.len(), expected_days);
            assert_eq!(
                cached.days.last().unwrap().date,
                data.days.last().unwrap().date
            );
        }
    }

    #[test]
    fn incomplete_current_window_is_stale_even_when_recent() {
        let fixture = Fixture::new();
        let time: DateTime<Utc> = "2026-10-02T22:30:00Z".parse().unwrap();
        let mut data = forecast();
        data.fetched_at = time;
        save_cache_at(&config(), &data, &fixture.path(), time).unwrap();
        let cached = load_cache_at(&config(), &fixture.path(), time + Duration::minutes(45))
            .unwrap()
            .unwrap();
        assert!(cached.stale);
        assert_eq!(cached.days.len(), 6);
        assert!(!cached.needs_refresh(time + Duration::minutes(45)));
    }

    #[test]
    fn invalid_cache_and_invalid_replacements_are_rejected() {
        let fixture = Fixture::new();
        let path = fixture.path();
        fs::write(&path, b"broken").unwrap();
        assert!(load_cache_at(&config(), &path, now()).is_err());
        save_cache_at(&config(), &forecast(), &path, now()).unwrap();
        let original = fs::read(&path).unwrap();
        for mutation in 0..6 {
            let mut invalid = forecast();
            match mutation {
                0 => invalid.days[0].min = f64::NAN,
                1 => invalid.days[0].max = f64::INFINITY,
                2 => invalid.days[1].weather_code = 42,
                3 => invalid.days[1].date = invalid.days[0].date,
                4 => invalid.fetched_at = now() + Duration::seconds(1),
                _ => invalid.days[0].precipitation_probability = 101,
            }
            assert!(save_cache_at(&config(), &invalid, &path, now()).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        for (pointer, value) in [
            ("/forecast/days/0/weather_code", json!(42)),
            ("/forecast/days/0/max", json!(null)),
            ("/forecast/days/0/date", json!("2030-01-01")),
            ("/forecast/fetched_at", json!(i64::MAX)),
            ("/version", json!(CACHE_VERSION + 1)),
            ("/config/timezone", json!("Invalid/Timezone")),
        ] {
            let mut invalid: Value = serde_json::from_slice(&original).unwrap();
            *invalid.pointer_mut(pointer).unwrap() = value;
            fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(load_cache_at(&config(), &path, now()).is_err(), "{pointer}");
        }
        let mut old_format: Value = serde_json::from_slice(&original).unwrap();
        old_format["version"] = json!(1);
        old_format["config"]
            .as_object_mut()
            .unwrap()
            .remove("timezone");
        fs::write(&path, serde_json::to_vec(&old_format).unwrap()).unwrap();
        assert!(load_cache_at(&config(), &path, now()).is_err());
    }

    #[test]
    fn body_limits_cover_network_and_disk() {
        assert_eq!(
            read_bounded(vec![b' '; MAX_BYTES as usize].as_slice())
                .unwrap()
                .len(),
            MAX_BYTES as usize
        );
        assert!(read_bounded(vec![b' '; MAX_BYTES as usize + 1].as_slice()).is_err());
        let fixture = Fixture::new();
        fs::write(fixture.path(), vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(load_cache_at(&config(), &fixture.path(), now()).is_err());
    }
}
