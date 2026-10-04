use crate::debug_log;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

// ── Language + Model data ────────────────────────────────────────────────────

pub struct Language {
    pub name: &'static str,
    pub code: &'static str,
}

/// All Whisper-supported languages + English default + auto-detect.
pub static LANGUAGES: &[Language] = &[
    Language { name: "English (recommended)", code: "en" },
    Language { name: "Auto-detect (all languages)", code: "auto" },
    Language { name: "Afrikaans", code: "af" },
    Language { name: "Albanian", code: "sq" },
    Language { name: "Amharic", code: "am" },
    Language { name: "Arabic", code: "ar" },
    Language { name: "Armenian", code: "hy" },
    Language { name: "Assamese", code: "as" },
    Language { name: "Azerbaijani", code: "az" },
    Language { name: "Bashkir", code: "ba" },
    Language { name: "Basque", code: "eu" },
    Language { name: "Belarusian", code: "be" },
    Language { name: "Bengali", code: "bn" },
    Language { name: "Bosnian", code: "bs" },
    Language { name: "Breton", code: "br" },
    Language { name: "Bulgarian", code: "bg" },
    Language { name: "Cantonese", code: "yue" },
    Language { name: "Catalan", code: "ca" },
    Language { name: "Chinese", code: "zh" },
    Language { name: "Croatian", code: "hr" },
    Language { name: "Czech", code: "cs" },
    Language { name: "Danish", code: "da" },
    Language { name: "Dutch", code: "nl" },
    Language { name: "Estonian", code: "et" },
    Language { name: "Faroese", code: "fo" },
    Language { name: "Finnish", code: "fi" },
    Language { name: "French", code: "fr" },
    Language { name: "Galician", code: "gl" },
    Language { name: "Georgian", code: "ka" },
    Language { name: "German", code: "de" },
    Language { name: "Greek", code: "el" },
    Language { name: "Gujarati", code: "gu" },
    Language { name: "Haitian Creole", code: "ht" },
    Language { name: "Hausa", code: "ha" },
    Language { name: "Hawaiian", code: "haw" },
    Language { name: "Hebrew", code: "he" },
    Language { name: "Hindi", code: "hi" },
    Language { name: "Hungarian", code: "hu" },
    Language { name: "Icelandic", code: "is" },
    Language { name: "Indonesian", code: "id" },
    Language { name: "Italian", code: "it" },
    Language { name: "Japanese", code: "ja" },
    Language { name: "Javanese", code: "jw" },
    Language { name: "Kannada", code: "kn" },
    Language { name: "Kazakh", code: "kk" },
    Language { name: "Khmer", code: "km" },
    Language { name: "Korean", code: "ko" },
    Language { name: "Lao", code: "lo" },
    Language { name: "Latin", code: "la" },
    Language { name: "Latvian", code: "lv" },
    Language { name: "Lingala", code: "ln" },
    Language { name: "Lithuanian", code: "lt" },
    Language { name: "Luxembourgish", code: "lb" },
    Language { name: "Macedonian", code: "mk" },
    Language { name: "Malagasy", code: "mg" },
    Language { name: "Malay", code: "ms" },
    Language { name: "Malayalam", code: "ml" },
    Language { name: "Maltese", code: "mt" },
    Language { name: "Maori", code: "mi" },
    Language { name: "Marathi", code: "mr" },
    Language { name: "Mongolian", code: "mn" },
    Language { name: "Myanmar", code: "my" },
    Language { name: "Nepali", code: "ne" },
    Language { name: "Norwegian", code: "no" },
    Language { name: "Nynorsk", code: "nn" },
    Language { name: "Occitan", code: "oc" },
    Language { name: "Pashto", code: "ps" },
    Language { name: "Persian", code: "fa" },
    Language { name: "Polish", code: "pl" },
    Language { name: "Portuguese", code: "pt" },
    Language { name: "Punjabi", code: "pa" },
    Language { name: "Romanian", code: "ro" },
    Language { name: "Russian", code: "ru" },
    Language { name: "Sanskrit", code: "sa" },
    Language { name: "Serbian", code: "sr" },
    Language { name: "Shona", code: "sn" },
    Language { name: "Sindhi", code: "sd" },
    Language { name: "Sinhala", code: "si" },
    Language { name: "Slovak", code: "sk" },
    Language { name: "Slovenian", code: "sl" },
    Language { name: "Somali", code: "so" },
    Language { name: "Spanish", code: "es" },
    Language { name: "Sundanese", code: "su" },
    Language { name: "Swahili", code: "sw" },
    Language { name: "Swedish", code: "sv" },
    Language { name: "Tagalog", code: "tl" },
    Language { name: "Tajik", code: "tg" },
    Language { name: "Tamil", code: "ta" },
    Language { name: "Tatar", code: "tt" },
    Language { name: "Telugu", code: "te" },
    Language { name: "Thai", code: "th" },
    Language { name: "Tibetan", code: "bo" },
    Language { name: "Turkish", code: "tr" },
    Language { name: "Turkmen", code: "tk" },
    Language { name: "Ukrainian", code: "uk" },
    Language { name: "Urdu", code: "ur" },
    Language { name: "Uzbek", code: "uz" },
    Language { name: "Vietnamese", code: "vi" },
    Language { name: "Welsh", code: "cy" },
    Language { name: "Yiddish", code: "yi" },
    Language { name: "Yoruba", code: "yo" },
];

