//! `tokenloom-engines` — engine trait, declarative interpreters, family
//! implementations, the 248-engine registry and RRF federation
//! (PLAN.md §5).

pub mod builtin_specs;
pub mod factory;
pub mod families;
pub mod federation;
pub mod generic;
pub mod html_util;
pub mod http_util;
pub mod json_path;
pub mod registry;
pub mod spec;
pub mod specialists;
pub mod trait_def;

pub use factory::build;
pub use federation::{Federator, MAX_ENGINES_PER_QUERY};
pub use registry::Registry;
pub use spec::{EngineSpec, RequestSpec, ResponseSpec, SpecFragment};
pub use trait_def::{Engine, EngineCapabilities, EngineError};
