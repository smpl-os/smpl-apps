//! WIFI: URI standard — encoding, decoding, and pixel-buffer rendering.
//!
//! The WIFI: URI format (also called MECARD Wi-Fi) is understood by iOS, Android,
//! and most QR-code scanner apps.  The canonical form is:
//!
//! ```text
//! WIFI:T:<auth>;S:<ssid>;P:<password>;;
//! ```
//!
//! where `<auth>` is one of `WPA`, `WPA2`, `WPA3`, `WEP`, or `nopass`.
//!
//! Special characters in field values must be escaped with a backslash:
//! `\;`, `\,`, `\\`, `\"`, and `\:`.
//!
//! # Memory safety
//!
//! * [`wifi_uri`] returns a [`SecretString`] so the URI (which embeds the
//!   password in plaintext) is zeroed on drop.
//! * [`parse_wifi_uri`] returns the password field as a [`SecretString`].
//! * Intermediate `String` buffers that hold escaped passwords are wrapped
//!   in [`zeroize::Zeroizing`] so they are zeroed when they go out of scope.

use zeroize::Zeroizing;

use qrcode::{Color as QrColor, QrCode};

use super::secure::SecretString;

// ── Auth type ─────────────────────────────────────────────────────────────────

/// The authentication type embedded in a WIFI: URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiAuth {
    /// Full passphrase-based AP (maps to "WPA" in the URI — phones accept this
    /// for WPA2 and WPA3 networks).
    Wpa,
    /// Legacy WEP key.
    Wep,
    /// Open / unprotected network.
    Open,
}

impl WifiAuth {
    /// The string token used in the `T:` field.
    pub fn as_mecard_str(&self) -> &'static str {
        match self {
            WifiAuth::Wpa => "WPA",
            WifiAuth::Wep => "WEP",
            WifiAuth::Open => "nopass",
        }
    }

    /// Infer an auth type from an nmcli security description.
    pub fn from_nmcli_security(s: &str) -> Self {
        let s = s.to_uppercase();
        if s.contains("WPA") {
            WifiAuth::Wpa
        } else if s.contains("WEP") {
            WifiAuth::Wep
        } else {
            WifiAuth::Open
        }
    }
}

// ── URI helpers ───────────────────────────────────────────────────────────────

/// Escape special characters in a WIFI: URI field value.
fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '"' => out.push_str("\\\""),
            ':' => out.push_str("\\:"),
            other => out.push(other),
        }
    }
    out
}