pub struct WhisperModel {
    pub id: &'static str,
    pub label: &'static str,
    pub size: &'static str,
    pub note: &'static str,
    pub english_only: bool,
}

pub static MODELS: &[WhisperModel] = &[
    WhisperModel { id: "base.en",         label: "Base (English)", size: "~150 MB", note: "Fastest, English only",     english_only: true  },
    WhisperModel { id: "base",            label: "Base",           size: "~150 MB", note: "Fast, 99 languages",        english_only: false },
    WhisperModel { id: "small",           label: "Small",          size: "~500 MB", note: "Good balance",              english_only: false },
    WhisperModel { id: "medium",          label: "Medium",         size: "~1.5 GB", note: "Higher accuracy, slower",   english_only: false },
    WhisperModel { id: "large-v3-turbo",  label: "Large Turbo",    size: "~1.6 GB", note: "Best quality, needs 6GB+ RAM", english_only: false },
];

pub fn find_language_idx(code: &str) -> Option<usize> {
    LANGUAGES.iter().position(|l| l.code == code)
}

pub fn find_model_idx(id: &str) -> Option<usize> {
    MODELS.iter().position(|m| m.id == id)
}

fn language_name(code: &str) -> &str {
    LANGUAGES.iter()
        .find(|l| l.code == code)
        .map(|l| l.name)
        .unwrap_or(code)
}

pub fn language_display(cfg: &DictationConfig) -> slint::SharedString {
    if cfg.also_english {
        format!("{} + English", language_name(&cfg.primary_code)).into()
    } else {
        language_name(&cfg.primary_code).into()
    }
}

fn model_label(model_id: &str) -> &str {
    MODELS.iter()
        .find(|m| m.id == model_id)
        .map(|m| m.label)
        .unwrap_or(model_id)
}

pub fn model_display(model_id: &str) -> slint::SharedString {
    model_label(model_id).into()
}

pub fn is_model_english_only(model_idx: usize) -> bool {
    MODELS.get(model_idx).map(|m| m.english_only).unwrap_or(false)
}

static INSTALL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub fn is_install_running() -> bool {
    INSTALL_IN_PROGRESS.load(Ordering::Relaxed)
}

fn set_install_running(v: bool) {
    INSTALL_IN_PROGRESS.store(v, Ordering::Relaxed);
}

// ── Config read/write ────────────────────────────────────────────────────────

pub struct DictationConfig {
    pub primary_code: String,
    pub also_english: bool,
    pub model: String,
    /// Top-level `engine`; voxtype uses whisper when it's absent.
    pub engine: String,
    /// `[whisper] mode` (or the deprecated `backend`): local, remote or cli.
    pub whisper_mode: String,
}

impl DictationConfig {
    /// Whether voxtype loads a local whisper.cpp model file with this config.
    pub fn uses_local_model(&self) -> bool {
        self.engine == "whisper" && self.whisper_mode != "remote"
    }
}

fn home_dir() -> Option<String> {
    let h = std::env::var("HOME").ok()?;
    if h.is_empty() { None } else { Some(h) }
}

fn config_path() -> Option<String> {
    Some(format!("{}/.config/voxtype/config.toml", home_dir()?))
}

pub fn config_exists() -> bool {
    config_path()
        .map(|p| std::path::Path::new(&p).exists())
        .unwrap_or(false)
}

pub fn is_installed() -> bool {
    Command::new("which")
        .arg("voxtype")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn read_config() -> Option<DictationConfig> {
    let path = config_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let cfg = parse_config(&content);

    debug_log!("[settings] dictation config: primary={}, also_en={}, model={}",
        cfg.primary_code, cfg.also_english, cfg.model);

    Some(cfg)
}

/// `key = value` with both sides trimmed.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

/// A TOML string value without its quotes or a trailing comment.
fn string_value(raw: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(rest) = raw.strip_prefix(quote) {
            return rest.split(quote).next().unwrap_or_default();
        }
    }
    raw.split('#').next().unwrap_or_default().trim()
}

fn parse_config(content: &str) -> DictationConfig {
    let mut language = String::from("auto");
    // voxtype's defaults when the keys are absent.
    let mut model = String::from("base.en");
    let mut engine = String::from("whisper");
    let mut mode: Option<String> = None;
    let mut backend: Option<String> = None;
    let mut section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.to_string();
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = key_value(trimmed) else {
            continue;
        };

        match (section.as_str(), key) {
            ("", "engine") => engine = string_value(raw).to_string(),
            ("[whisper]", "model") => {
                let val = string_value(raw);
                if !val.is_empty() {
                    model = val.to_string();
                }
            }
            ("[whisper]", "mode") => mode = Some(string_value(raw).to_string()),
            ("[whisper]", "backend") => backend = Some(string_value(raw).to_string()),
            ("[whisper]", "language") => {
                if raw.starts_with('[') {
                    let inner = raw
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !inner.is_empty() {
                        language = inner;
                    }
                } else {
                    let v = raw.trim_matches('"').to_string();
                    if !v.is_empty() {
                        language = v;
                    }
                }
            }
            _ => {}
        }
    }

    let codes: Vec<&str> = language.split(',').map(|s| s.trim()).collect();
    let (primary_code, also_english) = if codes.len() > 1 {
        let non_en: Vec<&&str> = codes.iter().filter(|c| **c != "en").collect();
        let primary = non_en.first().map(|c| c.to_string()).unwrap_or_else(|| "en".to_string());
        (primary, codes.contains(&"en"))
    } else {
        (codes[0].to_string(), false)
    };

    DictationConfig {
        primary_code,
        also_english,
        model,
        engine,
        whisper_mode: mode.or(backend).unwrap_or_else(|| "local".to_string()),
    }
}

