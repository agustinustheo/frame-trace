//! Route the process hook by thread; only panics inside our execution catch are quiet.
use std::{cell::Cell, panic::AssertUnwindSafe, sync::Once};
thread_local! {
    static CAUGHT: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn catch<T>(run: impl FnOnce() -> T) -> std::thread::Result<T> {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !CAUGHT.with(Cell::get) {
                previous(info);
            }
        }));
    });
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CAUGHT.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(CAUGHT.with(|flag| flag.replace(true)));
    std::panic::catch_unwind(AssertUnwindSafe(run))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #[test]
    fn hook_only_suppresses_the_execution_thread_inside_the_catch_scope() {
        const CHILD: &str = "FRAME_TRACE_HOOK_TEST";
        if std::env::var_os(CHILD).is_some() {
            std::panic::set_hook(Box::new(|_| eprintln!("outside replay hook")));
            assert!(super::catch(|| panic!("quiet replay")).is_err());
            assert!(std::panic::catch_unwind(|| panic!("genuine panic")).is_err());
            super::catch(|| {
                assert!(
                    std::thread::spawn(|| panic!("other thread"))
                        .join()
                        .is_err()
                );
                assert!(super::catch(|| panic!("nested replay")).is_err());
            })
            .unwrap();
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panic_scope::tests::hook_only_suppresses_the_execution_thread_inside_the_catch_scope", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr)
                .matches("outside replay hook")
                .count(),
            2
        );
    }
}
