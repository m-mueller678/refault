use crate::{packet_network::receive, simulator::with_node_simulator};

pub struct Transport<Item, SinkItem> {
    dst: NodeId,
    incoming: mpsc::Receiver<Item>,
}

enum Msg<Req, Resp> {
    Connect(u64),
    Accept(u64),
    Request(u64, Req),
    Response(u64, Resp),
}

struct Sim {
    next_id: u64,
}

struct NodeSim<A, B> {
    incoming: mpsc::Sender<u64>,
    request_handlers: HashMap<u64, onsehot::Sender<A>>,
    open_requests: HashMap<u64, oneshot::Sender<B>>,
}
async fn run_dispatcher<A, B>() {
    loop {
        let msg = receive::<Msg<A, B>>().await;
        with_node_simulator::<NodeSim<A, B>>(|s| match msg {
            Ok(Msg::Connect(id)) => s.incoming.send(id),
        })
    }
}
