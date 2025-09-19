//! A [tower] RPC implementation based on [Net].
use std::{
    cell::RefCell, collections::HashMap, future::poll_fn, io::ErrorKind, marker::PhantomData,
    rc::Rc, sync::Arc,
};

use either::Either::{self, Left, Right};
use futures::{
    SinkExt, StreamExt,
    channel::{mpsc, oneshot},
    never::Never,
};
use tower::Service;

use crate::{
    executor::{AbortGuard, spawn},
    id::Id,
    net::{Addr, Net, Packet, Socket},
    simulator::SimulatorHandle,
};

impl<R: 'static> Packet for TowerRequest<R> {}
enum TowerRequest<R> {
    Start { req: R, id: crate::id::Id },
}

impl<R: 'static> Packet for TowerResponse<R> {}
enum TowerResponse<R> {
    Complete { result: R, id: crate::id::Id },
}

type TowerResult<B, E> = Result<B, Either<E, std::io::Error>>;
type ReqWithSender<A, B, E> = (A, oneshot::Sender<TowerResult<B, E>>);

/// A [Service] that forwards calls to a server running [run_server].
pub struct Client<A, B, E> {
    request_sender: mpsc::Sender<ReqWithSender<A, B, E>>,
    _send_task: AbortGuard,
    _recv_task: AbortGuard,
    _p: PhantomData<*const u8>,
}

impl<A, B, E: std::fmt::Debug> Service<A> for Client<A, B, E> {
    type Response = B;

    type Error = Either<E, std::io::Error>;

    type Future = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.request_sender.poll_ready(cx).map(|x| {
            x.unwrap();
            Ok(())
        })
    }

    fn call(&mut self, req: A) -> Self::Future {
        let mut sender = self.request_sender.clone();
        async move {
            let (s, r) = oneshot::channel();
            sender
                .send((req, s))
                .await
                .map_err(|_| Right(ErrorKind::ConnectionReset.into()))?;
            dbg!();
            r.await
                .unwrap_or_else(|_| Err(Right(ErrorKind::ConnectionReset.into())))
        }
    }
}

impl<A: 'static, B: 'static, E: 'static> Client<A, B, E> {
    pub fn new(remote: Addr) -> Self {
        let (s, mut r) = mpsc::channel(0);
        let socket = Socket::<TowerResponse<Result<B, E>>>::open(Id::new()).unwrap();
        let responders = Rc::new(RefCell::new(
            HashMap::<crate::id::Id, oneshot::Sender<_>>::new(),
        ));
        let responders_2 = responders.clone();
        let local_port = socket.local_port();
        let send_task = spawn(async move {
            loop {
                // if sender is dropped, this task is aborted
                let (req, responder) = r.next().await.unwrap();
                let id = Id::new();
                match SimulatorHandle::<Net>::get()
                    .with(|net| net.send(local_port, remote, TowerRequest::Start { req, id }))
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
                    Ok((response, addr)) => {
                        debug_assert!(addr == remote);
                        match response {
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
        Client {
            request_sender: s,
            _send_task: send_task.abort_handle().abort_on_drop(),
            _recv_task: recv_task.abort_handle().abort_on_drop(),
            _p: PhantomData,
        }
    }
}

/// Run a server that handles requests from [Clients](Client).
///
/// Each Service call is run on a new task.
pub async fn run_server<R, S: Service<R>>(
    port: crate::id::Id,
    mut service: S,
) -> Result<Never, Either<S::Error, std::io::Error>>
where
    R: 'static,
    S::Response: 'static,
    S::Error: 'static,
    S::Future: 'static,
{
    let socket = Socket::<TowerRequest<R>>::open(port).map_err(|e| Right(e.into()))?;
    loop {
        let (request, address) = socket.receive().await.map_err(Right)?;
        poll_fn(|cx| service.poll_ready(cx)).await.map_err(Left)?;
        match request {
            TowerRequest::Start { req, id } => {
                #[cfg(feature = "emit-tracing")]
                tracing::info!(id = id.tv(), client = ?address, "start");
                let fut = service.call(req);
                let local_port = socket.local_port();
                let future = async move {
                    let result = fut.await;
                    SimulatorHandle::<Net>::get()
                        .with(|net| {
                            net.send(local_port, address, TowerResponse::Complete { result, id })
                        })
                        .await
                        .ok();
                };
                #[cfg(feature = "emit-tracing")]
                let future = tracing::Instrument::instrument(
                    future,
                    tracing::error_span!("rpc", id = id.tv()),
                );
                spawn(future).detach()
            }
        }
    }
}
