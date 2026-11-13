//! Notetaker logic that does not need a browser: opt-out detection and speaker matching.
pub mod browser;
pub mod capture;
pub mod driver;
pub mod links;
pub mod meet;
pub mod optout;
pub mod profile;
pub mod session;
pub use recapz_core::speakers;
#[cfg(all(test, feature = "browser-tests"))]
mod test_support;
pub mod wav;
