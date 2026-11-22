//! Adapters between async HTTP bodies and the blocking readers operations use. A dropped reader stops
//! its producer; the upload cap counts bytes actually read.

use std::io::Read;

/// A response body fed from a blocking reader on its own thread.
pub fn reader_body(reader: Box<dyn Read + Send>) -> axum::body::Body {
    let _ = reader;
    todo!("stream::reader_body")
}

/// A request body read from a blocking thread. Reading past `cap` bytes fails.
pub struct BodyReader {
    _todo: (),
}

pub fn body_reader(body: axum::body::Body, cap: Option<u64>) -> BodyReader {
    let _ = (body, cap);
    todo!("stream::body_reader")
}

impl Read for BodyReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let _ = buf;
        todo!("BodyReader::read")
    }
}
