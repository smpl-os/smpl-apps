//! Source-level contracts for the search boxes' Escape key, the Keybindings
//! tab's filter focus and its column order. The window needs a display to run,
//! so these pin the wiring in `main.slint` and `main.rs`; the launch logic
//! itself is unit-tested in `instance.rs`.

const UI: &str = include_str!("../ui/main.slint");
const MAIN: &str = include_str!("main.rs");
const INSTANCE: &str = include_str!("instance.rs");

/// `src` without comments or whitespace, so neither formatting nor a comment
/// between two statements breaks a contract.
fn squashed(src: &str) -> String {
    let mut squashed = String::with_capacity(src.len());
    for line in src.lines() {
        let mut code = line;
        let (mut in_string, mut escaped, mut previous) = (false, false, ' ');
        for (at, c) in line.char_indices() {
            if in_string {
                match c {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => in_string = false,
                    _ => {}
                }
            } else if c == '"' {
                in_string = true;
            } else if c == '/' && previous == '/' {
                code = &line[..at - 1];
                break;
            }
            previous = c;
        }
        squashed.extend(code.chars().filter(|c| !c.is_whitespace()));
    }
    squashed
}

/// From the first `from` up to, not including, the next `to` after it.
fn between(src: &'static str, from: &str, to: &str) -> &'static str {
    let start = src.find(from).unwrap_or_else(|| panic!("missing {from:?}"));
    let rest = &src[start..];
    let end = rest[from.len()..]
        .find(to)
        .unwrap_or_else(|| panic!("missing {to:?} after {from:?}"));
    &rest[..from.len() + end]
}

/// The element or component that opens with `header`, up to its closing brace.
fn block(src: &'static str, header: &str) -> &'static str {
    let start = src.find(header).unwrap_or_else(|| panic!("missing {header:?}"));
    let open = start + src[start..].find('{').unwrap();
    let mut depth = 0;
    for (offset, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[start..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after {header:?}");
}

fn element(id: &str) -> String {
    squashed(block(UI, &format!(" {id} := ")))
}

fn component(name: &str) -> String {
    squashed(block(UI, &format!("\ncomponent {name} inherits ")))
}

fn sidebar_button(label: &str) -> String {
    let at = UI
        .find(&format!("label: \"{label}\";"))
        .unwrap_or_else(|| panic!("no {label} button"));
    let start = UI[..at].rfind("TabButton {").unwrap();
    squashed(block(&UI[start..], "TabButton {"))
}

fn keybindings_tab() -> &'static str {
    between(UI, "// KEYBINDINGS TAB", "// TASKBAR TAB")
}

const SEARCH_BOXES: [&str; 4] = [
    "global-search-input",
    "kb-filter-input",
    "search-input",
    "dictation-lang-search-input",
];

#[test]
fn every_search_box_is_a_search_input_that_empties_itself_on_escape() {
    let search_input = component("SearchInput");
    assert!(search_input.contains("publicfunctionescape-pressed(key:string)->bool{"));
    assert!(search_input.contains("ifkey!=Key.Escape{returnfalse;}"));
    // The clear runs the host's own `edited` handler, so the results refresh
    // exactly as if the text had been typed away.
    assert!(search_input.contains("self.text=\"\";self.edited();"));
    // Escape with nothing left to clear is the host's to handle.
    assert!(search_input.contains("}else{self.dismissed();}returntrue;"));
    assert!(search_input.contains("key-pressed(e)=>{ifself.escape-pressed(e.text){returnaccept;}reject}"));

    for id in SEARCH_BOXES {
        assert!(UI.contains(&format!(" {id} := SearchInput {{")), "{id} is not a SearchInput");
    }
    assert!(component("KeypadInput").contains("input:=SearchInput{"));

    // A search or filter box added later has to join in.
    for line in UI.lines() {
        if let Some((id, _)) = line.trim().split_once(" := TextInput {") {
            assert!(
                !id.contains("search") && !id.contains("filter"),
                "{id} is a search box that does not clear on Escape"
            );
        }
    }
}

#[test]
fn the_global_and_keybinding_filters_refresh_their_results_when_cleared() {
    let global = element("global-search-input");
    assert!(!global.contains("key-pressed"), "overrides the Escape handling");
    assert!(global.contains("text<=>root.global-search-text;"));
    assert!(global.contains("edited=>{root.search-selected-index=0;root.filter-search(self.text);}"));

    let filter = element("kb-filter-input");
    assert!(!filter.contains("key-pressed"), "overrides the Escape handling");
    assert!(filter.contains("text<=>root.kb-filter-text;"));
    assert!(filter.contains("edited=>{root.kb-filter(self.text,root.kb-selected-section);}"));
}

#[test]
fn the_dropdown_searches_clear_first_and_close_on_the_next_escape() {
    for (id, close) in [
        ("search-input", "root.dropdown-open=false;"),
        ("dictation-lang-search-input", "root.dictation-lang-dropdown-open=false;"),
    ] {
        let search = element(id);
        assert!(search.contains("ifself.escape-pressed(e.text){returnaccept;}"), "{id}");
        assert!(search.contains(&format!("dismissed=>{{{close}focus-scope.focus();}}")), "{id}");
        assert!(!search.contains("Key.Escape"), "{id} handles Escape on its own again");
    }
}

#[test]
fn the_icon_picker_search_is_the_only_keypad_input_that_clears_on_escape() {
    let input = component("KeypadInput");
    assert!(input.contains("inproperty<bool>escape-clears:false;"));
    assert!(input.contains("ifroot.escape-clears&&self.escape-pressed(e.text){returnaccept;}"));
    assert_eq!(UI.matches("escape-clears: true;").count(), 1);
    assert!(block(UI, "\ncomponent KeypadIconPicker inherits ").contains("escape-clears: true;"));
}

#[test]
fn escape_on_an_empty_search_box_steps_back_one_layer_like_it_does_elsewhere() {
    let cascade = squashed(block(UI, "function dismiss-top-layer() {"));
    let mut at = 0;
    for step in [
        "root.dropdown-open=false;",
        "root.dictation-lang-dropdown-open=false;",
        "root.show-help=false;",
        "root.dictation-configuring=false;",
        "root.kb-edit-idx=-1;",
        "root.kb-adding=false;",
        "root.close();",
    ] {
        let found = cascade[at..]
            .find(step)
            .unwrap_or_else(|| panic!("{step} is missing or out of order"));
        at += found + step.len();
    }

    // The window's own Escape and the two page-level boxes share the cascade.
    let ui = squashed(UI);
    assert_eq!(
        ui.matches("ife.text==Key.Escape{root.dismiss-top-layer();returnaccept;}").count(),
        1
    );
    for id in ["global-search-input", "kb-filter-input"] {
        assert!(element(id).contains("dismissed=>{root.dismiss-top-layer();}"), "{id}");
    }
}

#[test]
fn the_keybindings_filter_takes_the_keyboard_whenever_the_tab_is_entered() {
    assert!(squashed(UI).contains("in-outproperty<bool>kb-focus-filter:false;"));

    // Clicking the tab.
    let button = sidebar_button("Keybindings");
    assert!(button.contains("active:root.active-tab==5;"));
    assert!(button.contains("root.active-tab=5;"));
    assert!(button.contains("focus-scope.focus();root.kb-focus-filter=true;"));

    // The tab is created lazily. A request made before that is served by a
    // timer that starts with the tab, one made while it shows by the same timer
    // starting when the flag is raised; it lowers the flag once it has focused.
    let tab = squashed(between(UI, "// KEYBINDINGS TAB", "tab5-flick := Flickable {"));
    assert!(tab.contains("ifroot.active-tab==5:Rectangle{"));
    assert!(tab.contains(concat!(
        "Timer{interval:1ms;running:root.kb-focus-filter;triggered=>{",
        "ifroot.kb-focus-filter{root.kb-focus-filter=false;",
        "kb-filter-input.focus();kb-filter-input.select-all();}}}"
    )));

    // Leaving the tab hands the keyboard back so shortcuts and Escape work.
    let leaving = squashed(between(UI, "changed active-tab => {", "if root.active-tab == 3"));
    assert!(leaving.contains("ifroot.active-tab!=5{root.kb-focus-filter=false;focus-scope.focus();}"));

    // It is the Keybindings tab's filter, not the Keyboard tab's.
    let tab_source = keybindings_tab();
    assert!(tab_source.contains("kb-filter-input := SearchInput {"));
    assert!(tab_source.contains("\"Filter keybindings...\""));
}

#[test]
fn only_the_window_scope_takes_focus_from_an_init_handler() {
    // Slint creates an `if` or `for` subtree while laying its parent out, and
    // focusing from there is rejected as a binding loop that panics the app.
    // Focus moves from event handlers, change handlers and timers instead.
    let mut focusing = Vec::new();
    let mut from = 0;
    while let Some(at) = UI[from..].find("init => {") {
        let start = from + at;
        let handler = squashed(block(&UI[start..], "init => {"));
        if handler.contains(".focus()") {
            focusing.push(handler);
        }
        from = start + "init => {".len();
    }
    assert_eq!(focusing, ["init=>{self.focus();}"]);
    assert!(element("focus-scope").contains("init=>{self.focus();}"));
}

#[test]
fn rebinding_still_reads_its_keys_from_the_window_not_the_filter_box() {
    for touch in ["kb-combo-touch", "kb-rebind-touch"] {
        assert!(
            element(touch).contains("root.kb-edit-combo(entry.idx);focus-scope.focus();"),
            "{touch}"
        );
    }
}

#[test]
fn launches_reach_the_keybindings_filter_through_one_shared_request() {
    assert_eq!(crate::instance::KEYBINDINGS_TAB, 5);
    assert!(MAIN.contains(r#""keybindings" => 5,"#));

    let main = squashed(MAIN);
    // `--tab` is remembered only when given, so a bare `settings` just raises.
    assert!(main.contains("requested_tab=Some(initial_tab);"));
    // The first launch applies its own command line; later ones hand it over.
    assert!(main.contains(
        "letlaunch=instance::Request::from_launch(requested_tab,&initial_highlight);instance::acquire_or_hand_off(&launch);"
    ));
    assert!(main.contains("instance::apply(&ui,&launch);"));
    assert!(main.contains("instance::watch(&ui);"));
    assert!(!MAIN.contains("settings-deeplink"), "main.rs reads the request file again");

    let instance = squashed(INSTANCE);
    assert!(instance.contains("fnfocus_keybindings_filter(&self){self.set_kb_focus_filter(true);}"));
    assert!(instance.contains("ifrequest.focuses_keybindings_filter(){window.focus_keybindings_filter();}"));
    assert!(instance.contains("if!request.highlight.is_empty(){window.blink_setting(&request.highlight);}"));
}

#[test]
fn keybinding_rows_list_the_function_before_the_key_combination() {
    let tab = keybindings_tab();

    // Normal row: description, then the combination, then the edit pencil.
    let normal = between(tab, "if root.kb-edit-idx != entry.idx: HorizontalLayout {", "// Edit view");
    let description = normal.find("text: entry.description;").expect("description");
    let combo = normal.find("entry.combo").expect("combination");
    let pencil = normal.find("kb-edit-touch := TouchArea").expect("edit button");
    assert!(description < combo, "the combination comes before the function");
    assert!(combo < pencil);
    assert_eq!(normal.matches("text: entry.description;").count(), 1);

    // The function still takes the spare width and elides; the combination keeps
    // its fixed width, so the key column lines up between rows.
    let text = squashed(block(normal, "Text {"));
    assert!(text.contains("horizontal-stretch:1;"));
    assert!(text.contains("text:entry.description;"));
    assert!(text.contains("overflow:elide;"));
    let badge = squashed(between(normal, "// Key combo badge", "kb-combo-touch :="));
    assert!(badge.contains("width:180px;"));
    assert!(badge.contains("overflow:elide;"));

    // Expanded row: the function first, then the rebind button.
    let edit = between(tab, "// Edit view", "kb-args-input := TextInput");
    assert!(edit.find("text: entry.description;").unwrap() < edit.find("// Rebind button").unwrap());
    assert_eq!(edit.matches("text: entry.description;").count(), 1);
}
