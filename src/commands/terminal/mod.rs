//! Terminal memory write surface: hooks, flush, session clear, and one-shot ingest.
//!
//! Read path is not here — use `workset` / `sivtr-core::query` (`terminal` source).

pub mod clear;
pub mod flush;
pub mod init;
pub mod pipe;
pub mod run;
