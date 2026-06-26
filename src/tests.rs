use std::future::Future;

use futures::{
	Stream, StreamExt,
	future::join_all,
	pin_mut,
	stream::{BoxStream, FusedStream}
};
use tokio::sync::mpsc;

use super::{Yielder, async_stream};

pub(crate) fn run_test(fut: impl Future<Output = ()>) {
	tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(fut)
}

#[test]
fn noop_stream() {
	run_test(async {
		let s = async_stream(|_yielder: Yielder<()>| async move {});
		pin_mut!(s);

		#[allow(clippy::never_loop)]
		while s.next().await.is_some() {
			unreachable!();
		}
	})
}

#[test]
fn empty_stream() {
	run_test(async {
		let mut ran = false;

		{
			let r = &mut ran;
			let s = async_stream(|_yield: Yielder<()>| async move {
				*r = true;
				println!("hello world!");
			});
			pin_mut!(s);

			#[allow(clippy::never_loop)]
			while s.next().await.is_some() {
				unreachable!();
			}
		}

		assert!(ran);
	})
}

#[test]
fn yield_single_value() {
	run_test(async {
		let s = async_stream(|yielder| async move {
			yielder.y("hello").await;
		});

		let values: Vec<_> = s.collect().await;

		assert_eq!(1, values.len());
		assert_eq!("hello", values[0]);
	})
}

#[test]
fn fused() {
	run_test(async {
		let s = async_stream(|yielder| async move {
			yielder.y("hello").await;
		});
		pin_mut!(s);

		assert!(!s.is_terminated());
		assert_eq!(s.next().await, Some("hello"));
		assert_eq!(s.next().await, None);

		assert!(s.is_terminated());
		assert_eq!(s.next().await, None);
	})
}

#[test]
fn yield_multi_value() {
	run_test(async {
		let stream = async_stream(|yielder| async move {
			yielder.y("hello").await;
			yielder.y("world").await;
			yielder.y("foobar").await;
		});

		let values: Vec<_> = stream.collect().await;

		assert_eq!(3, values.len());
		assert_eq!("hello", values[0]);
		assert_eq!("world", values[1]);
		assert_eq!("foobar", values[2]);
	})
}

#[test]
fn unit_yield_in_select() {
	run_test(async {
		#[allow(clippy::unused_async)]
		async fn do_stuff_async() {}

		let stream = async_stream(|yielder| async move {
			tokio::select! {
				() = do_stuff_async() => yielder.y(()).await,
				else => yielder.y(()).await
			}
		});

		let values: Vec<_> = stream.collect().await;
		assert_eq!(values.len(), 1);
	})
}

#[test]
#[allow(clippy::unused_async)]
fn yield_with_select() {
	run_test(async {
		async fn do_stuff_async() {}
		async fn more_async_work() {}

		let stream = async_stream(|yielder| async move {
			tokio::select! {
				() = do_stuff_async() => yielder.y("hey").await,
				() = more_async_work() => yielder.y("hey").await,
				else => yielder.y("hey").await
			}
		});

		let values: Vec<_> = stream.collect().await;
		assert_eq!(values, vec!["hey"]);
	})
}

#[test]
fn return_stream() {
	run_test(async {
		fn build_stream() -> impl Stream<Item = u32> {
			async_stream(|yielder| async move {
				yielder.y(1).await;
				yielder.y(2).await;
				yielder.y(3).await;
			})
		}

		let stream = build_stream();

		let values: Vec<_> = stream.collect().await;
		assert_eq!(3, values.len());
		assert_eq!(1, values[0]);
		assert_eq!(2, values[1]);
		assert_eq!(3, values[2]);
	})
}

#[test]
fn boxed_stream() {
	run_test(async {
		fn build_stream() -> BoxStream<'static, u32> {
			Box::pin(async_stream(|yielder| async move {
				yielder.y(1).await;
				yielder.y(2).await;
				yielder.y(3).await;
			}))
		}

		let stream = build_stream();

		let values: Vec<_> = stream.collect().await;
		assert_eq!(3, values.len());
		assert_eq!(1, values[0]);
		assert_eq!(2, values[1]);
		assert_eq!(3, values[2]);
	})
}

