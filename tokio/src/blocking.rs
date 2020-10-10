cfg_rt_core! {
    pub(crate) use crate::runtime::spawn_blocking;
    pub(crate) use crate::task::JoinHandle;
}

cfg_not_rt_core! {
    use std::fmt;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    pub(crate) fn spawn_blocking<F, R>(_f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        panic!("requires the `rt-core` Tokio feature flag")

    }

    pub(crate) struct JoinHandle<R> {
        _p: std::marker::PhantomData<R>,
    }

    impl<R> Future for JoinHandle<R> {
        type Output = Result<R, std::io::Error>;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            unreachable!()
        }
    }

    impl<T> fmt::Debug for JoinHandle<T>
    where
        T: fmt::Debug,
    {
        fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt.debug_struct("JoinHandle").finish()
        }
    }
}