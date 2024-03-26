# Serial pacing wrappers around `tokio_serial::SerialStream` 

This can be used on anything that implements AsyncRead + AsyncWrite, but is
currently only tested for serial ports.

I built this as the Modbus RTU specification requires a certain time of silence
between Reads and Writes to be in spec. (It also requires a specific
inter-character timing that I will just ignore)

This code is NOT at all specific enough about the timer timeouts, as tokio
timeouts are not high precision enough to guarantee anything. However, it works
well enough to ensure _at least_ a certain amount of time goes between read &
write operations.

Due to reasons (...)  I've also implemented the inverse, Waiting after a write
before reading, although I have no idea why that would be necessary.


# Example

```rust
use std::time::Duration;
use tokio_serial::{SerialPort, SerialStream};
use tokio_serial_pacing::{SerialPacing, SerialWritePacing};

#[tokio::main(flavor="current_thread")]
async fn main() -> std::io::Result<()> {
  let (tx, mut rx) = SerialStream::pair().expect("Failed to open PTY");
  let mut rx: SerialWritePacing<SerialStream> = rx.into();
  rx.set_delay(Duration::from_millis(3));
  Ok(())
}
```
