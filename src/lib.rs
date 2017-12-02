//! Tokio wrappers which apply timeouts to IO operations.
//!
//! These timeouts are analagous to the read and write timeouts on traditional
//! blocking sockets. A timeout countdown is initiated when a read/write
//! operation returns `WouldBlock`. If a read/write does not return successfully
//! the before the countdown expires, `TimedOut` is returned.
#![doc(html_root_url = "https://docs.rs/tokio-io-timeout/0.1")]
#![warn(missing_docs)]
extern crate bytes;
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_service;

use bytes::{Buf, BufMut};
use futures::{Async, Future, Poll};
use std::time::{Duration, Instant};
use std::io::{self, Read, Write};
use tokio_core::reactor::{Handle, Timeout};
use tokio_io::{AsyncRead, AsyncWrite};

struct TimeoutState {
    timeout: Option<Duration>,
    cur: Timeout,
    active: bool,
}

impl TimeoutState {
    fn new(handle: &Handle) -> io::Result<TimeoutState> {
        Ok(TimeoutState {
            timeout: None,
            cur: Timeout::new(Duration::from_secs(0), handle)?,
            active: false,
        })
    }

    #[inline]
    fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    #[inline]
    fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
        self.reset();
    }

    #[inline]
    fn reset(&mut self) {
        if self.active {
            self.active = false;
            self.cur.reset(Instant::now());
        }
    }

    #[inline]
    fn check(&mut self) -> io::Result<()> {
        let timeout = match self.timeout {
            Some(timeout) => timeout,
            None => return Ok(()),
        };

        if !self.active {
            self.cur.reset(Instant::now() + timeout);
            self.active = true;
        }

        if self.cur.poll()?.is_ready() {
            Err(io::Error::from(io::ErrorKind::TimedOut))
        } else {
            Ok(())
        }
    }
}

/// An `AsyncRead`er which applies a timeout to read operations.
pub struct TimeoutReader<R> {
    reader: R,
    state: TimeoutState,
}

impl<R> TimeoutReader<R>
where
    R: AsyncRead,
{
    /// Returns a new `TimeoutReader` wrapping the specified reader.
    ///
    /// There is initially no timeout.
    pub fn new(reader: R, handle: &Handle) -> io::Result<TimeoutReader<R>> {
        Ok(TimeoutReader {
            reader,
            state: TimeoutState::new(handle)?,
        })
    }

    /// Returns the current read timeout.
    pub fn timeout(&self) -> Option<Duration> {
        self.state.timeout()
    }

    /// Sets the read timeout.
    ///
    /// This will reset any pending timeout.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.state.set_timeout(timeout);
    }

    /// Returns a shared reference to the inner reader.
    pub fn get_ref(&self) -> &R {
        &self.reader
    }

    /// Returns a mutable reference to the inner reader.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    /// Consumes the `TimeoutReader`, returning the inner reader.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R> Read for TimeoutReader<R>
where
    R: AsyncRead,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let r = self.reader.read(buf);
        match r {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => self.state.check()?,
            _ => self.state.reset(),
        }
        r
    }
}

impl<R> AsyncRead for TimeoutReader<R>
where
    R: AsyncRead,
{
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.reader.prepare_uninitialized_buffer(buf)
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        let r = self.reader.read_buf(buf);
        match r {
            Ok(Async::NotReady) => self.state.check()?,
            _ => self.state.reset(),
        }
        r
    }
}

impl<R> Write for TimeoutReader<R>
where
    R: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.reader.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.reader.flush()
    }
}

impl<R> AsyncWrite for TimeoutReader<R>
where
    R: AsyncWrite,
{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.reader.shutdown()
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.reader.write_buf(buf)
    }
}

/// An `AsyncWrite`er which applies a timeout to write operations.
pub struct TimeoutWriter<W> {
    writer: W,
    state: TimeoutState,
}

