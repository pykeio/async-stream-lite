use alloc::sync::Arc;
use core::{
	future::Future,
	marker::PhantomData,
	pin::Pin,
	task::{Context, Poll}
};

use futures_core::stream::{FusedStream, Stream};

use crate::{SharedStore, Yielder, enter};

pin_project_lite::pin_project! {
	/// A [`Stream`] created from a fallible, asynchronous generator-like function.
	///
	/// To create a [`TryAsyncStream`], use the [`try_async_stream`] function. See also [`crate::AsyncStream`].
	pub struct TryAsyncStream<T, E, U> {
		store: Arc<SharedStore<T>>,
		done: bool,
		#[pin]
		generator: U,
		_p: PhantomData<E>
	}
}

impl<T, E, U> FusedStream for TryAsyncStream<T, E, U>
where
	U: Future<Output = Result<(), E>>
{
	fn is_terminated(&self) -> bool {
		self.done
	}
}

impl<T, E, U> Stream for TryAsyncStream<T, E, U>
where
	U: Future<Output = Result<(), E>>
{
	type Item = Result<T, E>;

	fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		let me = self.project();
		if *me.done {
			return Poll::Ready(None);
		}

		let res = {
			let _enter = enter(&me.store);
			me.generator.poll(cx)
		};

		*me.done = res.is_ready();

		if let Poll::Ready(Err(e)) = res {
			return Poll::Ready(Some(Err(e)));
		} else if me.store.has_value() {
			return Poll::Ready(me.store.cell.take().map(Ok));
		}

		if *me.done { Poll::Ready(None) } else { Poll::Pending }
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		if self.done { (0, Some(0)) } else { (0, None) }
	}
}

/// Create an asynchronous [`Stream`] from a fallible asynchronous generator function.
///
/// Usage is similar to [`crate::async_stream`], however the future returned by `generator` is assumed to return
/// `Result<(), E>` instead of `()`.
///
/// ```
/// use std::{io, net::SocketAddr};
///
/// use async_stream_lite::try_async_stream;
/// use futures::stream::Stream;
/// use tokio::net::{TcpListener, TcpStream};
///
/// fn bind_and_accept(addr: SocketAddr) -> impl Stream<Item = io::Result<TcpStream>> {
/// 	try_async_stream(|yielder| async move {
/// 		let mut listener = TcpListener::bind(addr).await?;
/// 		loop {
/// 			let (stream, addr) = listener.accept().await?;
/// 			println!("received on {addr:?}");
/// 			yielder.r#yield(stream).await;
/// 		}
/// 	})
/// }
/// ```
///
/// The resulting stream yields `Result<T, E>`. The yielder function will cause the stream to yield `Ok(T)`. When an
/// error is encountered, the stream yields `Err(E)` and is subsequently terminated.
pub fn try_async_stream<T, E, F, U>(generator: F) -> TryAsyncStream<T, E, U>
where
	F: FnOnce(Yielder<T>) -> U,
	U: Future<Output = Result<(), E>>
{
	let store = Arc::new(SharedStore::default());
	let generator = generator(Yielder { store: Arc::downgrade(&store) });
	TryAsyncStream {
		store,
		done: false,
		generator,
		_p: PhantomData
	}
}

#[cfg(test)]
mod tests {
	use futures::{Stream, StreamExt};

	use super::try_async_stream;

	#[tokio::test]
	async fn single_err() {
		let s = try_async_stream(|yielder| async move {
			if true {
				Err("hello")?;
			} else {
				yielder.r#yield("world").await;
			}
			Ok(())
		});

		let values: Vec<_> = s.collect().await;
		assert_eq!(1, values.len());
		assert_eq!(Err("hello"), values[0]);
	}

	#[tokio::test]
	async fn yield_then_err() {
		let s = try_async_stream(|yielder| async move {
			yielder.r#yield("hello").await;
			Err("world")?;
			unreachable!();
		});

		let values: Vec<_> = s.collect().await;
		assert_eq!(2, values.len());
		assert_eq!(Ok("hello"), values[0]);
		assert_eq!(Err("world"), values[1]);
	}

	#[tokio::test]
	async fn convert_err() {
		struct ErrorA(u8);
		#[derive(PartialEq, Debug)]
		struct ErrorB(u8);
		impl From<ErrorA> for ErrorB {
			fn from(a: ErrorA) -> Self {
				ErrorB(a.0)
			}
		}

		fn test() -> impl Stream<Item = Result<&'static str, ErrorB>> {
			try_async_stream(|yielder| async move {
				if true {
					Err(ErrorA(1))?;
				} else {
					Err(ErrorB(2))?;
				}
				yielder.r#yield("unreachable").await;
				Ok(())
			})
		}

		let values: Vec<_> = test().collect().await;
		assert_eq!(1, values.len());
		assert_eq!(Err(ErrorB(1)), values[0]);
	}
}
