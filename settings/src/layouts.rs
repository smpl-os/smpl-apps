use crate::debug_log;
use std::io::BufRead;
use std::process::Command;

/// OS keyboard config path: ~/.config/hypr/input.conf
fn input_conf_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".config/hypr/input.conf"))
}

/// Remove old legacy kb-center config file if it still exists.
/// OS config (`input.conf`) is the single source of truth.
pub fn cleanup_legacy_config() {
    if let Ok(home) = std::env::var("HOME") {
        let legacy = std::path::PathBuf::from(home).join(".config/kb-center/layouts.conf");
        if legacy.exists() {
            match std::fs::remove_file(&legacy) {
                Ok(_) => debug_log!("[settings] removed legacy config {:?}", legacy),
                Err(e) => eprintln!("[settings] failed to remove legacy config {:?}: {}", legacy, e),
            }
        }
    }
}

/// Load active layouts from OS config (`~/.config/hypr/input.conf`).
pub fn load_from_os_config(available: &[AvailableLayout]) -> Option<Vec<ActiveLayout>> {
    let path = input_conf_path()?;
    debug_log!("[settings] load_from_os_config: trying {:?}", path);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return None,
    };

    let mut layout_str = String::new();
    let mut variant_str = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("kb_layout") {
            if let Some((_, rhs)) = rest.split_once('=') {
                layout_str = rhs.trim().replace(' ', "");
            }
        } else if let Some(rest) = trimmed.strip_prefix("kb_variant") {
            if let Some((_, rhs)) = rest.split_once('=') {
                variant_str = rhs.trim().replace(' ', "");
            }
        }
    }

    if layout_str.is_empty() {
        return None;
    }

    let mut result = Vec::new();
    let codes: Vec<&str> = layout_str.split(',').collect();
    let variants: Vec<&str> = variant_str.split(',').collect();
    for (i, code_raw) in codes.iter().enumerate() {
        let code = code_raw.trim().to_string();
        if code.is_empty() {
            continue;
        }
        let variant = variants.get(i).map(|v| v.trim().to_string()).unwrap_or_default();
        let description = describe(available, &code, &variant);
        result.push(ActiveLayout { code, variant, description });
    }

    if result.is_empty() {
        None
    } else {
        validate_layout_variants(available, &mut result);
        debug_log!("[settings] load_from_os_config: {} layouts", result.len());
        Some(result)
    }
}

/// An available XKB layout (or layout+variant) from evdev.lst.
#[derive(Clone, Debug)]
pub struct AvailableLayout {
    pub code: String,
    pub variant: String,
    pub description: String,
}

impl AvailableLayout {
    pub fn display(&self) -> &str {
        &self.description
    }
}

/// A currently-configured layout (kept in memory, synced to Hyprland when possible).
#[derive(Clone, Debug)]
pub struct ActiveLayout {
    pub code: String,
    pub variant: String,
    pub description: String,
}

