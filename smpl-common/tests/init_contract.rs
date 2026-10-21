// Every app starts through `smpl_common::init`, so these two calls are what protect all of them.
// Dropping either brings back a silent abort: Slint reads the desktop colour scheme through zbus
// on the UI thread, which needs a Tokio runtime context, and the apps run with stderr discarded.
const LIB: &str = include_str!("../src/lib.rs");

fn init_body() -> &'static str {
    let start = LIB.find("pub fn init(").expect("smpl_common::init exists");
    let end = LIB[start..].find("\n}\n").expect("init ends at a closing brace in column 0");
    &LIB[start..start + end]
}

#[test]
fn init_installs_the_crash_log_then_the_ui_runtime_before_building_the_backend() {
    let body = init_body();
    let crash_log = body.find("crash_log::install(app_id);").expect("init installs the crash log");
    let runtime = body.find("runtime::enter_ui_thread()?;").expect("init enters the UI runtime");
    let backend = body.find("Backend::builder()").expect("init builds the backend");
    assert!(crash_log < runtime, "the crash log must be able to record a failure to start the runtime");
    assert!(runtime < backend, "the runtime context must exist before the backend can spawn the watcher");
}
