use std::{cell::Cell, net::SocketAddr, pin::Pin};

use bytes::Bytes;

use crate::{
    check_send::{CheckSend, NodeBound},
    ip_addr::IpAddrSimulator,
    packet_network::{ConNetSocket, Packet, SocketReceiveFuture},
    runtime::Id,
    simulator::SimulatorHandle,
};

struct TcpDatagram {
    src: SocketAddr,
    dst: SocketAddr,
    msg: Msg,
}

impl Packet for TcpDatagram {}

enum Msg {
    Connect(Id),
    Accept,
    Data(Bytes),
}

pub struct TcpSocketInner {
    socket: ConNetSocket<TcpDatagram>,
    local_addr: SocketAddr,
    ip: SimulatorHandle<IpAddrSimulator>,
    peer_addr: SocketAddr,
    receive_state: Cell<ReceiveState>,
}

enum ReceiveState {
    None,
    Taken,
    Future(Pin<Box<SocketReceiveFuture<TcpDatagram>>>),
    Peeked(TcpDatagram),
}
pub struct TcpSocket(CheckSend<TcpSocketInner, NodeBound>);