impl<W> TimeoutWriter<W>
where
    W: AsyncWrite,
{
    /// Returns a new `TimeoutReader` wrapping the specified reader.
    ///
    /// There is initially no timeout.
    pub fn new(writer: W, handle: &Handle) -> io::Result<TimeoutWriter<W>> {
        Ok(TimeoutWriter {
            writer,
            state: TimeoutState::new(handle)?,
        })
    }

    /// Returns the current write timeout.
    pub fn timeout(&self) -> Option<Duration> {
        self.state.timeout()
    }

    /// Sets the write timeout.
    ///
    /// This will reset any pending timeout.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.state.set_timeout(timeout);
    }

    /// Returns a shared reference to the inner writer.
    pub fn get_ref(&self) -> &W {
        &self.writer
    }

    /// Returns a mutable reference to the inner writer.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    /// Consumes the `TimeoutWriter`, returning the inner writer.
    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W> Write for TimeoutWriter<W>
where
    W: AsyncWrite,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let r = self.writer.write(buf);
        match r {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => self.state.check()?,
            _ => self.state.reset(),
        }
        r
    }

    fn flush(&mut self) -> io::Result<()> {
        let r = self.writer.flush();
        match r {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => self.state.check()?,
            _ => self.state.reset(),
        }
        r
    }
}

impl<W> AsyncWrite for TimeoutWriter<W>
where
    W: AsyncWrite,
{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        let r = self.writer.shutdown();
        match r {
            Ok(Async::NotReady) => self.state.check()?,
            _ => self.state.reset(),
        }
        r
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        let r = self.writer.write_buf(buf);
        match r {
            Ok(Async::NotReady) => self.state.check()?,
            _ => self.state.reset(),
        }
        r
    }
}

impl<W> Read for TimeoutWriter<W>
where
    W: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.writer.read(buf)
    }
}

impl<W> AsyncRead for TimeoutWriter<W>
where
    W: AsyncRead,
{
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.writer.prepare_uninitialized_buffer(buf)
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.writer.read_buf(buf)
    }
}

/// A stream which applies read and write timeouts to an inner stream.
// TODO this stores two copies of the Handle which is maybe not great?
pub struct TimeoutStream<S>(TimeoutReader<TimeoutWriter<S>>);

impl<S> TimeoutStream<S>
where
    S: AsyncRead + AsyncWrite,
{
    /// Returns a new `TimeoutStream` wrapping the specified stream.
    ///
    /// There is initially no read or write timeout.
    pub fn new(stream: S, handle: &Handle) -> io::Result<TimeoutStream<S>> {
        let writer = TimeoutWriter::new(stream, handle)?;
        let reader = TimeoutReader::new(writer, handle)?;
        Ok(TimeoutStream(reader))
    }

    /// Returns the current read timeout.
    pub fn read_timeout(&self) -> Option<Duration> {
        self.0.timeout()
    }

    /// Sets the read timeout.
    ///
    /// This will reset any pending read timeout.
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.0.set_timeout(timeout)
    }

    /// Returns the current write timeout.
    pub fn write_timeout(&self) -> Option<Duration> {
        self.0.get_ref().timeout()
    }

    /// Sets the write timeout.
    ///
    /// This will reset any pending write timeout.
    pub fn set_write_timeout(&mut self, timeout: Option<Duration>) {
        self.0.get_mut().set_timeout(timeout)
    }

    /// Returns a shared reference to the inner stream.
    pub fn get_ref(&self) -> &S {
        self.0.get_ref().get_ref()
    }

    /// Returns a mutable reference to the inner stream.
    pub fn get_mut(&mut self) -> &mut S {
        self.0.get_mut().get_mut()
    }

    /// Consumes the stream, returning the inner stream.
    pub fn into_inner(self) -> S {
        self.0.into_inner().into_inner()
    }
}

impl<S> Read for TimeoutStream<S>
where
    S: AsyncRead + AsyncWrite,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<S> AsyncRead for TimeoutStream<S>
where
    S: AsyncRead + AsyncWrite,
{
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.0.prepare_uninitialized_buffer(buf)
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.0.read_buf(buf)
    }
}

impl<S> Write for TimeoutStream<S>
where
    S: AsyncRead + AsyncWrite,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<S> AsyncWrite for TimeoutStream<S>
where
    S: AsyncRead + AsyncWrite,
{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.0.shutdown()
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, io::Error> {
        self.0.write_buf(buf)
    }
}

