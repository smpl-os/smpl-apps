//! Hyprland's fractional-scale rules (`CMonitor::applyMonitorRule`, v0.56).
//!
//! A scale is accepted unchanged only when the mode's width and height divide
//! into whole logical pixels. Otherwise Hyprland searches the k/120 grid
//! around the request (k, k+1, k-1, k+2, …) and silently uses the first
//! clean value, so Settings snaps to that value itself and the saved file,
//! the canvas and the live result agree.

pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;

fn clean(pixels: i32, scale: f64) -> bool {
    let logical = pixels as f64 / scale;
    logical == logical.round()
}

/// Both axes divide into whole logical pixels (Hyprland's exact test).
pub fn is_exact(width: i32, height: i32, scale: f64) -> bool {
    scale.is_finite() && scale > 0.0 && clean(width, scale) && clean(height, scale)
}

/// The scale Hyprland ends up using for an explicit `requested` scale, or
/// `None` when it finds no clean divisor and falls back to its default.
pub fn hyprland_effective(width: i32, height: i32, requested: f64) -> Option<f64> {
    let requested = requested as f32;
    if is_exact(width, height, requested as f64) {
        return Some(requested as f64);
    }
    let search = ((requested as f64) * 120.0).round() as f32;
    let zero = search as f64 / 120.0;
    if is_exact(width, height, zero) {
        return Some(zero as f32 as f64);
    }
    for step in 1..90 {
        let up = (search + step as f32) as f64 / 120.0;
        if is_exact(width, height, up) {
            return Some(up as f32 as f64);
        }
        let down = (search - step as f32) as f64 / 120.0;
        if is_exact(width, height, down) {
            return Some(down as f32 as f64);
        }
    }
    None
}

/// Nearest scale in [`MIN_SCALE`, `MAX_SCALE`] that Hyprland keeps unchanged
/// for this mode, searched in Hyprland's own order. 1.0 is always clean.
pub fn snap(width: i32, height: i32, requested: f64) -> f64 {
    let low = (MIN_SCALE * 120.0).round() as i64;
    let high = (MAX_SCALE * 120.0).round() as i64;
    let start = (requested.clamp(MIN_SCALE, MAX_SCALE) * 120.0).round() as i64;
    for distance in 0..=(high - low) {
        let candidates = if distance == 0 {
            [Some(start), None]
        } else {
            [Some(start + distance), Some(start - distance)]
        };
        for k in candidates.into_iter().flatten() {
            if (low..=high).contains(&k) && is_exact(width, height, k as f64 / 120.0) {
                return k as f64 / 120.0;
            }
        }
    }
    1.0
}

/// The real scale behind hyprctl's two-decimal print: the clean k/120 value
/// within print rounding of `reported` (e.g. 1.33 → 4/3), else `reported`.
pub fn recover_live(width: i32, height: i32, reported: f64) -> f64 {
    if !reported.is_finite() || reported <= 0.0 {
        return 1.0;
    }
    let low = ((reported - 0.0051) * 120.0).ceil().max(1.0) as i64;
    let high = ((reported + 0.0051) * 120.0).floor() as i64;
    (low..=high)
        .map(|k| k as f64 / 120.0)
        .filter(|&scale| is_exact(width, height, scale))
        .min_by(|a, b| (a - reported).abs().total_cmp(&(b - reported).abs()))
        .unwrap_or(reported)
}

/// Shortest decimal form, at most six places: 1, 1.25, 1.333333.
pub fn format(scale: f64) -> String {
    let text = format!("{scale:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactness_matches_hyprland_examples() {
        assert!(!is_exact(2560, 1440, 1.5));
        assert!(is_exact(1920, 1080, 1.5));
        assert!(is_exact(3840, 2160, 1.5));
        assert!(is_exact(2560, 1440, 1.25));
        assert!(is_exact(2560, 1440, 160.0 / 120.0));
    }

    #[test]
    fn hyprland_fixup_replaces_unclean_scales() {
        let fixed = hyprland_effective(2560, 1440, 1.5).unwrap();
        assert!((fixed - 1.6).abs() < 1e-6, "{fixed}");
        assert!((hyprland_effective(1920, 1080, 1.5).unwrap() - 1.5).abs() < 1e-9);
        let third = hyprland_effective(2560, 1440, 1.333333).unwrap();
        assert!((third - 4.0 / 3.0).abs() < 1e-6, "{third}");
    }

    #[test]
    fn snapping_follows_hyprland_search_within_range() {
        assert!((snap(2560, 1440, 1.5) - 1.6).abs() < 1e-9);
        assert_eq!(snap(1920, 1080, 1.5), 1.5);
        assert_eq!(snap(3840, 2160, 1.5), 1.5);
        assert_eq!(snap(2560, 1440, 1.25), 1.25);
        assert!((snap(2560, 1440, 1.75) - 200.0 / 120.0).abs() < 1e-9);
        assert_eq!(snap(1920, 1080, 9.0), 3.0);
        assert_eq!(snap(1920, 1080, 0.1), 0.5);
        for requested in [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 2.75, 3.0] {
            for (w, h) in [
                (2560, 1440),
                (1920, 1080),
                (3840, 2160),
                (1366, 768),
                (2880, 1800),
            ] {
                let snapped = snap(w, h, requested);
                assert!(is_exact(w, h, snapped), "{w}x{h} {requested} -> {snapped}");
                assert!((MIN_SCALE..=MAX_SCALE).contains(&snapped));
                let applied = hyprland_effective(w, h, format(snapped).parse().unwrap()).unwrap();
                assert!(
                    (applied - snapped).abs() < 1e-6,
                    "{w}x{h} {snapped} -> {applied}"
                );
            }
        }
    }

    #[test]
    fn live_scale_is_recovered_from_two_printed_decimals() {
        assert!((recover_live(2560, 1440, 1.33) - 4.0 / 3.0).abs() < 1e-12);
        assert!((recover_live(2560, 1440, 1.67) - 200.0 / 120.0).abs() < 1e-12);
        assert_eq!(recover_live(2560, 1440, 1.6), 1.6);
        assert_eq!(recover_live(1920, 1080, 1.0), 1.0);
        assert_eq!(recover_live(1920, 1080, f64::NAN), 1.0);
    }

    #[test]
    fn written_scales_are_short_and_exact_enough() {
        assert_eq!(format(1.0), "1");
        assert_eq!(format(1.25), "1.25");
        assert_eq!(format(4.0 / 3.0), "1.333333");
        assert_eq!(format(1.6), "1.6");
    }
}
