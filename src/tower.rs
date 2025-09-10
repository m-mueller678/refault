use std::{cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc, sync::Arc};

use either::Either::{self, Left, Right};
use futures::{
    StreamExt,
    channel::{mpsc, oneshot},
};
use tower::Service;

use crate::{
    check_send::{CheckSend, Constraint, NodeBound},
    context::executor::AbortGuard,
    packet_network::{Addr, ConNet, ConNetSocket, Packet},
    runtime::{Id, spawn},
    simulator::simulator,
};

impl<R: 'static> Packet for TowerRequest<R> {}
enum TowerRequest<R> {
    Start { req: R, id: Id },
}

impl<R: 'static> Packet for TowerResponse<R> {}
enum TowerResponse<R> {
    Complete { result: R, id: Id },
}

type TowerResult<R, S: Service<R>> = Result<S::Response, Either<S::Error, std::io::Error>>;
type ReqWithSender<R, S: Service<R>> = (R, oneshot::Sender<TowerResult<R, S>>);

pub struct ClientUnSend<R, S: Service<R>> {
    request_sender: mpsc::Sender<ReqWithSender<R, S>>,
    _send_task: AbortGuard,
    _recv_task: AbortGuard,
    _p: PhantomData<*const u8>,
}

pub struct Client<R, S: Service<R>>(pub CheckSend<ClientUnSend<R, S>, NodeBound>);

impl<R, S: Service<R>> Service<R> for Client<R, S> {
    type Response = S::Response;

    type Error = Either<S::Error, std::io::Error>;

    type Future = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.0.request_sender.poll_ready(cx).map(|x| Ok(x.unwrap()))
    }

    fn call(&mut self, req: R) -> Self::Future {
        let (s, r) = oneshot::channel();
        self.0.request_sender.try_send((req, s)).unwrap();
        async move { r.await.unwrap() }
    }
}

impl<R, S: Service<R>> Client<R, S>
where
    S::Response: 'static,
    S::Error: 'static,
    R: 'static,
{
    pub fn new(remote: Addr) -> Self {
        let (s, mut r) = mpsc::channel(0);
        let socket =
            ConNetSocket::<TowerResponse<Result<S::Response, S::Error>>>::open(Id::new()).unwrap();
        let responders = Rc::new(RefCell::new(HashMap::<Id, oneshot::Sender<_>>::new()));
        let responders_2 = responders.clone();
        let local_addr = socket.local_addr();
        let send_task = spawn(async move {
            let net = simulator::<ConNet>();
            loop {
                // if sender is dropped, this task is aborted
                let (req, responder) = r.next().await.unwrap();
                let id = Id::new();
                match net
                    .with(|net| net.send(local_addr, remote, TowerRequest::Start { req: req, id }))
                    .await
                {
                    Ok(()) => {
                        responders.borrow_mut().insert(id, responder);
                    }
                    Err(e) => {
                        responder.send(Err(Right(e))).ok();
                    }
                };
            }
        });
        let recv_task = spawn(async move {
            loop {
                match socket.receive().await {
                    Ok(x) => {
                        debug_assert!(x.addr == remote);
                        match x.content {
                            TowerResponse::Complete { result, id } => {
                                responders_2
                                    .borrow_mut()
                                    .remove(&id)
                                    .unwrap()
                                    .send(result.map_err(Left))
                                    .ok();
                            }
                        }
                    }
                    Err(e) => {
                        let error = Arc::new(e);
                        for r in responders_2.take().into_values() {
                            r.send(Err(Right(std::io::Error::new(error.kind(), error.clone()))))
                                .ok();
                        }
                    }
                }
            }
        });
        Client(NodeBound::wrap(ClientUnSend {
            request_sender: s,
            _send_task: send_task.abort_handle().abort_on_drop(),
            _recv_task: recv_task.abort_handle().abort_on_drop(),
            _p: PhantomData,
        }))
    }
}