#[cfg(test)]
mod test {
    use futures::Async;
    use tokio_core::reactor::Core;
    use tokio_core::net::TcpStream;
    use std::net::TcpListener;
    use std::io::Write;
    use std::thread;

    use super::*;

    struct DelayStream(Timeout);

    impl Read for DelayStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.poll()?.is_ready() {
                buf[0] = 0;
                Ok(1)
            } else {
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            }
        }
    }

    impl AsyncRead for DelayStream {
        unsafe fn prepare_uninitialized_buffer(&self, _: &mut [u8]) -> bool {
            true
        }
    }

    impl Write for DelayStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.0.poll()?.is_ready() {
                Ok(buf.len())
            } else {
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl AsyncWrite for DelayStream {
        fn shutdown(&mut self) -> Poll<(), io::Error> {
            Ok(Async::Ready(()))
        }
    }

    struct ReadFuture<S>(Option<S>);

    impl<S> Future for ReadFuture<S>
    where
        S: AsyncRead,
    {
        type Item = S;
        type Error = io::Error;

        fn poll(&mut self) -> Poll<S, io::Error> {
            let mut reader = self.0.take().unwrap();

            let mut buf = [0; 1];
            match reader.read(&mut buf) {
                Ok(_) => Ok(Async::Ready(reader)),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.0 = Some(reader);
                    Ok(Async::NotReady)
                }
                Err(e) => Err(e),
            }
        }
    }

    #[test]
    fn read_timeout() {
        let mut core = Core::new().unwrap();

        let reader = DelayStream(Timeout::new(Duration::from_millis(500), &core.handle()).unwrap());
        let mut reader = TimeoutReader::new(reader, &core.handle()).unwrap();
        reader.set_timeout(Some(Duration::from_millis(100)));

        let r = core.run(ReadFuture(Some(reader)));
        assert_eq!(r.err().unwrap().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn read_ok() {
        let mut core = Core::new().unwrap();

        let reader = DelayStream(Timeout::new(Duration::from_millis(100), &core.handle()).unwrap());
        let mut reader = TimeoutReader::new(reader, &core.handle()).unwrap();
        reader.set_timeout(Some(Duration::from_millis(500)));

        core.run(ReadFuture(Some(reader))).unwrap();
    }

    struct WriteFuture(TimeoutWriter<DelayStream>);

    impl Future for WriteFuture {
        type Item = ();
        type Error = io::Error;

        fn poll(&mut self) -> Poll<(), io::Error> {
            match self.0.write(&[0]) {
                Ok(_) => Ok(Async::Ready(())),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(Async::NotReady),
                Err(e) => Err(e),
            }
        }
    }

    #[test]
    fn write_timeout() {
        let mut core = Core::new().unwrap();

        let writer = DelayStream(Timeout::new(Duration::from_millis(500), &core.handle()).unwrap());
        let mut writer = TimeoutWriter::new(writer, &core.handle()).unwrap();
        writer.set_timeout(Some(Duration::from_millis(100)));

        let r = core.run(WriteFuture(writer));
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn write_ok() {
        let mut core = Core::new().unwrap();

        let writer = DelayStream(Timeout::new(Duration::from_millis(100), &core.handle()).unwrap());
        let mut writer = TimeoutWriter::new(writer, &core.handle()).unwrap();
        writer.set_timeout(Some(Duration::from_millis(500)));

        core.run(WriteFuture(writer)).unwrap();
    }

    #[test]
    fn tcp_read() {
        let mut core = Core::new().unwrap();
        let handle = core.handle();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        thread::spawn(move || {
            let mut socket = listener.accept().unwrap().0;
            thread::sleep(Duration::from_millis(10));
            socket.write_all(b"f").unwrap();
            thread::sleep(Duration::from_millis(500));
            let _ = socket.write_all(b"f"); // this may hit an eof
        });

        let f = TcpStream::connect(&addr, &handle).and_then(|s| {
            let mut s = TimeoutStream::new(s, &handle).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(100)));
            ReadFuture(Some(s))
        });
        let s = core.run(f).unwrap();

        let f = ReadFuture(Some(s));
        match core.run(f) {
            Ok(_) => panic!("unexpected success"),
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => panic!("{:?}", e),
        }
    }
}