pub fn is_service_running() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "voxtype"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Model files ──────────────────────────────────────────────────────────────
//
// voxtype's Whisper engine is whisper.cpp: it loads one GGML file per model
// from its data directory, and `voxtype setup --download` fetches that file
// from Hugging Face. The faster-whisper copies smplOS primes into
// ~/.cache/huggingface are a different format that voxtype never reads.

/// Where `voxtype setup --download` gets Whisper models.
pub const MODEL_SOURCE: &str = "huggingface.co/ggerganov/whisper.cpp";

/// Models `voxtype setup --download --model` accepts, with download sizes.
const DOWNLOADS: &[(&str, &str)] = &[
    ("tiny", "~78 MB"),
    ("tiny.en", "~78 MB"),
    ("base", "~150 MB"),
    ("base.en", "~150 MB"),
    ("small", "~500 MB"),
    ("small.en", "~500 MB"),
    ("medium", "~1.5 GB"),
    ("medium.en", "~1.5 GB"),
    ("large-v3", "~3.1 GB"),
    ("large-v3-turbo", "~1.6 GB"),
];

/// whisper.cpp model files start with the GGML magic 0x67676d6c (little-endian).
const GGML_MAGIC: [u8; 4] = *b"lmgg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelStatus {
    Ready,
    Missing,
    /// Present but not a whisper.cpp model, e.g. an error page that a failed
    /// download saved in its place.
    Invalid,
}

/// voxtype's models directory, resolved like voxtype does:
/// `$XDG_DATA_HOME/voxtype/models`, normally `~/.local/share/voxtype/models`.
pub fn models_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("voxtype").join("models"))
}

/// The file voxtype loads for a `[whisper] model` value, following its
/// `resolve_model_path`: an absolute path, a model name, or a `.bin` file
/// name in the models directory. `None` for names voxtype rejects.
pub fn model_file(model: &str, models_dir: &Path) -> Option<PathBuf> {
    if Path::new(model).is_absolute() {
        return Some(PathBuf::from(model));
    }
    let file = match model {
        "large" | "large-v1" => "ggml-large-v1.bin".to_string(),
        "large-v2" => "ggml-large-v2.bin".to_string(),
        name if is_downloadable(name) => format!("ggml-{name}.bin"),
        name if name.ends_with(".bin") => name.to_string(),
        _ => return None,
    };
    Some(models_dir.join(file))
}

pub fn is_downloadable(model: &str) -> bool {
    download_size(model).is_some()
}

fn download_size(model: &str) -> Option<&'static str> {
    DOWNLOADS.iter().find(|(name, _)| *name == model).map(|(_, size)| *size)
}

pub fn model_status(path: &Path) -> ModelStatus {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ModelStatus::Missing,
        _ => return ModelStatus::Invalid,
    }
    let mut magic = [0; 4];
    match File::open(path).and_then(|mut file| file.read_exact(&mut magic)) {
        Ok(()) if magic == GGML_MAGIC => ModelStatus::Ready,
        _ => ModelStatus::Invalid,
    }
}

/// Whether each entry of `MODELS` is downloaded, in order.
pub fn downloaded_models(models_dir: &Path) -> Vec<bool> {
    MODELS
        .iter()
        .map(|m| {
            model_file(m.id, models_dir)
                .is_some_and(|path| model_status(&path) == ModelStatus::Ready)
        })
        .collect()
}

/// What the Dictation page says about the configured model; empty when the
/// model is ready or the config doesn't use a local Whisper model.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModelNotice {
    pub problem: String,
    pub help: String,
    /// Whether "Download model" can fetch it (`voxtype setup --download`).
    pub downloadable: bool,
}

pub fn model_notice(cfg: &DictationConfig, models_dir: &Path, home: Option<&Path>) -> ModelNotice {
    if !cfg.uses_local_model() {
        return ModelNotice::default();
    }
    let Some(path) = model_file(&cfg.model, models_dir) else {
        return ModelNotice {
            problem: format!(
                "voxtype doesn't know the model \"{}\", so dictation can't start.",
                cfg.model
            ),
            help: "Pick a model under Reconfigure.".to_string(),
            downloadable: false,
        };
    };
    let status = model_status(&path);
    if status == ModelStatus::Ready {
        return ModelNotice::default();
    }
    let label = model_label(&cfg.model);
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    match (download_size(&cfg.model), status) {
        (Some(size), ModelStatus::Missing) => ModelNotice {
            problem: format!("The {label} model isn't downloaded, so dictation can't start."),
            help: format!(
                "Download model fetches {file} ({size}) from {MODEL_SOURCE} into {}; \
                 needs internet. Terminal: voxtype setup --download --model {}",
                display_path(models_dir, home),
                cfg.model
            ),
            downloadable: true,
        },
        (Some(size), _) => ModelNotice {
            problem: format!(
                "The {label} model file isn't a valid Whisper model, so dictation can't start."
            ),
            help: format!(
                "Download model moves it aside and fetches {file} ({size}) again from \
                 {MODEL_SOURCE}; needs internet."
            ),
            downloadable: true,
        },
        (None, status) => ModelNotice {
            problem: if status == ModelStatus::Missing {
                "voxtype can't find the model file, so dictation can't start."
            } else {
                "The model file isn't a valid Whisper model, so dictation can't start."
            }
            .to_string(),
            help: format!(
                "It looks for {}. Put a whisper.cpp model there, or pick another model \
                 under Reconfigure.",
                display_path(&path, home)
            ),
            downloadable: false,
        },
    }
}

