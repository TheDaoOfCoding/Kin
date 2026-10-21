//! Dev-only (feature `mock`): a pipe SERVER, and a stand-in for Kin's side of docs/extension.md. Used by the tests and by
//! smoke/run-real-host.ts. `server::create_instance` is also the reference for how Kin must create its pipe (see docs/extension.md).

pub mod kin;
pub mod server;
