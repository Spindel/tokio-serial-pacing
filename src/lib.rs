// Author: D.S. Ljungmark <spider@skuggor.se>, Modio FA AB
// SPDX-License-Identifier: MIT
//! This crate is a simple wrapper around
//!
//! [tokio_serial::SerialStream](https://docs.rs/tokio-serial/latest/tokio_serial/struct.SerialStream.html)
//! for use with [tokio-modbus](https://docs.rs/tokio-modbus/latest/tokio_modbus/) in RTU mode.
//! The wrappers can ensure that an application obeys the inter frame delay of 3.5 characters
//! between reading and writing.
//!
//! The helper `wait_time` can attempt to calculate the proper delay from the serial port settings.
//! In practice, the timers in tokio are not very exact, but since in theory, it's always okay to
//! have a longer delay, that is not considered a big concern.
//!
//! the SerialPacing  trait just wraps the "set_delay" function as shared functionality, and then
//! SerialReadPacing and SerialWritePacing implement the functionality around AsyncRead and
//! AsyncWrite traits.
//!
//! Example
//! ```rust
//! #[tokio::main(flavor="current_thread")]
//! async fn main() -> std::io::Result<()> {
//!   use tokio_serial::{SerialPort, SerialStream};
//!   use tokio_serial_pacing::{SerialPacing, SerialWritePacing};
//!
//!   let (tx, mut rx) = SerialStream::pair().expect("Failed to open PTY");
//!   let mut rx: SerialWritePacing<SerialStream> = rx.into();
//!   rx.set_delay(std::time::Duration::from_millis(3));
//!   Ok(())
//! }
//! ```
mod wrp;
pub use wrp::wait_time;
pub use wrp::{SerialPacing, SerialReadPacing, SerialWritePacing};

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio::time::Duration;
    use tokio::time::Instant;
    use tokio_serial::SerialStream;

    #[tokio::test]
    async fn check_wait_time() {
        let (s1, _s2) = SerialStream::pair().expect("Failed to open PTY");
        let out = wait_time(&s1);

        // 1.75ms is from the modbus spec, magic number is magic, but all baudrates > 19200 is
        //   expected to use that.
        assert_eq!(out, Duration::from_micros(1750));
    }

    // Perform a read-write operation
    async fn read_write<T, U>(mut tx: T, mut rx: U)
    where
        // Trait bounds how I love you, wait. That other thing. Ewww.
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        U: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let write_buf = b"The implementation of RTU reception driver may imply the management of a lot of interruptions due to the t 1.5 and t 3.5 timers.";
        /* With high communication baud rates, this leads to a heavy CPU load. Consequently these two timers must be strictly respected when the
        baud rate is equal or lower than 19200 Bps. For baud rates greater than 19200 Bps, fixed values for the 2 timers should be used: it is
        recommended to use a value of 750us for the inter-character time-out (t 1.5 ) and a value of 1.750ms for inter-frame delay (t 3.5 ).*/
        const DATA_LEN: usize = 128;
        assert_eq!(
            write_buf.len(),
            DATA_LEN,
            "check that the buffer we work with is around"
        );

        let tx_task = tokio::spawn(async move {
            eprintln!("TX=>RX:  Writing large buf");
            tx.write_all(write_buf)
                .await
                .expect("TX=>RX Failed to write bytes to PTY");
            eprintln!("TX=>RX Flushing");
            tx.flush().await.expect("TX: can flush fail? on a PTY");
            // we write an ack back to the sender.
            eprintln!("TX<=RX Reading ack");
            let mut ack = [0; 1];
            tx.read_exact(&mut ack)
                .await
                .expect("TX<=RX Reading failed?");
            assert_eq!(ack[0], 1);
            write_buf
        });

        let rx_task = tokio::spawn(async move {
            eprintln!("RX>=TX Reading large(?) buf");
            let mut read_buf = [0; 256];
            assert!(read_buf.len() >= DATA_LEN);
            let read_num = rx
                .read(&mut read_buf)
                .await
                .expect("RX<=TX Failed to eat bytes from PTY");

            eprintln!("RX=>TX, Writing ack data");
            rx.write_all(&[1])
                .await
                .expect("RX=>TX Failed to write ack");
            rx.flush().await.expect("RX=>TX Failed to flush");
            (read_buf, read_num)
        });

        let (read_buf, read_num) = rx_task.await.expect("Error in rx side");
        let write_buf = tx_task.await.expect("Error in tx side");
        assert_eq!(DATA_LEN, read_num);
        assert_eq!(write_buf[0..DATA_LEN], read_buf[0..DATA_LEN]);
        eprintln!("Test cycle complete");
    }

    #[tokio::test]
    async fn check_write_pacing() {
        let time_before = {
            let (tx, rx) = SerialStream::pair().expect("Failed to open PTY");
            let start = Instant::now();
            read_write(tx, rx).await;
            start.elapsed().as_micros()
        };
        assert!(
            time_before < 1000,
            "It should not take a millisecond normally."
        );
        let time_after = {
            let (tx, rx) = SerialStream::pair().expect("Failed to open PTY");
            // Wrap the _rx_ in the delay code, as it must ensure that it only writes a reply after the
            // elapsed timeout has happened.
            let mut rx: SerialWritePacing<SerialStream> = rx.into();
            rx.set_delay(Duration::from_micros(1000));

            let start = Instant::now();
            read_write(tx, rx).await;
            start.elapsed().as_micros()
        };
        println!("time_before={time_before} time_after={time_after}");
        assert!(
            time_after > 1000,
            "It should take a millisecond with our pacing code installed"
        );
    }

    #[tokio::test]
    async fn check_read_pacing() {
        let time_before = {
            let (tx, rx) = SerialStream::pair().expect("Failed to open PTY");
            let start = Instant::now();
            read_write(tx, rx).await;
            start.elapsed().as_micros()
        };
        assert!(
            time_before < 1000,
            "It should not take a millisecond normally."
        );

        let time_after = {
            let (tx, rx) = SerialStream::pair().expect("Failed to open PTY");
            // Wrap the tx side in a delay code, making it wait between writing and reading.
            let mut tx: SerialReadPacing<SerialStream> = tx.into();
            tx.set_delay(Duration::from_micros(1000));
            let start = Instant::now();
            read_write(tx, rx).await;
            start.elapsed().as_micros()
        };
        println!("time_before={time_before} time_after={time_after}");
        assert!(
            time_after > 1000,
            "It should take a millisecond with our pacing code installed"
        );
    }

    #[test]
    fn ensure_sync_port_works() {
        // As I had some troubles during development to make async read/write to a serial port
        // work, this opens the PTY naked, sync, and checks that read-write works.
        use serialport::TTYPort;
        use std::io::{Read, Write};
        let (mut tx, mut rx) = TTYPort::pair().expect("Unable to create pseudo-terminal pair");
        let mut buf = [0u8; 512];
        for x in 1..6 {
            let msg = format!("Message #{x}");
            assert_eq!(tx.write(msg.as_bytes()).unwrap(), msg.len());
            // Receive on the slave
            let bytes_recvd = rx.read(&mut buf).unwrap();
            assert_eq!(bytes_recvd, msg.len());
            let msg_recvd = std::str::from_utf8(&buf[..bytes_recvd]).unwrap();
            assert_eq!(msg_recvd, msg);
        }
    }
}
