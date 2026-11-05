//! Where the replay library's progress and warning lines go: standard error, unless the
//! caller sets a hook, for example to send them through a writer that never blocks.
use std::sync::OnceLock;

type Hook = Box<dyn Fn(String) + Send + Sync>;
static HOOK: OnceLock<Hook> = OnceLock::new();

/// Sends every later progress and warning line to `hook` instead of standard error. Only the
/// first call takes effect; it returns whether this one did. The hook must not block.
pub fn set_log_hook(hook: impl Fn(String) + Send + Sync + 'static) -> bool {
    HOOK.set(Box::new(hook)).is_ok()
}

pub(crate) fn line(text: String) {
    match HOOK.get() {
        Some(hook) => hook(text),
        None => eprintln!("{text}"),
    }
}

/// Writes one line through the hook. Callers must not hold a lock while using it.
macro_rules! note {
    ($($arg:tt)*) => { $crate::log::line(format!($($arg)*)) };
}
