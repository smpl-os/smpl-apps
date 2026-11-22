use crate::config::WorkspaceStyle;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompositorKind {
    Hyprland,
    Niri,
    X11,
    Unavailable,
}

impl CompositorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hyprland => "hyprland",
            Self::Niri => "niri",
            Self::X11 => "x11",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Monitor {
    pub id: String,
    pub connector: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub focused: bool,
    pub active_workspace: i32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Workspace {
    pub id: i32,
    pub monitor: String,
    pub occupied: bool,
    pub visible: bool,
    pub focused: bool,
    pub temporary: bool,
    pub windows: u32,
}

impl Workspace {
    pub fn label(&self, style: WorkspaceStyle) -> String {
        match style {
            WorkspaceStyle::Numbers if self.focused => "●".to_owned(),
            WorkspaceStyle::Numbers => self.id.to_string(),
            WorkspaceStyle::Squares => String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct KeyboardState {
    pub visible: bool,
    pub code: String,
    pub layout: String,
    pub xkb_layout: String,
    pub xkb_variant: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompositorState {
    pub kind: CompositorKind,
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub policy_primary: String,
    pub focused_workspace: i32,
    pub focused_monitor: String,
    pub active_window_title: String,
    pub keyboard: KeyboardState,
    pub error: String,
}

impl Default for CompositorState {
    fn default() -> Self {
        Self {
            kind: CompositorKind::Unavailable,
            monitors: Vec::new(),
            workspaces: Vec::new(),
            policy_primary: String::new(),
            focused_workspace: 1,
            focused_monitor: String::new(),
            active_window_title: String::new(),
            keyboard: KeyboardState::default(),
            error: String::new(),
        }
    }
}

impl CompositorState {
    pub fn primary_monitor(&self) -> Option<&Monitor> {
        self.monitors
            .iter()
            .find(|monitor| {
                !self.policy_primary.is_empty() && monitor.connector == self.policy_primary
            })
            .or_else(|| {
                self.monitors
                    .iter()
                    .find(|monitor| monitor.active_workspace == 1)
            })
            .or_else(|| {
                self.workspaces
                    .iter()
                    .find(|workspace| workspace.id == 1)
                    .and_then(|workspace| {
                        self.monitors
                            .iter()
                            .find(|monitor| monitor.connector == workspace.monitor)
                    })
            })
            .or_else(|| {
                self.monitors
                    .iter()
                    .find(|monitor| monitor.connector == self.focused_monitor)
            })
            .or_else(|| {
                self.monitors
                    .iter()
                    .min_by_key(|monitor| (monitor.x, monitor.y))
            })
    }

    pub fn workspace_for_bar(&self, maximum: usize) -> Vec<Workspace> {
        if maximum == 0 {
            return Vec::new();
        }
        let mut result: Vec<_> = (1..=maximum)
            .map(|id| Workspace {
                id: id as i32,
                focused: id as i32 == normalize_group(self.focused_workspace, maximum),
                monitor: self.focused_monitor.clone(),
                ..Workspace::default()
            })
            .collect();
        for workspace in &self.workspaces {
            if workspace.id <= 0 {
                continue;
            }
            let index = normalize_group(workspace.id, maximum).saturating_sub(1) as usize;
            if let Some(slot) = result.get_mut(index) {
                slot.occupied |= workspace.occupied;
                slot.visible |= workspace.visible;
                slot.windows += workspace.windows;
            }
        }
        result
    }
}

pub fn normalize_group(workspace: i32, maximum: usize) -> i32 {
    if workspace <= 0 || maximum == 0 {
        return 1;
    }
    ((workspace - 1) % maximum as i32) + 1
}

pub fn layout_code(layout: &str) -> String {
    let lower = layout.to_ascii_lowercase();
    let code = match lower.as_str() {
        "english (us)" | "us" => "EN",
        "english (uk)" | "uk" => "EN",
        "french" | "fr" => "FR",
        "german" | "de" => "DE",
        "spanish" | "es" => "ES",
        "italian" | "it" => "IT",
        "portuguese" | "pt" => "PT",
        "russian" | "ru" => "RU",
        "ukrainian" | "ua" => "UA",
        "polish" | "pl" => "PL",
        "czech" | "cz" => "CZ",
        "japanese" | "jp" => "JP",
        "korean" | "kr" => "KR",
        "chinese" | "cn" => "ZH",
        _ => {
            let mut chars = layout
                .chars()
                .filter(|character| character.is_alphanumeric());
            let first = chars.next().unwrap_or('?');
            let second = chars.next().unwrap_or(first);
            return format!("{first}{second}").to_ascii_uppercase();
        }
    };
    code.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_groups_are_bounded_and_merge_occupancy() {
        let state = CompositorState {
            focused_workspace: 12,
            focused_monitor: "DP-1".to_owned(),
            workspaces: vec![
                Workspace {
                    id: 2,
                    occupied: true,
                    windows: 1,
                    ..Workspace::default()
                },
                Workspace {
                    id: 12,
                    visible: true,
                    windows: 2,
                    ..Workspace::default()
                },
            ],
            ..CompositorState::default()
        };
        let bar = state.workspace_for_bar(10);
        assert_eq!(bar.len(), 10);
        assert!(bar[1].occupied);
        assert!(bar[1].visible);
        assert!(bar[1].focused);
        assert_eq!(bar[1].windows, 3);
    }

    #[test]
    fn keyboard_code_has_a_localized_short_fallback() {
        assert_eq!(layout_code("English (US)"), "EN");
        assert_eq!(layout_code("Russian"), "RU");
        assert_eq!(layout_code("Norwegian"), "NO");
    }

    #[test]
    fn panel_monitor_follows_workspace_one_instead_of_keyboard_focus() {
        let mut state = CompositorState {
            focused_monitor: "DP-2".to_owned(),
            monitors: vec![
                Monitor {
                    connector: "DP-1".to_owned(),
                    active_workspace: 1,
                    ..Monitor::default()
                },
                Monitor {
                    connector: "DP-2".to_owned(),
                    focused: true,
                    active_workspace: 4,
                    ..Monitor::default()
                },
            ],
            policy_primary: "DP-2".to_owned(),
            ..CompositorState::default()
        };
        assert_eq!(state.primary_monitor().unwrap().connector, "DP-2");
        state.policy_primary.clear();
        assert_eq!(state.primary_monitor().unwrap().connector, "DP-1");
    }
}
