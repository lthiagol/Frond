//! Test-only helpers for the unit tests in this crate. They live in the
//! `testkit` crate, the one home for the sample vault's path and copy; this
//! module re-exports them so the tests keep one import path.

pub use testkit::{copy_testvault, testvault_root, TempDir};
