//! MEV detector: find sandwich bundles in decoded swap events.

pub mod sandwich;
pub(crate) mod building;
pub(crate) mod discovery;
pub(crate) mod postprocess;
