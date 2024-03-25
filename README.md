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
