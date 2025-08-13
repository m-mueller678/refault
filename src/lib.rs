//! Many functions within this crate should only be called from within a simulation and will panic otherwise.
//! [NodeId::try_current](runtime::NodeId::try_current) may be used to check if inside the simulation.

pub mod connectivity_fn_network;
mod context;
mod event;
mod interception;
pub mod packet_network;
pub mod runtime;
pub mod simulator;
pub use context::time;
pub mod tarpc;
