#![feature(abort_unwind)]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]
//! Many functions within this crate should only be called from within a simulation and will panic otherwise.
//! [NodeId::try_current](runtime::NodeId::try_current) may be used to check if inside the simulation.

pub mod agnostic_lite_runtime;
mod context;
mod event;
mod interception;
pub mod packet_network;
pub mod runtime;
pub mod simulator;
pub use context::time;
pub mod send_bind;
mod send_bind_util;
pub mod tower;
