/// Platform-agnostic time types.
/// On native, delegates to std::time. On WASM, uses js_sys::Date.

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Duration, Instant};

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::*;

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    #[derive(Clone, Copy)]
    pub struct Instant(f64);
    #[derive(Clone, Copy)]
    pub struct Duration(f64);

    impl Instant {
        pub fn now() -> Self {
            Instant(js_sys::Date::now())
        }
        pub fn elapsed(&self) -> Duration {
            Duration(js_sys::Date::now() - self.0)
        }
    }

    impl Duration {
        pub fn from_millis(ms: u64) -> Self {
            Duration(ms as f64)
        }
        pub fn from_secs(secs: u64) -> Self {
            Duration(secs as f64 * 1000.0)
        }
    }

    impl PartialOrd for Duration {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            self.0.partial_cmp(&other.0)
        }
    }
    impl PartialEq for Duration {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }
}
