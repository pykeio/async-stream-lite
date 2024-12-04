# `async-stream-lite`
It's [`async-stream`](https://lib.rs/crates/async-stream), but without proc macros.

```rs
use async_stream_lite::async_stream;

use futures_util::{pin_mut, stream::StreamExt};

#[tokio::main]
async fn main() {
	let stream = async_stream(|yielder| async move {
		for i in 0..3 {
			yielder.r#yield(i).await;
		}
	});
	pin_mut!(stream);
	while let Some(value) = stream.next().await {
		println!("got {}", value);
	}
}
```

## `#![no_std]` support
`async-stream-lite` supports `#![no_std]`, but requires `alloc`.