/// Parse /usr/share/X11/xkb/rules/evdev.lst to get all available layouts + variants.
pub fn list_available_layouts() -> Vec<AvailableLayout> {
    let path = "/usr/share/X11/xkb/rules/evdev.lst";
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };

    let reader = std::io::BufReader::new(file);
    let mut layouts: Vec<AvailableLayout> = Vec::new();
    let mut section = "";

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();

        if trimmed.starts_with("! ") {
            section = match trimmed {
                "! layout" => "layout",
                "! variant" => "variant",
                "! option" => break,
                _ => "",
            };
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match section {
            "layout" => {
                let mut parts = trimmed.splitn(2, char::is_whitespace);
                let code = parts.next().unwrap_or("").trim().to_string();
                let desc = parts.next().unwrap_or("").trim().to_string();
                if !code.is_empty() {
                    layouts.push(AvailableLayout {
                        code,
                        variant: String::new(),
                        description: desc,
                    });
                }
            }
            "variant" => {
                let mut parts = trimmed.splitn(2, char::is_whitespace);
                let variant = parts.next().unwrap_or("").trim().to_string();
                let rest = parts.next().unwrap_or("").trim();
                if let Some((code_part, desc)) = rest.split_once(':') {
                    let code = code_part.trim().to_string();
                    let desc = desc.trim().to_string();
                    if !code.is_empty() && !variant.is_empty() {
                        layouts.push(AvailableLayout {
                            code,
                            variant,
                            description: desc,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    layouts.sort_by(|a, b| a.description.to_lowercase().cmp(&b.description.to_lowercase()));
    layouts
}

/// Look up a human-readable description for a layout code + variant.
pub fn describe(available: &[AvailableLayout], code: &str, variant: &str) -> String {
    available
        .iter()
        .find(|a| a.code == code && a.variant == variant)
        .map(|a| a.description.clone())
        .unwrap_or_else(|| {
            if variant.is_empty() {
                code.to_string()
            } else {
                format!("{} ({})", code, variant)
            }
        })
}

/// Check whether `variant` is a known XKB variant for `code`.
/// An empty variant is always valid (base layout).
fn is_valid_variant(available: &[AvailableLayout], code: &str, variant: &str) -> bool {
    if variant.is_empty() {
        return true;
    }
    available.iter().any(|a| a.code == code && a.variant == variant)
}

/// Validate and repair layout↔variant assignments parsed from a config file.
///
/// Hyprland uses positional CSV: `kb_layout = us,ru` + `kb_variant = ,phonetic`
/// means "us" with no variant and "ru" with "phonetic".
///
/// If a variant is invalid for its positional layout but valid for another
/// layout in the list, move it there.  Otherwise drop it.
fn validate_layout_variants(available: &[AvailableLayout], layouts: &mut [ActiveLayout]) {
    // Collect indices with invalid variants and their orphaned variant names.
    let mut orphans: Vec<(usize, String)> = Vec::new();
    for (i, l) in layouts.iter().enumerate() {
        if !l.variant.is_empty() && !is_valid_variant(available, &l.code, &l.variant) {
            orphans.push((i, l.variant.clone()));
        }
    }
    if orphans.is_empty() {
        return;
    }

    // Try to rehome each orphan variant to a layout that accepts it.
    for (bad_idx, variant) in &orphans {
        layouts[*bad_idx].variant.clear();
        // Find the first layout (without a variant) that accepts this variant.
        if let Some(target) = layouts.iter_mut().find(|l| {
            l.variant.is_empty() && is_valid_variant(available, &l.code, variant)
        }) {
            debug_log!("[settings] rehomed variant '{}' → layout '{}'", variant, target.code);
            target.variant = variant.clone();
        } else {
            debug_log!("[settings] dropped invalid variant '{}' from layout '{}'", variant, layouts[*bad_idx].code);
        }
    }

    // Refresh descriptions after any changes.
    for l in layouts.iter_mut() {
        l.description = describe(available, &l.code, &l.variant);
    }
}

/// Try to read currently configured layouts from Hyprland.
/// Returns None if hyprctl is unavailable or fails.
pub fn load_from_hyprland(available: &[AvailableLayout]) -> Option<Vec<ActiveLayout>> {
    debug_log!("[settings] load_from_hyprland: querying hyprctl...");
    let layout_str = hyprctl_get_option("input:kb_layout")?;
    if layout_str.is_empty() {
        return None;
    }

    let variant_str = hyprctl_get_option("input:kb_variant").unwrap_or_default();
    debug_log!("[settings] hyprland: layouts={:?} variants={:?}", layout_str, variant_str);
    parse_layout_variant_csv(available, &layout_str, &variant_str)
}

/// Compositor-agnostic fallback: use setxkbmap -query to detect current layouts.
pub fn load_from_system(available: &[AvailableLayout]) -> Option<Vec<ActiveLayout>> {
    debug_log!("[settings] load_from_system: setxkbmap -query");
    let out = Command::new("setxkbmap")
        .args(["-query"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut layout_str = String::new();
    let mut variant_str = String::new();

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("layout:") {
            layout_str = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("variant:") {
            variant_str = rest.trim().to_string();
        }
    }

    debug_log!("[settings] system: layouts={:?} variants={:?}", layout_str, variant_str);
    if layout_str.is_empty() {
        return None;
    }

    parse_layout_variant_csv(available, &layout_str, &variant_str)
}

/// Parse comma-separated layout and variant strings into ActiveLayout entries.
fn parse_layout_variant_csv(
    available: &[AvailableLayout],
    layout_str: &str,
    variant_str: &str,
) -> Option<Vec<ActiveLayout>> {
    let codes: Vec<&str> = layout_str.split(',').collect();
    let variants: Vec<&str> = variant_str.split(',').collect();

    let mut result: Vec<ActiveLayout> = codes
        .iter()
        .enumerate()
        .map(|(i, code)| {
            let code = code.trim().to_string();
            let variant = variants.get(i).map(|v| v.trim().to_string()).unwrap_or_default();
            let description = describe(available, &code, &variant);
            ActiveLayout { code, variant, description }
        })
        .collect();

    validate_layout_variants(available, &mut result);
    Some(result)
}

/// Push the active layout list to the running compositor (best-effort).
/// Also updates input.conf so layouts persist across reboots.
pub fn sync_to_compositor(active: &[ActiveLayout]) {
    if active.is_empty() {
        return;
    }

    let valid: Vec<&ActiveLayout> = active.iter().filter(|a| !a.code.trim().is_empty()).collect();
    if valid.is_empty() {
        return;
    }

    let layouts: String = valid.iter().map(|a| a.code.as_str()).collect::<Vec<_>>().join(",");
    let variants: String = valid.iter().map(|a| a.variant.as_str()).collect::<Vec<_>>().join(",");

    update_input_conf(&layouts, &variants);

    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        debug_log!("[settings] sync: hyprctl {} {}", layouts, variants);
        hyprctl_keyword("input:kb_layout", &layouts);
        hyprctl_keyword("input:kb_variant", &variants);
    } else {
        debug_log!("[settings] sync: setxkbmap {} {}", layouts, variants);
        let mut args = vec!["-layout", &layouts];
        if !variants.replace(',', "").is_empty() {
            args.extend_from_slice(&["-variant", &variants]);
        }
        let _ = Command::new("setxkbmap").args(&args).output();
    }
}

/// Update ~/.config/hypr/input.conf with the active layouts so they persist.
///
/// Guarantees before writing:
///   1. No empty layout codes (e.g. ",ru" is rejected)
///   2. Variant count == layout count (padded or truncated)
///   3. The resulting keymap compiles under XKB
fn update_input_conf(layouts: &str, variants: &str) {
    let clean_layouts: String = layouts.split(',').filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(",");
    if clean_layouts.is_empty() {
        debug_log!("[settings] update_input_conf: refusing to write empty layout");
        return;
    }

    let layout_count = clean_layouts.split(',').count();
    let mut variant_parts: Vec<String> = variants
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    if variant_parts.len() < layout_count {
        variant_parts.resize(layout_count, String::new());
    } else if variant_parts.len() > layout_count {
        variant_parts.truncate(layout_count);
    }
    let clean_variants = variant_parts.join(",");

    // Verify the keymap compiles before writing to the config file.
    // This prevents boot-time XKB errors from bad layout/variant combos.
    if let Ok(out) = Command::new("xkbcli")
        .args(["compile-keymap", "--layout", &clean_layouts, "--variant", &clean_variants])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!("[settings] update_input_conf: XKB compile failed for layout={} variant={}: {}",
                clean_layouts, clean_variants, stderr.lines().next().unwrap_or("unknown error"));
            debug_log!("[settings] update_input_conf: REFUSED to write invalid keymap");
            return;
        }
    }

    let layouts = clean_layouts.as_str();
    let variants = clean_variants.as_str();

    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let path = format!("{}/.config/hypr/input.conf", home);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut result = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("kb_layout") && trimmed.contains('=') {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            result.push_str(&format!("{}kb_layout = {}", indent, layouts));
        } else if trimmed.starts_with("kb_variant") && trimmed.contains('=') {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            result.push_str(&format!("{}kb_variant = {}", indent, variants));
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }

    match std::fs::write(&path, &result) {
        Ok(_) => debug_log!("[settings] update_input_conf: OK"),
        Err(e) => eprintln!("[settings] update_input_conf failed: {}", e),
    }
}

/// Detect current layouts from the running compositor.
pub fn load_from_compositor(available: &[AvailableLayout]) -> Option<Vec<ActiveLayout>> {
    debug_log!("[settings] load_from_compositor: wayland={}", std::env::var("WAYLAND_DISPLAY").is_ok());
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        load_from_hyprland(available)
            .or_else(|| load_from_system(available))
    } else {
        load_from_system(available)
    }
}

fn hyprctl_get_option(option: &str) -> Option<String> {
    let out = Command::new("hyprctl")
        .args(["getoption", option, "-j"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    json.get("str").and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn hyprctl_keyword(key: &str, value: &str) {
    let _ = Command::new("hyprctl").args(["keyword", key, value]).output();
}