/// `path` with the home directory shown as `~`.
fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

// ── Setup actions ────────────────────────────────────────────────────────────

pub fn write_config(lang_code: &str, model_id: &str, also_english: bool) -> bool {
    let dir = match home_dir() {
        Some(h) => format!("{}/.config/voxtype", h),
        None => {
            eprintln!("[settings] cannot write config: HOME not set");
            return false;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[settings] cannot create config dir: {}", e);
        return false;
    }

    let config = config_text(lang_code, model_id, also_english);
    let path = format!("{}/config.toml", dir);
    match std::fs::write(&path, &config) {
        Ok(_) => {
            debug_log!("[settings] wrote config to {}", path);
            true
        }
        Err(e) => {
            eprintln!("[settings] failed to write config: {}", e);
            false
        }
    }
}

fn config_text(lang_code: &str, model_id: &str, also_english: bool) -> String {
    let lang_line = if lang_code == "en" || lang_code == "auto" {
        format!("language = \"{}\"", lang_code)
    } else if also_english {
        format!("language = [\"en\", \"{}\"]", lang_code)
    } else {
        format!("language = \"{}\"", lang_code)
    };

    // voxtype treats any device name except "default" (the system microphone)
    // literally, so "auto" fails with "Audio device not found".
    format!(
        "# Voxtype configuration for smplOS\n\
         # Docs: https://github.com/peteonrails/voxtype\n\
         \n\
         # Use compositor keybindings (SUPER+CTRL+X) instead of built-in hotkey\n\
         [hotkey]\n\
         enabled = false\n\
         \n\
         state_file = \"auto\"\n\
         \n\
         [whisper]\n\
         model = \"{model_id}\"\n\
         {lang_line}\n\
         \n\
         [output]\n\
         mode = \"type\"\n\
         fallback_to_clipboard = true\n\
         append_text = \" \"\n\
         \n\
         [output.notification]\n\
         on_transcription = true\n\
         \n\
         [audio]\n\
         device = \"default\"\n\
         sample_rate = 16000\n\
         max_duration_secs = 60\n\
         \n\
         [audio.feedback]\n\
         enabled = true\n\
         theme = \"default\"\n"
    )
}

// ── Progress tracking ────────────────────────────────────────────────────────

fn progress_file_path() -> String {
    let run_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".to_string());
    format!("{}/settings-install-progress", run_dir)
}

pub fn read_progress() -> (f32, String) {
    let path = progress_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return (0.0, String::new());
            }
            if let Some((pct_str, msg)) = trimmed.split_once('|') {
                let pct: f32 = pct_str.parse().unwrap_or(0.0);
                (pct.clamp(0.0, 100.0) / 100.0, msg.to_string())
            } else {
                (0.0, trimmed.to_string())
            }
        }
        Err(_) => (0.0, String::new()),
    }
}

pub fn clear_progress() {
    let _ = std::fs::remove_file(progress_file_path());
    set_install_running(false);
}

fn clear_progress_file_only() {
    let _ = std::fs::remove_file(progress_file_path());
}

pub fn cleanup_stale_progress() {
    let path = progress_file_path();
    if std::path::Path::new(&path).exists() {
        debug_log!("[settings] removing stale progress file");
        let _ = std::fs::remove_file(&path);
    }
    set_install_running(false);
}