#[test]
fn consume_channel() {
	run_test(async {
		let (tx, mut rx) = mpsc::channel(10);

		let stream = async_stream(|yielder| async move {
			while let Some(v) = rx.recv().await {
				yielder.y(v).await;
			}
		});

		pin_mut!(stream);

		for i in 0..3 {
			assert!(tx.send(i).await.is_ok());
			assert_eq!(Some(i), stream.next().await);
		}

		drop(tx);
		assert_eq!(None, stream.next().await);
	})
}

#[test]
fn borrow_self() {
	run_test(async {
		struct Data(String);

		impl Data {
			fn stream(&self) -> impl Stream<Item = &str> + '_ {
				async_stream(|yielder| async move {
					yielder.y(&self.0[..]).await;
				})
			}
		}

		let data = Data("hello".to_string());
		let s = data.stream();
		pin_mut!(s);

		assert_eq!(Some("hello"), s.next().await);
	})
}

#[test]
fn borrow_self_boxed() {
	run_test(async {
		struct Data(String);

		impl Data {
			fn stream(&self) -> BoxStream<'_, &str> {
				Box::pin(async_stream(|yielder| async move {
					yielder.y(&self.0[..]).await;
				}))
			}
		}

		let data = Data("hello".to_string());
		let s = data.stream();
		pin_mut!(s);

		assert_eq!(Some("hello"), s.next().await);
	})
}

#[test]
fn stream_in_stream() {
	run_test(async {
		let s = async_stream(|yielder| async move {
			let s = async_stream(|yielder| async move {
				for i in 0..3 {
					yielder.y(i).await;
				}
			});
			pin_mut!(s);

			while let Some(v) = s.next().await {
				yielder.y(v).await;
			}
		});

		let values: Vec<_> = s.collect().await;
		assert_eq!(3, values.len());
	})
}

#[test]
fn streamception() {
	run_test(async {
		let s = async_stream(|yielder| async move {
			let s = async_stream(|yielder| async move {
				let s = async_stream(|yielder| async move {
					let s = async_stream(|yielder| async move {
						let s = async_stream(|yielder| async move {
							for i in 0..3 {
								yielder.y(i).await;
							}
						});
						pin_mut!(s);

						while let Some(v) = s.next().await {
							yielder.y(v).await;
						}
					});
					pin_mut!(s);

					while let Some(v) = s.next().await {
						yielder.y(v).await;
					}
				});
				pin_mut!(s);

				while let Some(v) = s.next().await {
					yielder.y(v).await;
				}
			});
			pin_mut!(s);

			while let Some(v) = s.next().await {
				yielder.y(v).await;
			}
		});

		let values: Vec<_> = s.collect().await;
		assert_eq!(3, values.len());
	})
}

#[test]
fn yield_non_unpin_value() {
	run_test(async {
		let s: Vec<_> = async_stream(|yielder| async move {
			for i in 0..3 {
				yielder.y(async move { i }).await;
			}
		})
		.buffered(1)
		.collect()
		.await;

		assert_eq!(s, vec![0, 1, 2]);
	})
}

#[cfg(not(miri))]
#[test]
fn multithreaded() {
	tokio::runtime::Builder::new_multi_thread()
		.worker_threads(4)
		.build()
		.unwrap()
		.block_on(async {
			fn build_stream() -> impl Stream<Item = u32> {
				async_stream(|yielder| async move {
					yielder.y(1).await;
					yielder.y(2).await;
					yielder.y(3).await;
				})
			}

			let mut futures = Vec::new();
			for _ in 0..4 {
				futures.push(tokio::spawn(async move {
					let stream = build_stream();
					let values: Vec<_> = stream.collect().await;
					assert_eq!(3, values.len());
					assert_eq!(1, values[0]);
					assert_eq!(2, values[1]);
					assert_eq!(3, values[2]);
				}));
			}
			join_all(futures).await;
		})
}

#[test]
#[should_panic = "attempted to use async-stream-lite yielder outside of stream context or across threads"]
fn test_move_yielder() {
	run_test(async {
		let mut slot = None;
		let s = async_stream(|yielder: Yielder<()>| async {
			slot.replace(yielder);
		});
		pin_mut!(s);

		let _ = s.next().await;
		drop(s);

		slot.take().unwrap().y(()).await;
	})
}
