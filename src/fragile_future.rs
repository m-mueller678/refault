pub struct FragileFuture<F>(Fragile<F>);

impl<F: Future> Future for FragileFuture<F> {
    type Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        todo!()
    }
}
