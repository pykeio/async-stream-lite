use futures::{
	Stream, StreamExt,
	future::join_all,
	pin_mut,
	stream::{BoxStream, FusedStream}
};
use tokio::sync::mpsc;

use super::{YieldFut, async_stream};

#[tokio::test]
async fn noop_stream() {
	let s = async_stream(|_yield: fn(()) -> YieldFut<()>| async move {});
	pin_mut!(s);

	#[allow(clippy::never_loop)]
	while s.next().await.is_some() {
		unreachable!();
	}
}

#[tokio::test]
async fn empty_stream() {
	let mut ran = false;

	{
		let r = &mut ran;
		let s = async_stream(|_yield: fn(()) -> YieldFut<()>| async move {
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
}

#[tokio::test]
async fn yield_single_value() {
	let s = async_stream(|r#yield| async move {
		r#yield("hello").await;
	});

	let values: Vec<_> = s.collect().await;

	assert_eq!(1, values.len());
	assert_eq!("hello", values[0]);
}

#[tokio::test]
async fn fused() {
	let s = async_stream(|r#yield| async move {
		r#yield("hello").await;
	});
	pin_mut!(s);

	assert!(!s.is_terminated());
	assert_eq!(s.next().await, Some("hello"));
	assert_eq!(s.next().await, None);

	assert!(s.is_terminated());
	assert_eq!(s.next().await, None);
}

#[tokio::test]
async fn yield_multi_value() {
	let stream = async_stream(|r#yield| async move {
		r#yield("hello").await;
		r#yield("world").await;
		r#yield("foobar").await;
	});

	let values: Vec<_> = stream.collect().await;

	assert_eq!(3, values.len());
	assert_eq!("hello", values[0]);
	assert_eq!("world", values[1]);
	assert_eq!("foobar", values[2]);
}

#[tokio::test]
async fn unit_yield_in_select() {
	#[allow(clippy::unused_async)]
	async fn do_stuff_async() {}

	let stream = async_stream(|r#yield| async move {
		tokio::select! {
			() = do_stuff_async() => r#yield(()).await,
			else => r#yield(()).await
		}
	});

	let values: Vec<_> = stream.collect().await;
	assert_eq!(values.len(), 1);
}

#[tokio::test]
#[allow(clippy::unused_async)]
async fn yield_with_select() {
	async fn do_stuff_async() {}
	async fn more_async_work() {}

	let stream = async_stream(|r#yield| async move {
		tokio::select! {
			() = do_stuff_async() => r#yield("hey").await,
			() = more_async_work() => r#yield("hey").await,
			else => r#yield("hey").await
		}
	});

	let values: Vec<_> = stream.collect().await;
	assert_eq!(values, vec!["hey"]);
}

#[tokio::test]
async fn return_stream() {
	fn build_stream() -> impl Stream<Item = u32> {
		async_stream(|r#yield| async move {
			r#yield(1).await;
			r#yield(2).await;
			r#yield(3).await;
		})
	}

	let stream = build_stream();

	let values: Vec<_> = stream.collect().await;
	assert_eq!(3, values.len());
	assert_eq!(1, values[0]);
	assert_eq!(2, values[1]);
	assert_eq!(3, values[2]);
}

#[tokio::test]
async fn boxed_stream() {
	fn build_stream() -> BoxStream<'static, u32> {
		Box::pin(async_stream(|r#yield| async move {
			r#yield(1).await;
			r#yield(2).await;
			r#yield(3).await;
		}))
	}

	let stream = build_stream();

	let values: Vec<_> = stream.collect().await;
	assert_eq!(3, values.len());
	assert_eq!(1, values[0]);
	assert_eq!(2, values[1]);
	assert_eq!(3, values[2]);
}

#[tokio::test]
async fn consume_channel() {
	let (tx, mut rx) = mpsc::channel(10);

	let stream = async_stream(|r#yield| async move {
		while let Some(v) = rx.recv().await {
			r#yield(v).await;
		}
	});

	pin_mut!(stream);

	for i in 0..3 {
		assert!(tx.send(i).await.is_ok());
		assert_eq!(Some(i), stream.next().await);
	}

	drop(tx);
	assert_eq!(None, stream.next().await);
}

#[tokio::test]
async fn borrow_self() {
	struct Data(String);

	impl Data {
		fn stream(&self) -> impl Stream<Item = &str> + '_ {
			async_stream(|r#yield| async move {
				r#yield(&self.0[..]).await;
			})
		}
	}

	let data = Data("hello".to_string());
	let s = data.stream();
	pin_mut!(s);

	assert_eq!(Some("hello"), s.next().await);
}

#[tokio::test]
async fn borrow_self_boxed() {
	struct Data(String);

	impl Data {
		fn stream(&self) -> BoxStream<'_, &str> {
			Box::pin(async_stream(|r#yield| async move {
				r#yield(&self.0[..]).await;
			}))
		}
	}

	let data = Data("hello".to_string());
	let s = data.stream();
	pin_mut!(s);

	assert_eq!(Some("hello"), s.next().await);
}

#[tokio::test]
async fn stream_in_stream() {
	let s = async_stream(|r#yield| async move {
		let s = async_stream(|r#yield| async move {
			for i in 0..3 {
				r#yield(i).await;
			}
		});
		pin_mut!(s);

		while let Some(v) = s.next().await {
			r#yield(v).await;
		}
	});

	let values: Vec<_> = s.collect().await;
	assert_eq!(3, values.len());
}

#[tokio::test]
async fn streamception() {
	let s = async_stream(|r#yield| async move {
		let s = async_stream(|r#yield| async move {
			let s = async_stream(|r#yield| async move {
				let s = async_stream(|r#yield| async move {
					let s = async_stream(|r#yield| async move {
						for i in 0..3 {
							r#yield(i).await;
						}
					});
					pin_mut!(s);

					while let Some(v) = s.next().await {
						r#yield(v).await;
					}
				});
				pin_mut!(s);

				while let Some(v) = s.next().await {
					r#yield(v).await;
				}
			});
			pin_mut!(s);

			while let Some(v) = s.next().await {
				r#yield(v).await;
			}
		});
		pin_mut!(s);

		while let Some(v) = s.next().await {
			r#yield(v).await;
		}
	});

	let values: Vec<_> = s.collect().await;
	assert_eq!(3, values.len());
}

#[tokio::test]
async fn yield_non_unpin_value() {
	let s: Vec<_> = async_stream(|r#yield| async move {
		for i in 0..3 {
			r#yield(async move { i }).await;
		}
	})
	.buffered(1)
	.collect()
	.await;

	assert_eq!(s, vec![0, 1, 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multithreaded() {
	fn build_stream() -> impl Stream<Item = u32> {
		async_stream(|r#yield| async move {
			r#yield(1).await;
			r#yield(2).await;
			r#yield(3).await;
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
}
