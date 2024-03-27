// Author: D.S. Ljungmark <spider@skuggor.se>, Modio FA AB
// SPDX-License-Identifier: MIT
use pin_project_lite::pin_project;
use std::future::Future;
use std::io::Result as IoResult;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{sleep_until, Duration, Instant};

/// Shared trait for [SerialReadPacing] and [SerialWritePacing] that implements `set_delay`
pub trait SerialPacing {
    /// Set the internal delay, if we want to override the default, which is [Duration::ZERO]
    fn set_delay(&mut self, delay: Duration);
}

pin_project! {
    #[derive(Debug)]
    /// Implement pacing by waiting between Write and Read operations.
    ///
    /// Will wait for _at least_ `delay` after an AsyncWrite and the next Read operation.
    ///
    /// Example
    /// ```rust
    /// # #[tokio::main(flavor="current_thread")]
    /// # async fn main() -> std::io::Result<()> {
    /// # use tokio_serial_pacing::SerialReadPacing;
    /// # use tokio_serial::SerialStream;
    ///   let (tx, rx) = tokio_serial::SerialStream::pair().expect("Failed to open PTY");
    ///   let mut rx: SerialReadPacing<SerialStream> = rx.into();
    /// #     Ok(())
    /// # }
    /// ```
    pub struct SerialReadPacing<S> {
        #[pin]
        inner: S,
        read_wait: Pin<Box<tokio::time::Sleep>>,
        delay: Duration,
    }
}

pin_project! {
    #[derive(Debug)]
    /// Implement pacing by waiting between Read and Write operations.
    ///
    /// It will wait for  _at least_ `delay` after an Async Read operation, before performing the
    /// next _write_ operation.
    ///
    /// Other Read operations are not delayed.
    ///
    /// Example
    /// ```rust
    /// # #[tokio::main(flavor="current_thread")]
    /// # async fn main() -> std::io::Result<()> {
    /// # use tokio_serial_pacing::SerialWritePacing;
    /// # use tokio_serial::SerialStream;
    ///   let (tx, rx) = tokio_serial::SerialStream::pair().expect("Failed to open PTY");
    ///   let mut rx: SerialWritePacing<SerialStream> = rx.into();
    /// #     Ok(())
    /// # }
    /// ```
    pub struct SerialWritePacing<S> {
        #[pin]
        inner: S,
        write_wait: Pin<Box<tokio::time::Sleep>>,
        delay: Duration,
    }
}

/// Calculate the Modbus RTU 3.5t wait time from a serial port
///
///
/// This should calculate a Delay based on baudrate/parity/data bits on a SerialPort
///
/// Example
/// ```rust
/// # #[tokio::main(flavor="current_thread")]
/// # async fn main() -> std::io::Result<()> {
/// # use tokio_serial::{SerialPort, SerialStream};
/// # use std::time::Duration;
///   use tokio_serial_pacing::{SerialPacing, SerialWritePacing, wait_time};
///   let (tx, mut rx) = tokio_serial::SerialStream::pair().expect("Failed to open PTY");
///   let delay = wait_time(&rx);
///   assert_eq!(delay, Duration::from_micros(1750), "High baudrate should have a delay of 1.75ms");
///   let mut rx: SerialWritePacing<SerialStream> = rx.into();
///   rx.set_delay(delay);
/// #     Ok(())
/// # }
/// ```
pub fn wait_time(port: &impl tokio_serial::SerialPort) -> Duration {
    use tokio_serial::{DataBits, Parity, StopBits};
    let byte_size = {
        let data_bits = match port.data_bits() {
            // Why isn't this a numeric tagged enum so I could just do "as u8" and be done with
            // it?
            Ok(DataBits::Five) => 5,
            Ok(DataBits::Six) => 6,
            Ok(DataBits::Seven) => 7,
            Ok(DataBits::Eight) => 8,
            Err(_) => 8,
        };
        let stop_bits = match port.stop_bits() {
            Ok(StopBits::One) => 1,
            Ok(StopBits::Two) => 2,
            Err(_) => 1,
        };
        let parity_bits = match port.parity() {
            Ok(Parity::None) => 0,
            Ok(Parity::Even) | Ok(Parity::Odd) => 1,
            Err(_) => 0,
        };
        data_bits + stop_bits + parity_bits
    };

    let wait_time = {
        let baudrate = port.baud_rate().unwrap_or(9600) as u64;
        if baudrate > 19200 {
            1750
        } else {
            // per character wait in seconds is byte_size / baudrate.
            // We scale byte size with 1_000_000 to get microseconds
            // And then with 3.5 to get the 3.5 character wait time required.
            (3_500_000 * byte_size) / baudrate
        }
    };
    Duration::from_micros(wait_time)
}

// we do not want to trigger a timer the first time read/write is polled, thus we
// explicitly create a timer that expires 1ms in the past.
fn past() -> Instant {
    let now = Instant::now();
    now.checked_sub(Duration::from_millis(1)).unwrap_or(now)
}

