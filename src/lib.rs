mod wrp;
pub use wrp::wait_time;
pub use wrp::SerialWrapper;

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;
    use tokio_serial::SerialStream;
    use tokio::io::{AsyncWrite,AsyncRead};
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn check_wait_time() {
        let (s1, _s2) = SerialStream::pair().expect("Failed to open PTY");
        let out = wait_time(&s1);

        // 1.75ms is from the modbus spec, magic number is magic, but all baudrates > 19200 is
        //   expected to use that.
        assert_eq!(out, Duration::from_micros(1750));
    }

    async fn read_write<T,U>(tx: &mut T, rx: &mut U)  where T: AsyncRead+AsyncWrite+Unpin, U: AsyncRead+AsyncWrite+Unpin {
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

        eprintln!("Writing to tx");
        tx.write_all(write_buf)
            .await
            .expect("TX=>RX Failed to write bytes to PTY");

        eprintln!("Flushing tx");
        tx.flush().await.expect("TX: can flush fail? on a PTY");


        eprintln!("Reading from rx");
        let mut read_buf = [0; 256];
        assert!(read_buf.len() >= DATA_LEN);
        let read_num = rx
            .read(&mut read_buf)
            .await
            .expect("RX<=TX Failed to eat bytes from PTY");

        eprintln!("RX=>TX, Writing ack data");
        rx.write_all(&[1]).await.expect("RX=>TX Failed to write ack");
        rx.flush().await.expect("RX=>TX Failed to flush");

        // we write an ack back to the sender.
        eprintln!("TX<=RX Reading ack");
        let mut ack = [0; 1];
        tx.read_exact(&mut ack).await.expect("TX<=RX Reading failed?");
        assert_eq!(ack[0], 1);

        assert_eq!(DATA_LEN, read_num);
        assert_eq!(write_buf[0..DATA_LEN], read_buf[0..DATA_LEN]);
        eprintln!("Test cycle complete");
    }


    #[tokio::test]
    async fn wrapping() {
        use tokio::time::Instant;

        let (mut tx, mut rx) = SerialStream::pair().expect("Failed to open PTY");
        let start = Instant::now();
        read_write(&mut tx, &mut rx).await;
        let time_before = start.elapsed();

        // Wrap them in the delay code
        let mut tx: SerialWrapper<SerialStream> = tx.into();
        tx.set_delay(Duration::from_micros(0));
/*        let mut rx: SerialWrapper<SerialStream> = rx.into();
        rx.set_delay(Duration::from_millis(3));*/

        let start = Instant::now();
        read_write(&mut tx, &mut rx).await;
        let time_after = start.elapsed();
        println!("time_before {} time_after {}", time_before.as_micros(), time_after.as_micros());
        assert!(time_before < Duration::from_millis(1), "It should not take a millisecond to write a line."); 
        assert!(time_after > Duration::from_millis(1), "It should take a millisecond with our pacing code installed");
    }

    #[test]
    fn ensure_sync_port_works() {
        // As I had some troubles during development to make async read/write to a serial port
        // work, this opens the PTY naked, sync, and checks that read-write works.
        use serialport::TTYPort;
        use std::io::{Read, Write};
        use std::str;
        let (mut tx, mut rx) = TTYPort::pair().expect("Unable to create pseudo-terminal pair");
        let mut buf = [0u8; 512];
        for x in 1..6 {
            let msg = format!("Message #{x}");
            assert_eq!(tx.write(msg.as_bytes()).unwrap(), msg.len());
            // Receive on the slave
            let bytes_recvd = rx.read(&mut buf).unwrap();
            assert_eq!(bytes_recvd, msg.len());
            let msg_recvd = str::from_utf8(&buf[..bytes_recvd]).unwrap();
            assert_eq!(msg_recvd, msg);
        }
    }
}