/// Shell functions shared by the setup scripts. `download_model <percent>`
/// makes sure `$2` is a whisper.cpp model, downloading model `$1` (size `$3`)
/// with voxtype when it isn't. Arguments come from `spawn_script`.
macro_rules! model_download_functions {
    () => {
        concat!(
            "MODEL=\"$1\"; MODEL_FILE=\"$2\"; MODEL_SIZE=\"$3\"\n",
            "MODEL_URL=\"https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${MODEL_FILE##*/}\"\n",
            "# whisper.cpp model files start with the GGML magic \"lmgg\".\n",
            "model_ok() {\n",
            "    [[ -f \"$MODEL_FILE\" ]] &&\n",
            "        [[ \"$(od -An -tx1 -N4 -- \"$MODEL_FILE\" 2>/dev/null | tr -d ' \\n')\" == 6c6d6767 ]]\n",
            "}\n",
            "download_failed() {\n",
            "    echo '0|Error: Model download failed. Check your internet connection, press Back, then click Download model.' > \"$PROG\"\n",
            "    echo ''\n",
            "    echo \"  ERROR: Could not download the $MODEL model ($MODEL_SIZE).\"\n",
            "    echo '  It comes from Hugging Face, so it needs an internet connection:'\n",
            "    echo \"    $MODEL_URL\"\n",
            "    echo ''\n",
            "    echo '  Check your connection, then try again:'\n",
            "    echo '    - Settings > Dictation > Download model, or'\n",
            "    echo \"    - in a terminal: voxtype setup --download --model $MODEL\"\n",
            "    echo \"  Or download that file yourself and save it as $MODEL_FILE\"\n",
            "    echo ''\n",
            "    echo '  Press Enter to close.'; read -r; exit 1\n",
            "}\n",
            "download_model() {\n",
            "    if model_ok; then\n",
            "        echo '70|Model already downloaded' > \"$PROG\"\n",
            "        echo \"  The $MODEL model is already downloaded -- skipping download.\"\n",
            "        return\n",
            "    fi\n",
            "    # voxtype skips files that exist, so move an invalid one aside first.\n",
            "    if [[ -e \"$MODEL_FILE\" ]]; then\n",
            "        mv -f -- \"$MODEL_FILE\" \"$MODEL_FILE.invalid\" &&\n",
            "            echo \"  Moved an invalid model file aside: $MODEL_FILE.invalid\"\n",
            "    fi\n",
            "    echo \"$1|Downloading the $MODEL model ($MODEL_SIZE)...\" > \"$PROG\"\n",
            "    echo ''\n",
            "    echo \"  Downloading the $MODEL model ($MODEL_SIZE) from Hugging Face...\"\n",
            "    voxtype setup --download --no-post-install --model \"$MODEL\" 2>&1 || download_failed\n",
            "    # curl saves HTTP error pages too; only a whisper.cpp model counts.\n",
            "    if ! model_ok; then\n",
            "        [[ -e \"$MODEL_FILE\" ]] && mv -f -- \"$MODEL_FILE\" \"$MODEL_FILE.invalid\"\n",
            "        download_failed\n",
            "    fi\n",
            "}\n",
        )
    };
}

/// Runs a setup script in a terminal with the arguments `download_model`
/// needs. Returns false, with the reason in the progress file, if it can't.
fn spawn_script(script: &str, model_id: &str) -> bool {
    let error = match models_dir().and_then(|dir| model_file(model_id, &dir)) {
        None => "Error: Could not find voxtype's models directory",
        Some(file) => match Command::new("terminal")
            .args(["-e", "bash", "-c", script, "bash", model_id])
            .arg(file)
            .arg(download_size(model_id).unwrap_or("unknown size"))
            .spawn()
        {
            Ok(_) => return true,
            Err(e) => {
                eprintln!("[settings] failed to spawn terminal: {}", e);
                "Error: Could not open terminal"
            }
        },
    };
    set_install_running(false);
    let _ = std::fs::write(progress_file_path(), format!("0|{error}"));
    false
}

