#![allow(clippy::tabs_in_doc_comments)]
#![cfg_attr(feature = "unstable-thread-local", feature(thread_local))]
#![cfg_attr(all(not(test), feature = "unstable-thread-local"), no_std)]

extern crate core;

use core::{
	cell::Cell,
	future::Future,
	marker::PhantomData,
	pin::Pin,
	ptr,
	task::{Context, Poll}
};

use futures_core::stream::{FusedStream, Stream};

#[cfg(test)]
mod tests;
mod r#try;

#[cfg(not(feature = "unstable-thread-local"))]
thread_local! {
	static STORE: Cell<*mut ()> = const { Cell::new(ptr::null_mut()) };
}
#[cfg(feature = "unstable-thread-local")]
#[thread_local]
static STORE: Cell<*mut ()> = Cell::new(ptr::null_mut());

pub struct Yielder<T> {
	_p: PhantomData<T>
}

impl<T> Yielder<T> {
	pub fn r#yield(&self, value: T) -> YieldFut<'_, T> {
		YieldFut { value: Some(value), _p: PhantomData }
	}
}

/// Future returned by an [`AsyncStream`]'s yield function.
///
/// This future must be `.await`ed inside the generator in order for the item to be yielded by the stream.
#[must_use = "stream will not yield this item unless the future returned by yield is awaited"]
pub struct YieldFut<'y, T> {
	value: Option<T>,
	_p: PhantomData<&'y ()>
}

impl<T> Unpin for YieldFut<'_, T> {}

impl<T> Future for YieldFut<'_, T> {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
		if self.value.is_none() {
			return Poll::Ready(());
		}

		fn op<T>(cell: &Cell<*mut ()>, value: &mut Option<T>) {
			let ptr = cell.get().cast::<Option<T>>();
			let option_ref = unsafe { ptr.as_mut() }.expect("attempted to use async_stream yielder outside of stream context or across threads");
			if option_ref.is_none() {
				*option_ref = value.take();
			}
		}

		#[cfg(not(feature = "unstable-thread-local"))]
		return STORE.with(|cell| {
			op(cell, &mut self.value);
			Poll::Pending
		});
		#[cfg(feature = "unstable-thread-local")]
		{
			op(&STORE, &mut self.value);
			Poll::Pending
		}
	}
}

struct Enter<'a, T> {
	_p: PhantomData<&'a T>,
	prev: *mut ()
}

fn enter<T>(dst: &mut Option<T>) -> Enter<'_, T> {
	fn op<T>(cell: &Cell<*mut ()>, dst: &mut Option<T>) -> *mut () {
		let prev = cell.get();
		cell.set((dst as *mut Option<T>).cast::<()>());
		prev
	}
	#[cfg(not(feature = "unstable-thread-local"))]
	let prev = STORE.with(|cell| op(cell, dst));
	#[cfg(feature = "unstable-thread-local")]
	let prev = op(&STORE, dst);
	Enter { _p: PhantomData, prev }
}

impl<T> Drop for Enter<'_, T> {
	fn drop(&mut self) {
		#[cfg(not(feature = "unstable-thread-local"))]
		STORE.with(|cell| cell.set(self.prev));
		#[cfg(feature = "unstable-thread-local")]
		STORE.set(self.prev);
	}
}

pin_project_lite::pin_project! {
	/// A [`Stream`] created from an asynchronous generator-like function.
	///
	/// To create an [`AsyncStream`], use the [`async_stream`] function.
	#[derive(Debug)]
	pub struct AsyncStream<T, U> {
		_p: PhantomData<T>,
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

		let mut dst = None;
		let res = {
			let _enter = enter(&mut dst);
			me.generator.poll(cx)
		};

		*me.done = res.is_ready();

		if dst.is_some() {
			return Poll::Ready(dst.take());
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
	let generator = generator(Yielder { _p: PhantomData });
	AsyncStream {
		_p: PhantomData,
		done: false,
		generator
	}
}

pub use self::r#try::{TryAsyncStream, try_async_stream};
