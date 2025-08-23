use std::pin::Pin;

use fragile::Fragile;

pin_project_lite::pin_project! {
    pub struct FragileFuture<F> {
        #[pin]
        inner: Fragile<F>,
    }
}

impl<F: Future> Future for FragileFuture<F> {
    type Output = F::Output;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        fragile_pinned(self.project().inner).poll(cx)
    }
}

impl<F> FragileFuture<F> {
    pub fn new(inner: F) -> Self {
        FragileFuture {
            inner: Fragile::new(inner),
        }
    }
}

fn fragile_pinned<T>(x: Pin<&mut Fragile<T>>) -> Pin<&mut T> {
    unsafe { Pin::new_unchecked(Pin::get_unchecked_mut(x).get_mut()) }
}