/// Reverse of `escape_field`.
fn unescape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&nc) = chars.peek() {
                out.push(nc);
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split a WIFI: URI body (everything after `WIFI:`) on unescaped semicolons.
fn split_mecard_fields(s: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2; // skip escaped char
        } else if bytes[i] == b';' {
            fields.push(&s[start..i]);
            start = i + 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    if start < s.len() {
        fields.push(&s[start..]);
    }
    fields
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Build a WIFI: URI for embedding in a QR code.
///
/// Returns a [`SecretString`] so the URI (which embeds `password` in
/// plaintext) is zero-wiped from the heap when it is no longer needed.
///
/// `password` is ignored when `auth` is `WifiAuth::Open`.
pub fn wifi_uri(ssid: &str, password: &str, auth: &WifiAuth) -> SecretString {
    let escaped_ssid = escape_field(ssid); // not secret — plain String ok
    let uri = match auth {
        WifiAuth::Open => {
            format!("WIFI:T:nopass;S:{};;" , escaped_ssid)
        }
        _ => {
            // Wrap the escaped password in Zeroizing so it is cleared as soon
            // as the format! macro consumes it and the binding goes out of scope.
            let escaped_pass = Zeroizing::new(escape_field(password));
            format!(
                "WIFI:T:{};S:{};P:{};;" ,
                auth.as_mecard_str(),
                escaped_ssid,
                escaped_pass.as_str(),
            )
        }
    };
    // SecretString::from(String) moves the String into the zeroing wrapper.
    SecretString::from(uri)
}

/// Parse a WIFI: URI string into `(ssid, password, auth)`.
///
/// The password is returned as a [`SecretString`] which zeroes its heap
/// allocation on drop.  The intermediate unescaped password `String` is held
/// inside a [`Zeroizing`] wrapper while parsing, so it is also cleared.
///
/// Returns `None` if the string does not start with `WIFI:` or has no SSID.
pub fn parse_wifi_uri(uri: &str) -> Option<(String, SecretString, WifiAuth)> {
    let stripped = uri.trim().strip_prefix("WIFI:")?;

    let mut ssid = String::new();
    // Use Zeroizing<String> for the password field to zero the intermediate buffer.
    let mut raw_password: Zeroizing<String> = Zeroizing::new(String::new());
    let mut auth = WifiAuth::Open;

    for field in split_mecard_fields(stripped) {
        if let Some((key, val)) = field.split_once(':') {
            match key.trim() {
                "S" => ssid = unescape_field(val),
                "P" => {
                    // unescape_field returns a plain String; move it into the
                    // Zeroizing wrapper so any previous value is zeroed first.
                    *raw_password = unescape_field(val);
                }
                "T" => {
                    auth = match val.to_uppercase().as_str() {
                        "WPA" | "WPA2" | "WPA3" => WifiAuth::Wpa,
                        "WEP" => WifiAuth::Wep,
                        _ => WifiAuth::Open,
                    }
                }
                _ => {}
            }
        }
    }

    if ssid.is_empty() {
        return None;
    }
    // Convert the Zeroizing<String> into a SecretString.
    // raw_password will be zeroed when it drops at the end of this function.
    Some((ssid, SecretString::from(raw_password.as_str()), auth))
}

// ── QR pixel rendering ────────────────────────────────────────────────────────

/// Render `data` as a QR code and return an RGB pixel buffer.
///
/// Each QR module is upscaled to `scale × scale` pixels.  A quiet zone of
/// `quiet_modules` is added on all four sides (the QR standard requires ≥ 4).
///
/// Returns `(image_width_px, rgb_bytes)` where `rgb_bytes.len() == w * w * 3`.
/// Returns `None` if the data is too long to fit in any QR version.
pub fn generate_qr_pixels(data: &str, scale: u32, quiet_modules: u32) -> Option<(u32, Vec<u8>)> {
    let code = QrCode::new(data.as_bytes()).ok()?;
    let module_count = code.width() as u32;
    let total_modules = module_count + 2 * quiet_modules;
    let image_px = total_modules * scale;

    // Allocate an all-white image (RGB, 3 bytes per pixel).
    let mut pixels = vec![255u8; (image_px * image_px * 3) as usize];

    for row in 0..module_count {
        for col in 0..module_count {
            if code[(col as usize, row as usize)] == QrColor::Dark {
                // Paint a (scale × scale) dark square at the corresponding position.
                for py in 0..scale {
                    for px in 0..scale {
                        let x = (quiet_modules + col) * scale + px;
                        let y = (quiet_modules + row) * scale + py;
                        let base = ((y * image_px + x) * 3) as usize;
                        pixels[base] = 0;     // R
                        pixels[base + 1] = 0; // G
                        pixels[base + 2] = 0; // B
                    }
                }
            }
        }
    }

    Some((image_px, pixels))
}

/// Convenience wrapper: generate a Slint-ready RGB pixel buffer for a WIFI: URI.
///
/// Automatically picks `scale=8` and `quiet=4` which produces a crisp image
/// suitable for scanning from a phone at arm's length.
///
/// Returns `None` if QR generation fails (e.g., data too long).
pub fn generate_wifi_qr(
    ssid: &str,
    password: &str,
    auth: &WifiAuth,
) -> Option<(u32, Vec<u8>)> {
    let uri = wifi_uri(ssid, password, auth);
    generate_qr_pixels(uri.as_str(), 8, 4)
}

// ── SVG export ───────────────────────────────────────────────────────────────

/// Encode `data` as a QR code and write it as a crisp SVG file to `path`.
///
/// SVG is chosen over PNG because it requires no compression library,
/// scales to any size without blurring, and is natively understood by
/// every modern browser and image viewer.
pub fn export_qr_svg(data: &str, path: &str) -> Result<(), String> {
    use std::io::Write;

    let code = QrCode::new(data.as_bytes())
        .map_err(|e| format!("QR generation error: {}", e))?;

    let module_count = code.width();
    let quiet = 4usize;     // required quiet zone in modules
    let scale = 10usize;    // 10 px per module → readable on screen
    let total = (module_count + 2 * quiet) * scale;

    let mut svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" \
              viewBox=\"0 0 {s} {s}\" width=\"{s}\" height=\"{s}\" \
              style=\"shape-rendering:crispEdges\">\n\
         <rect width=\"{s}\" height=\"{s}\" fill=\"white\"/>\n",
        s = total
    );

    for row in 0..module_count {
        for col in 0..module_count {
            if code[(col, row)] == QrColor::Dark {
                let x = (quiet + col) * scale;
                let y = (quiet + row) * scale;
                svg.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>\n",
                    x, y, scale, scale
                ));
            }
        }
    }
    svg.push_str("</svg>\n");

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| format!("Cannot write {}: {}", path, e))?;
    f.write_all(svg.as_bytes())
        .map_err(|e| format!("Write error: {}", e))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_wpa() {
        let uri = wifi_uri("My Network", "s3cr3t!", &WifiAuth::Wpa);
        let (ssid, pass, auth) = parse_wifi_uri(&uri).expect("parse failed");
        assert_eq!(ssid, "My Network");
        assert_eq!(pass, "s3cr3t!");
        assert_eq!(auth, WifiAuth::Wpa);
    }

    #[test]
    fn round_trip_special_chars() {
        let ssid = r#"My:Net;work\""#;
        let pass = r#"p@ss;w\ord:"#;
        let uri = wifi_uri(ssid, pass, &WifiAuth::Wpa);
        let (s, p, _) = parse_wifi_uri(&uri).expect("parse failed");
        assert_eq!(s, ssid);
        assert_eq!(p, pass);
    }

    #[test]
    fn round_trip_open() {
        let uri = wifi_uri("FreeWifi", "", &WifiAuth::Open);
        let (ssid, pass, auth) = parse_wifi_uri(&uri).expect("parse failed");
        assert_eq!(ssid, "FreeWifi");
        assert_eq!(pass, "");
        assert_eq!(auth, WifiAuth::Open);
    }

    #[test]
    fn qr_pixels_are_square() {
        let (w, pixels) = generate_qr_pixels("WIFI:T:WPA;S:test;P:pass;;", 4, 4)
            .expect("QR generation failed");
        assert_eq!(pixels.len() as u32, w * w * 3);
    }
}
