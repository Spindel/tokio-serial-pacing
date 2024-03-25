// Author: D.S. Ljungmark <spider@skuggor.se>, Modio FA AB
// SPDX-License-Identifier: AGPL-3.0-or-later
use pin_project_lite::pin_project;
use std::future::Future;
use std::io::Result as IoResult;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Duration, Instant};

pin_project! {
    #[derive(Debug)]
    pub struct SerialWrapper<S> {
        #[pin]
        inner: S,
        read_wait: Pin<Box<tokio::time::Sleep>>,
        write_wait: Pin<Box<tokio::time::Sleep>>,
        delay: Duration,
    }
}

// get the modbus wait time from a serialport / Serial stream
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

// In theory I could make this take anything that is AsyncRead + AsyncWrite as the input trait,
// but that seems to be over-the-top abstraction for the sake of abstraction, and I am not sure
// I would gain anything from it.
// impl<S>  From<S> for SerialWrapper<S> where S: AsyncRead + AsyncWrite { .... }
// The whole thing w ould be identical?
impl From<tokio_serial::SerialStream> for SerialWrapper<tokio_serial::SerialStream> {
    fn from(inner: tokio_serial::SerialStream) -> Self {
        use tokio::time::sleep_until;
        // we do not want to trigger a timer the first time read/write is polled, thus we
        // explicitly create a timer that expires 1ms in the past.
        let now = Instant::now();
        // let past = now.checked_sub(Duration::from_millis(1)).unwrap_or(now);
        let read_wait = Box::pin(sleep_until(past));
        let write_wait = Box::pin(sleep_until(past));
        SerialWrapper {
            inner,
            read_wait,
            write_wait,
            delay: Duration::ZERO,
        }
    }
}

// If you find it so desirable, implementing SerialPort for the SerialWrapper should be
// perfectly possible, but I do not see the point to do so, as there's no real trait bound _I_
// need that require it
impl<S> SerialWrapper<S> {
    /// Set the internal delay, if we want to override the default calculated one.
    pub fn set_delay(&mut self, delay: Duration) {
        // info!(delay = delay.as_micros(), "Setting serial flush/read delay");
        self.delay = delay;
    }
}

impl<S> AsyncRead for SerialWrapper<S>
where
    S: AsyncRead + Unpin + std::fmt::Debug,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        eprintln!("Poll read enter {self:?}");
        let this = self.project();
        // If our read wait is not yet ready, we return early.
        // This is to enforce a silence between us finishing a write_flush and starting a
        // receive, as there should be a 3.5 character timeout in between
        
        // Check if the deadline is in the future before we await the timer, otherwise it causes a
        // few ms of extra time spent, for some reason.
        if this.read_wait.deadline() >= Instant::now() {
            if this.read_wait.as_mut().poll(cx).is_pending() {
                eprintln!("Poll read pending");
                return Poll::Pending;
            }
        }
        match this.inner.poll_read(cx, buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(data) => {
                eprintln!("Arming write delay due to poll_read");
                let wait = Instant::now() + *this.delay;
                this.write_wait.as_mut().reset(wait);
                Poll::Ready(data)
            }
        }
    }
}
impl<S> AsyncWrite for SerialWrapper<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<IoResult<usize>> {
        let this = self.project();
        eprintln!("Poll write enter");
        // Check if we are in read to Write pacing or not.
        if this.write_wait.as_mut().poll(cx).is_pending() {
            eprintln!("Poll write pending");
            return Poll::Pending;
        }
        match this.inner.poll_write(cx, buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(data) => {
                // Flush has finished, re-arm the read_wait timeout so our next read will be
                // after a moment of silence
                eprintln!("Arming read delay due to poll_write");
                let wait = Instant::now() + *this.delay;
                this.read_wait.as_mut().reset(wait);
                Poll::Ready(data)
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<IoResult<()>> {
        eprintln!("Entering poll_flush");
        let this = self.project();
        if this.write_wait.as_mut().poll(cx).is_pending() {
            eprintln!("Poll flush pending");
            return Poll::Pending;
        }
        // After a succesful _flush_ of the write buffer, we want to have a moment of Silence
        // before the next _read_ or _write_ of data.
        // Thus we probably want to write a timeout on "flush" and then on the next write, or
        // read, chck said timeout and return pending?
        match this.inner.poll_flush(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(data) => {
                // Flush has finished, re-arm the read_wait timeout so our next read will be
                // after a moment of silence
                eprintln!("Arming read delay due to poll_flush");
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