pub fn launch_install(model_id: &str) -> bool {
    if INSTALL_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        debug_log!("[settings] install already in progress, ignoring");
        return false;
    }
    debug_log!("[settings] launching install in terminal");
    clear_progress_file_only();
    let script = concat!(
        "PROG=\"${XDG_RUNTIME_DIR:-/tmp}/settings-install-progress\"\n",
        "cleanup() { echo \"0|Error: Interrupted\" > \"$PROG\"; exit 1; }\n",
        "trap cleanup INT TERM\n",
        model_download_functions!(),
        "echo ''\n",
        "echo '  Setting up dictation...'\n",
        "\n",
        "# Prime bundled model cache (fast, idempotent)\n",
        "echo '3|Priming model cache...' > \"$PROG\"\n",
        "if command -v dictation-prime &>/dev/null; then\n",
        "    dictation-prime 2>/dev/null || true\n",
        "fi\n",
        "\n",
        "# If voxtype is already installed, skip package installation entirely\n",
        "if command -v voxtype &>/dev/null && command -v wtype &>/dev/null; then\n",
        "    echo '30|Packages already installed' > \"$PROG\"\n",
        "    echo '  Packages already installed.'\n",
        "else\n",
        "    echo '  Installing dictation packages...'\n",
        "    echo '  You may be prompted for your password once.'\n",
        "    echo ''\n",
        "    sudo -v || { echo '0|Error: Authentication failed' > \"$PROG\"; exit 1; }\n",
        "    while sudo -vn 2>/dev/null; do sleep 50; done &\n",
        "    SUDO_KEEPALIVE=$!\n",
        "    trap 'kill $SUDO_KEEPALIVE 2>/dev/null; cleanup' INT TERM\n",
        "    echo '5|Installing packages...' > \"$PROG\"\n",
        "\n",
        "    install_pkg() {\n",
        "        local pkg=\"$1\"\n",
        "        if command -v paru &>/dev/null && paru --version &>/dev/null 2>&1; then\n",
        "            echo \"  Using paru for $pkg...\"\n",
        "            paru -S --needed --noconfirm \"$pkg\" 2>&1 && return 0\n",
        "        fi\n",
        "        if command -v yay &>/dev/null && yay --version &>/dev/null 2>&1; then\n",
        "            echo \"  Using yay for $pkg...\"\n",
        "            yay -S --needed --noconfirm \"$pkg\" 2>&1 && return 0\n",
        "        fi\n",
        "        if pacman -Si \"$pkg\" &>/dev/null; then\n",
        "            echo \"  Using pacman for $pkg...\"\n",
        "            sudo pacman -S --needed --noconfirm \"$pkg\" 2>&1 && return 0\n",
        "        fi\n",
        "        echo \"  Building $pkg from AUR manually...\"\n",
        "        local tmp\n",
        "        tmp=$(mktemp -d)\n",
        "        if git clone --depth 1 \"https://aur.archlinux.org/${pkg}.git\" \"$tmp\" 2>/dev/null; then\n",
        "            (cd \"$tmp\" && makepkg -si --noconfirm 2>&1)\n",
        "            local rc=$?\n",
        "            rm -rf \"$tmp\"\n",
        "            return $rc\n",
        "        fi\n",
        "        rm -rf \"$tmp\"\n",
        "        return 1\n",
        "    }\n",
        "\n",
        "    echo '10|Installing wtype...' > \"$PROG\"\n",
        "    if ! install_pkg wtype; then\n",
        "        echo '0|Error: Could not install wtype' > \"$PROG\"\n",
        "        echo ''\n",
        "        echo '  ERROR: Could not install wtype.'\n",
        "        echo '  Press Enter to close.'; read -r; exit 1\n",
        "    fi\n",
        "    echo '20|Installing voxtype...' > \"$PROG\"\n",
        "    if ! install_pkg voxtype-bin; then\n",
        "        echo '0|Error: Could not install voxtype-bin' > \"$PROG\"\n",
        "        echo ''\n",
        "        echo '  ERROR: Could not install voxtype-bin.'\n",
        "        echo '  Press Enter to close.'; read -r; exit 1\n",
        "    fi\n",
        "    if ! command -v voxtype &>/dev/null; then\n",
        "        echo '0|Error: voxtype not found after install' > \"$PROG\"\n",
        "        echo ''\n",
        "        echo '  ERROR: voxtype command not found after install.'\n",
        "        echo '  The package may have failed to build.'\n",
        "        echo '  Press Enter to close.'; read -r; exit 1\n",
        "    fi\n",
        "    kill $SUDO_KEEPALIVE 2>/dev/null || true\n",
        "fi\n",
        "\n",
        "download_model 40\n",
        "echo '85|Setting up service...' > \"$PROG\"\n",
        "echo ''\n",
        "echo '  Setting up systemd service...'\n",
        "if [[ ! -f \"$HOME/.config/systemd/user/voxtype.service\" ]]; then\n",
        "    voxtype setup systemd 2>/dev/null || true\n",
        "fi\n",
        "systemctl --user daemon-reload 2>/dev/null || true\n",
        "systemctl --user enable voxtype 2>/dev/null || true\n",
        "systemctl --user restart voxtype 2>/dev/null || true\n",
        "mkdir -p \"$HOME/.config/smplos\"\n",
        "touch \"$HOME/.config/smplos/.dictation-primed\"\n",
        "echo '100|Done! Dictation is ready.' > \"$PROG\"\n",
        "echo ''\n",
        "echo '  Done! Dictation is ready.'\n",
        "echo '  Press SUPER+CTRL+X to start/stop speaking.'\n",
        "echo ''\n",
        "echo '  You can close this window now.'\n",
        "read -r\n",
    );
    spawn_script(script, model_id)
}

pub fn launch_model_download(model_id: &str) -> bool {
    if INSTALL_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        debug_log!("[settings] install already in progress, ignoring");
        return false;
    }
    debug_log!("[settings] launching model download in terminal");
    clear_progress_file_only();
    let script = concat!(
        "PROG=\"${XDG_RUNTIME_DIR:-/tmp}/settings-install-progress\"\n",
        "cleanup() { echo \"0|Error: Interrupted\" > \"$PROG\"; exit 1; }\n",
        "trap cleanup INT TERM\n",
        model_download_functions!(),
        "if ! command -v voxtype &>/dev/null; then\n",
        "    echo '0|Error: voxtype not found' > \"$PROG\"\n",
        "    echo '  ERROR: voxtype is not installed.'\n",
        "    echo '  Press Enter to close.'; read -r; exit 1\n",
        "fi\n",
        "download_model 10\n",
        "echo '80|Restarting service...' > \"$PROG\"\n",
        "echo ''\n",
        "echo '  Restarting dictation service...'\n",
        "systemctl --user enable voxtype 2>/dev/null || true\n",
        "systemctl --user restart voxtype 2>/dev/null || true\n",
        "echo '100|Done! Model updated.' > \"$PROG\"\n",
        "echo ''\n",
        "echo '  Done! You can close this window.'\n",
        "read -r\n",
    );
    spawn_script(script, model_id)
}

pub fn open_config() {
    let cfg = match config_path() {
        Some(p) => p,
        None => return,
    };
    if !std::path::Path::new(&cfg).exists() {
        write_config("en", "base", false);
    }
    let editor = if Command::new("which").arg("nvim")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success()).unwrap_or(false)
    { "nvim" } else { "nano" };
    debug_log!("[settings] opening config with {}", editor);
    let _ = Command::new("terminal").args(["-e", editor, &cfg]).spawn();
}

