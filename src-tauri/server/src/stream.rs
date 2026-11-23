//! Adapters between async HTTP bodies and the blocking readers operations use. A dropped reader stops
//! its producer; the upload cap counts bytes actually read.

use std::io::Read;

use axum::body::Bytes;
use http_body_util::BodyExt;
use http_body_util::channel::Channel;
use tokio::runtime::Handle;

const CHUNK: usize = 64 * 1024;

/// A response body fed from a blocking reader on its own thread. Call it inside the server's
/// runtime.
pub fn reader_body(reader: Box<dyn Read + Send>) -> axum::body::Body {
    let (mut sender, body) = Channel::<Bytes, std::io::Error>::new(4);
    let handle = Handle::current();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; CHUNK];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    // Fails once the client has gone; returning drops the reader, which stops its
                    // producer.
                    if handle.block_on(sender.send_data(Bytes::copy_from_slice(&buf[..n]))).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    sender.abort(e);
                    return;
                }
            }
        }
    });
    axum::body::Body::new(body)
}

/// A request body read from a blocking thread. Reading past `cap` bytes fails.
pub struct BodyReader {
    handle: Handle,
    body: axum::body::Body,
    pending: Bytes,
    read: u64,
    cap: Option<u64>,
}

/// Call it inside the server's runtime; read the result from a blocking thread.
pub fn body_reader(body: axum::body::Body, cap: Option<u64>) -> BodyReader {
    BodyReader { handle: Handle::current(), body, pending: Bytes::new(), read: 0, cap }
}

impl Read for BodyReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pending.is_empty() {
            match self.handle.block_on(self.body.frame()) {
                None => return Ok(0),
                Some(Err(e)) => return Err(std::io::Error::other(e)),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        self.pending = data;
                    }
                }
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending.split_to(n));
        self.read += n as u64;
        if self.cap.is_some_and(|cap| self.read > cap) {
            return Err(std::io::Error::other("The upload is larger than the server accepts."));
        }
        Ok(n)
    }
}
