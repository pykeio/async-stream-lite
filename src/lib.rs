#![allow(clippy::tabs_in_doc_comments)]
#![cfg_attr(not(test), no_std)]

extern crate alloc;
extern crate core;

use alloc::sync::{Arc, Weak};
use core::{
	cell::Cell,
	future::Future,
	pin::Pin,
	sync::atomic::{AtomicBool, Ordering},
	task::{Context, Poll}
};

use futures_core::stream::{FusedStream, Stream};

#[cfg(test)]
mod tests;
mod r#try;

pub(crate) struct SharedStore<T> {
	entered: AtomicBool,
	cell: Cell<Option<T>>
}

impl<T> Default for SharedStore<T> {
	fn default() -> Self {
		Self {
			entered: AtomicBool::new(false),
			cell: Cell::new(None)
		}
	}
}

impl<T> SharedStore<T> {
	pub fn has_value(&self) -> bool {
		unsafe { &*self.cell.as_ptr() }.is_some()
	}
}

unsafe impl<T> Sync for SharedStore<T> {}

pub struct Yielder<T> {
	pub(crate) store: Weak<SharedStore<T>>
}

impl<T> Yielder<T> {
	pub fn r#yield(&self, value: T) -> YieldFut<T> {
		#[cold]
		fn invalid_usage() -> ! {
			panic!("attempted to use async_stream_lite yielder outside of stream context or across threads")
		}

		let Some(store) = self.store.upgrade() else {
			invalid_usage();
		};
		if !store.entered.load(Ordering::Relaxed) {
			invalid_usage();
		}

		store.cell.replace(Some(value));

		YieldFut { store }
	}
}

/// Future returned by an [`AsyncStream`]'s yield function.
///
/// This future must be `.await`ed inside the generator in order for the item to be yielded by the stream.
#[must_use = "stream will not yield this item unless the future returned by yield is awaited"]
pub struct YieldFut<T> {
	store: Arc<SharedStore<T>>,
}

impl<T> Unpin for YieldFut<T> {}

impl<T> Future for YieldFut<T> {
	type Output = ();

	fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
		if !self.store.has_value() {
			return Poll::Ready(());
		}

		Poll::Pending
	}
}

struct Enter<'s, T> {
	store: &'s SharedStore<T>
}

fn enter<T>(store: &SharedStore<T>) -> Enter<'_, T> {
	store.entered.store(true, Ordering::Relaxed);
	Enter { store }
}

impl<T> Drop for Enter<'_, T> {
	fn drop(&mut self) {
		self.store.entered.store(false, Ordering::Relaxed);
	}
}

pin_project_lite::pin_project! {
	/// A [`Stream`] created from an asynchronous generator-like function.
	///
	/// To create an [`AsyncStream`], use the [`async_stream`] function.
	pub struct AsyncStream<T, U> {
		store: Arc<SharedStore<T>>,
		done: bool,
		#[pin]
		generator: U
	}
}

impl<T, U> FusedStream for AsyncStream<T, U>
where
	U: Future<Output = ()>
{
	fn is_terminated(&self) -> bool {
		self.done
	}
}

impl<T, U> Stream for AsyncStream<T, U>
where
	U: Future<Output = ()>
{
	type Item = T;

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

		if me.store.has_value() {
			return Poll::Ready(me.store.cell.take());
		}

		if *me.done { Poll::Ready(None) } else { Poll::Pending }
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		if self.done { (0, Some(0)) } else { (0, None) }
	}
}

/// Create an asynchronous [`Stream`] from an asynchronous generator function.
///
/// The provided function will be given a [`Yielder`], which, when called, causes the stream to yield an item:
/// ```
/// use async_stream_lite::async_stream;
/// use futures::{pin_mut, stream::StreamExt};
///
/// #[tokio::main]
/// async fn main() {
/// 	let stream = async_stream(|yielder| async move {
/// 		for i in 0..3 {
/// 			yielder.r#yield(i).await;
/// 		}
/// 	});
/// 	pin_mut!(stream);
/// 	while let Some(value) = stream.next().await {
/// 		println!("{value}");
/// 	}
/// }
/// ```
///
/// Streams may be returned by using `impl Stream<Item = T>`:
/// ```
/// use async_stream_lite::async_stream;
/// use futures::{
/// 	pin_mut,
/// 	stream::{Stream, StreamExt}
/// };
///
/// fn zero_to_three() -> impl Stream<Item = u32> {
/// 	async_stream(|yielder| async move {
/// 		for i in 0..3 {
/// 			yielder.r#yield(i).await;
/// 		}
/// 	})
/// }
///
/// #[tokio::main]
/// async fn main() {
/// 	let stream = zero_to_three();
/// 	pin_mut!(stream);
/// 	while let Some(value) = stream.next().await {
/// 		println!("{value}");
/// 	}
/// }
/// ```
///
/// or with [`futures::stream::BoxStream`]:
/// ```
/// use async_stream_lite::async_stream;
/// use futures::{
/// 	pin_mut,
/// 	stream::{BoxStream, StreamExt}
/// };
///
/// fn zero_to_three() -> BoxStream<'static, u32> {
/// 	Box::pin(async_stream(|yielder| async move {
/// 		for i in 0..3 {
/// 			yielder.r#yield(i).await;
/// 		}
/// 	}))
/// }
///
/// #[tokio::main]
/// async fn main() {
/// 	let mut stream = zero_to_three();
/// 	while let Some(value) = stream.next().await {
/// 		println!("{value}");
/// 	}
/// }
/// ```
///
/// Streams may also be implemented in terms of other streams:
/// ```
/// use async_stream_lite::async_stream;
/// use futures::{
/// 	pin_mut,
/// 	stream::{Stream, StreamExt}
/// };
///
/// fn zero_to_three() -> impl Stream<Item = u32> {
/// 	async_stream(|yielder| async move {
/// 		for i in 0..3 {
/// 			yielder.r#yield(i).await;
/// 		}
/// 	})
/// }
///
/// fn double<S: Stream<Item = u32>>(input: S) -> impl Stream<Item = u32> {
/// 	async_stream(|yielder| async move {
/// 		pin_mut!(input);
/// 		while let Some(value) = input.next().await {
/// 			yielder.r#yield(value * 2).await;
/// 		}
/// 	})
/// }
///
/// #[tokio::main]
/// async fn main() {
/// 	let stream = double(zero_to_three());
/// 	pin_mut!(stream);
/// 	while let Some(value) = stream.next().await {
/// 		println!("{value}");
/// 	}
/// }
/// ```
///
/// See also [`try_async_stream`], a variant of [`async_stream`] which supports try notation (`?`).
pub fn async_stream<T, F, U>(generator: F) -> AsyncStream<T, U>
where
	F: FnOnce(Yielder<T>) -> U,
	U: Future<Output = ()>
{
	let store = Arc::new(SharedStore::default());
	let generator = generator(Yielder { store: Arc::downgrade(&store) });
	AsyncStream { store, done: false, generator }
}

pub use self::r#try::{TryAsyncStream, try_async_stream};