pub fn restart_service() {
    debug_log!("[settings] restarting voxtype service");
    let _ = Command::new("systemctl")
        .args(["--user", "enable", "voxtype"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let _ = Command::new("systemctl")
        .args(["--user", "restart", "voxtype"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;
    use std::sync::atomic::AtomicU64;

    /// The start of a real whisper.cpp model file.
    const GGML: &[u8] = b"lmgg\x99\xca\x00\x00";

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "settings-dictation-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("models")).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            path
        }

        fn progress(&self) -> String {
            fs::read_to_string(self.0.join("progress")).unwrap_or_default()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(model: &str) -> DictationConfig {
        parse_config(&format!("[whisper]\nmodel = \"{model}\"\n"))
    }

    #[test]
    fn model_names_resolve_to_the_files_voxtype_loads() {
        let dir = Path::new("/models");
        for (model, file) in [
            ("base.en", "/models/ggml-base.en.bin"),
            ("large-v3-turbo", "/models/ggml-large-v3-turbo.bin"),
            ("large", "/models/ggml-large-v1.bin"),
            ("base_en_acft_q8_0.bin", "/models/base_en_acft_q8_0.bin"),
            ("/opt/whisper/custom.bin", "/opt/whisper/custom.bin"),
        ] {
            assert_eq!(model_file(model, dir), Some(PathBuf::from(file)), "{model}");
        }
        assert_eq!(model_file("huge", dir), None);
        assert!(!is_downloadable("large-v2"));
        for model in MODELS {
            assert!(is_downloadable(model.id), "{} can't be downloaded", model.id);
            assert_eq!(download_size(model.id), Some(model.size), "{} size", model.id);
        }
    }

    #[test]
    fn only_whisper_cpp_model_files_count_as_downloaded() {
        let fx = Fixture::new();
        assert_eq!(model_status(&fx.0.join("ggml-base.bin")), ModelStatus::Missing);
        assert_eq!(model_status(&fx.write("ggml-base.bin", GGML)), ModelStatus::Ready);
        assert_eq!(model_status(&fx.write("error.bin", b"Entry not found")), ModelStatus::Invalid);
        assert_eq!(model_status(&fx.write("empty.bin", b"")), ModelStatus::Invalid);
        // smplOS's bundled faster-whisper (CTranslate2) model.bin can't be loaded by voxtype.
        assert_eq!(model_status(&fx.write("model.bin", &[6, 0, 0, 0, 1])), ModelStatus::Invalid);
        fs::create_dir(fx.0.join("dir.bin")).unwrap();
        assert_eq!(model_status(&fx.0.join("dir.bin")), ModelStatus::Invalid);
    }

    #[test]
    fn missing_model_notice_says_where_and_how_to_get_it_until_downloaded() {
        let fx = Fixture::new();
        let dir = fx.0.join(".local/share/voxtype/models");
        let notice = model_notice(&config("base.en"), &dir, Some(&fx.0));
        assert_eq!(
            notice.problem,
            "The Base (English) model isn't downloaded, so dictation can't start."
        );
        assert_eq!(
            notice.help,
            "Download model fetches ggml-base.en.bin (~150 MB) from \
             huggingface.co/ggerganov/whisper.cpp into ~/.local/share/voxtype/models; \
             needs internet. Terminal: voxtype setup --download --model base.en"
        );
        assert!(notice.downloadable);
        assert_eq!(downloaded_models(&dir), vec![false; MODELS.len()]);

        fx.write(".local/share/voxtype/models/ggml-base.en.bin", GGML);
        assert_eq!(model_notice(&config("base.en"), &dir, Some(&fx.0)), ModelNotice::default());
        let downloaded = downloaded_models(&dir);
        for (model, downloaded) in MODELS.iter().zip(downloaded) {
            assert_eq!(downloaded, model.id == "base.en", "{}", model.id);
        }
    }

    #[test]
    fn invalid_and_custom_model_files_get_matching_advice() {
        let fx = Fixture::new();
        let dir = fx.0.join("models");
        fx.write("models/ggml-small.bin", b"<!DOCTYPE html>");
        let notice = model_notice(&config("small"), &dir, None);
        assert_eq!(
            notice.problem,
            "The Small model file isn't a valid Whisper model, so dictation can't start."
        );
        assert_eq!(
            notice.help,
            "Download model moves it aside and fetches ggml-small.bin (~500 MB) again from \
             huggingface.co/ggerganov/whisper.cpp; needs internet."
        );
        assert!(notice.downloadable);

        // voxtype can't download a custom file, so say where it looks for it.
        let notice = model_notice(&config("base_en_acft_q8_0.bin"), &dir, Some(&fx.0));
        assert_eq!(notice.problem, "voxtype can't find the model file, so dictation can't start.");
        assert_eq!(
            notice.help,
            "It looks for ~/models/base_en_acft_q8_0.bin. Put a whisper.cpp model there, \
             or pick another model under Reconfigure."
        );
        assert!(!notice.downloadable);
        fx.write("models/base_en_acft_q8_0.bin", GGML);
        assert_eq!(model_notice(&config("base_en_acft_q8_0.bin"), &dir, None), ModelNotice::default());

        let notice = model_notice(&config("huge"), &dir, None);
        assert_eq!(notice.problem, "voxtype doesn't know the model \"huge\", so dictation can't start.");
        assert!(!notice.downloadable);
    }

    #[test]
    fn notice_only_applies_when_voxtype_loads_a_local_whisper_model() {
        let fx = Fixture::new();
        for text in [
            "engine = \"parakeet\"\n[whisper]\nmodel = \"base\"\n",
            "[whisper]\nmodel = \"base\"\nmode = \"remote\"\n",
            "[whisper]\nmodel = \"base\"\nbackend = \"remote\"\n",
        ] {
            assert_eq!(model_notice(&parse_config(text), &fx.0, None), ModelNotice::default(), "{text}");
        }
        // whisper-cli mode loads the same file, and `mode` wins over the deprecated `backend`.
        for text in [
            "[whisper]\nmodel = \"base\"\nmode = \"cli\"\n",
            "engine = \"whisper\"\n[whisper]\nmodel = \"base\"\nmode = \"local\"\nbackend = \"remote\"\n",
        ] {
            assert!(!model_notice(&parse_config(text), &fx.0, None).problem.is_empty(), "{text}");
        }
    }

    #[test]
    fn config_reading_follows_voxtype_keys_sections_and_defaults() {
        assert_eq!(parse_config("[whisper]\nlanguage = \"en\"\n").model, "base.en");
        let cfg = parse_config(
            "# engine = \"parakeet\"\n\
             [hotkey]\n\
             model_modifier = \"LEFTSHIFT\"\n\
             [whisper]\n\
             model = \"small\" # better accuracy\n\
             language = [\"en\", \"fr\"]\n\
             [meeting.summary]\n\
             backend = \"remote\"\n\
             [vad]\n\
             model = \"/models/ggml-silero-vad.bin\"\n",
        );
        assert_eq!(cfg.model, "small");
        assert!(cfg.uses_local_model());
        assert_eq!(cfg.primary_code, "fr");
        assert!(cfg.also_english);
    }

    #[test]
    fn written_config_uses_the_system_microphone_and_reads_back() {
        let text = config_text("fr", "small", true);
        assert!(text.contains("[audio]\ndevice = \"default\"\n"), "{text}");
        assert!(!text.contains("\"auto\"\nsample_rate"));
        let cfg = parse_config(&text);
        assert_eq!(cfg.model, "small");
        assert_eq!(cfg.primary_code, "fr");
        assert!(cfg.also_english);
        assert!(cfg.uses_local_model());
    }

    /// Runs the setup scripts' `download_model` step for base.en with a stub
    /// `voxtype` that runs `stub`.
    fn run_download_step(fx: &Fixture, stub: &str) -> (bool, String) {
        let voxtype = fx.write(
            "bin/voxtype",
            format!("#!/bin/bash\necho \"$*\" >> '{}'\n{stub}\n", fx.0.join("calls").display())
                .as_bytes(),
        );
        fs::set_permissions(&voxtype, fs::Permissions::from_mode(0o755)).unwrap();
        let script = concat!("PROG=\"$PROGRESS\"\n", model_download_functions!(), "download_model 10\n");
        let path = format!("{}:{}", fx.0.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let output = Command::new("bash")
            .args(["-c", script, "bash", "base.en"])
            .arg(fx.0.join("models/ggml-base.en.bin"))
            .arg("~150 MB")
            .env("PATH", path)
            .env("PROGRESS", fx.0.join("progress"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned())
    }

    #[test]
    fn download_step_keeps_a_ready_model_and_downloads_a_missing_or_invalid_one() {
        let fx = Fixture::new();
        fx.write("models/ggml-base.en.bin", GGML);
        let (ok, out) = run_download_step(&fx, "exit 1");
        assert!(ok, "{out}");
        assert!(!fx.0.join("calls").exists(), "voxtype ran for a ready model");
        assert_eq!(fx.progress(), "70|Model already downloaded\n");

        let fx = Fixture::new();
        let model = fx.0.join("models/ggml-base.en.bin");
        let stub = format!("printf 'lmgg\\0\\0' > '{}'", model.display());
        // A file voxtype would treat as present is moved aside, not deleted.
        fx.write("models/ggml-base.en.bin", b"<html>");
        let (ok, out) = run_download_step(&fx, &stub);
        assert!(ok, "{out}");
        assert_eq!(
            fs::read_to_string(fx.0.join("calls")).unwrap(),
            "setup --download --no-post-install --model base.en\n"
        );
        assert_eq!(model_status(&model), ModelStatus::Ready);
        assert_eq!(fs::read(fx.0.join("models/ggml-base.en.bin.invalid")).unwrap(), b"<html>");
        assert_eq!(fx.progress(), "10|Downloading the base.en model (~150 MB)...\n");
    }

    #[test]
    fn failed_downloads_say_what_to_do_next() {
        for saves_error_page in [false, true] {
            let fx = Fixture::new();
            let model = fx.0.join("models/ggml-base.en.bin");
            // curl without --fail saves an HTTP error page as the model and exits 0.
            let stub = if saves_error_page {
                format!("echo 'Entry not found' > '{}'", model.display())
            } else {
                "exit 1".to_string()
            };
            let (ok, out) = run_download_step(&fx, &stub);
            assert!(!ok, "{out}");
            assert!(!model.exists());
            assert_eq!(
                fx.progress(),
                "0|Error: Model download failed. Check your internet connection, press Back, \
                 then click Download model.\n"
            );
            for advice in [
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
                "needs an internet connection",
                "Settings > Dictation > Download model",
                "voxtype setup --download --model base.en",
                &format!("save it as {}", model.display()),
            ] {
                assert!(out.contains(advice), "missing {advice:?} in:\n{out}");
            }
        }
    }
}
