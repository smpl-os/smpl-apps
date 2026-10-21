//! Action assignment, shared by every tab that maps controls to actions
//! (Keypad, Mouse): the binding grammar and its validation, profiles with
//! global and per-app scopes, the action picker's view-model, and the
//! profile edit flows. Nothing here knows about a particular device; a tab
//! brings its own controls, its own wording and its own storage.

pub mod action;
pub mod flow;
pub mod json;
pub mod picker;
pub mod profile;

/// How a tab keeps its edits. The shared editing code says so in its
/// status messages, so it can serve a tab with a Save button and one that
/// writes every edit at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saving {
    /// Edits wait for the tab's Save button (Keypad).
    Manual,
    /// Every edit is written as soon as it is made (Mouse).
    Immediate,
}

impl Saving {
    /// What a status message ends with after an edit.
    pub fn tail(self) -> &'static str {
        match self {
            Self::Manual => "Not saved yet.",
            Self::Immediate => "Saved.",
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::json::{self, Json};
    use super::profile::Profiles;

    /// The smallest document that carries profiles: what the shared code is
    /// tested against, with no tab's own settings around it.
    pub struct Doc(pub Json);

    impl Profiles for Doc {
        fn doc(&self) -> &Json {
            &self.0
        }

        fn doc_mut(&mut self) -> &mut Json {
            &mut self.0
        }
    }

    pub fn doc_from(text: &str) -> Doc {
        Doc(json::parse(text).expect("test document"))
    }

    /// One global profile and nothing else.
    pub fn global_only() -> Doc {
        doc_from(r#"{"profiles":[{"name":"global","bindings":{}}]}"#)
    }
}