/// Wrap a [tokio_serial::SerialStream] with Read Pacing, causing a delay between a Write and the
/// next Read.
///
/// This is probably not useful in practice.
///
/// In theory I could make this take anything that is AsyncRead + AsyncWrite as the input trait,
/// but that seems to be over-the-top abstraction for the sake of abstraction, and I am not sure
/// I would gain anything from it.
///
impl<S> From<S> for SerialReadPacing<S>
where
    S: AsyncWrite + AsyncRead + Unpin,
{
    fn from(inner: S) -> Self {
        let past = past();
        let read_wait = Box::pin(sleep_until(past));
        Self {
            inner,
            read_wait,
            delay: Duration::ZERO,
        }
    }
}

/// Wrap a [tokio_serial::SerialStream] with Write Pacing, causing a delay between a Read and the
/// next Write operation.
/// This is required according to the Modbus RTU standard, and some equipment will (properly) fail
/// to respond to commands that come too quickly after another message was sent.
///
///
///
/// In theory I could make this take anything that is AsyncRead + AsyncWrite as the input trait,
/// but that seems to be over-the-top abstraction for the sake of abstraction, and I am not sure
/// I would gain anything from it.
impl<S> From<S> for SerialWritePacing<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn from(inner: S) -> Self {
        let past = past();
        let write_wait = Box::pin(sleep_until(past));
        Self {
            inner,
            write_wait,
            delay: Duration::ZERO,
        }
    }
}

/// If you find it so desirable, implementing SerialPort for the SerialReadPacing should be
/// perfectly possible, but I do not see the point to do so, as there's no real trait bound _I_
/// need that require it
impl<S> SerialPacing for SerialReadPacing<S> {
    /// Set the internal delay, if we want to override the default calculated one.
    fn set_delay(&mut self, delay: Duration) {
        // info!(delay = delay.as_micros(), "Setting serial flush/read delay");
        self.delay = delay;
    }
}

/// If you find it so desirable, implementing SerialPort for the SerialWritePacing should be
/// perfectly possible, but I do not see the point to do so, as there's no real trait bound _I_
/// need that require it
impl<S> SerialPacing for SerialWritePacing<S> {
    /// Set the internal delay, if we want to override the default calculated one.
    fn set_delay(&mut self, delay: Duration) {
        // info!(delay = delay.as_micros(), "Setting serial flush/read delay");
        self.delay = delay;
    }
}

impl<S> AsyncRead for SerialReadPacing<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let this = self.project();
        // If our read wait is not yet ready, we return early.
        // This is to enforce a silence between us finishing a write_flush and starting a
        // receive, as there should be a 3.5 character timeout in between

        // Check if the deadline is in the future before we await the timer, otherwise it causes a
        // few ms of extra time spent, for some reason.
        if this.read_wait.deadline() >= Instant::now() {
            // Now schedule the timer/timeout to wait for the deadline.
            if this.read_wait.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
        }
        this.inner.poll_read(cx, buf)
    }
}
impl<S> AsyncWrite for SerialReadPacing<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<IoResult<usize>> {
        let this = self.project();
        match this.inner.poll_write(cx, buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(data) => {
                // Flush has finished, re-arm the read_wait timeout so our next read will be
                // after a moment of silence
                let wait = Instant::now() + *this.delay;
                this.read_wait.as_mut().reset(wait);
                Poll::Ready(data)
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        let this = self.project();

        // After a succesful _flush_ of the write buffer, we want to have a moment of Silence
        // before the next _read_ or _write_ of data.
        // Thus we probably want to write a timeout on "flush" and then on the next write, or
        // read, chck said timeout and return pending?
        match this.inner.poll_flush(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(data) => {
                // Flush has finished, re-arm the read_wait timeout so our next read will be
                // after a moment of silence
                let wait = Instant::now() + *this.delay;
                this.read_wait.as_mut().reset(wait);
                Poll::Ready(data)
            }
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        self.project().inner.poll_shutdown(cx)
    }
}

impl<S> AsyncRead for SerialWritePacing<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let this = self.project();
        // If our read wait is not yet ready, we return early.
        // This is to enforce a silence between us finishing a write_flush and starting a
        // receive, as there should be a 3.5 character timeout in between
        match this.inner.poll_read(cx, buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(data) => {
                let wait = Instant::now() + *this.delay;
                this.write_wait.as_mut().reset(wait);
                Poll::Ready(data)
            }
        }
    }
}
impl<S> AsyncWrite for SerialWritePacing<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<IoResult<usize>> {
        let this = self.project();
        // Check if the deadline is in the future before we await the timer, otherwise it causes a
        // few ms of extra time spent, for some reason.
        if this.write_wait.deadline() >= Instant::now() {
            // Now schedule the timer/timeout to wait for the deadline.
            if this.write_wait.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
        }
        this.inner.poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        let this = self.project();
        // Check if the deadline is in the future before we await the timer, otherwise it causes a
        // few ms of extra time spent, for some reason.
        if this.write_wait.deadline() >= Instant::now() {
            // Now schedule the timer/timeout to wait for the deadline.
            if this.write_wait.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
        }
        this.inner.poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        self.project().inner.poll_shutdown(cx)
    }
}
