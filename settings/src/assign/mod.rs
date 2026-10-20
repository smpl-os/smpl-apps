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
