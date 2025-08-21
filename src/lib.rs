//! Many functions within this crate should only be called from within a simulation and will panic otherwise.
//! [NodeId::try_current](runtime::NodeId::try_current) may be used to check if inside the simulation.

mod agnostic_lite_runtime;
mod context;
mod event;
mod interception;
pub mod ip_addr;
pub mod packet_network;
pub mod runtime;
pub mod simulator;
pub mod udp;
pub use context::time;
mod fragile_future;

pub use smallvec;
