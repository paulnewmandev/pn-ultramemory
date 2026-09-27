// SPDX-License-Identifier: Apache-2.0
//! Newline-delimited framing with a bounded line length.
//!
//! # Role in the architecture
//! The stdio transport carries one JSON-RPC message per line. Reading a line with the standard
//! `read_line` would let a peer that never sends a newline grow the buffer without limit. This
//! module reads a line into a caller-owned buffer, refuses to hold more than a fixed number of
//! bytes, and after an oversized line skips to the next newline **without buffering the skipped
//! bytes**, so the stream stays usable.
//!
//! # Invariants
//! * The buffer never holds more than `limit` bytes.
//! * Bytes are consumed from the reader exactly up to and including the newline that ends the
//!   line, so the next call starts at the next line whatever happened to this one.
//! * `Interrupted` reads are retried; every other I/O error is returned unchanged.

use std::io::{self, BufRead, ErrorKind};

/// What one call to [`read_frame`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Frame {
    /// A complete line, without its terminator, is in the buffer. The last line of a stream that
    /// does not end in a newline is also reported this way.
    Line,
    /// The line was longer than the limit; it has been skipped and the buffer is empty.
    TooLong,
    /// The stream ended and no bytes were left.
    Eof,
}

/// Reads the next line into `buf`, holding at most `limit` bytes of it.
///
/// `buf` is cleared first. The line ending (`\n`) is consumed but not stored; a `\r` before it
/// is left in place for the caller to strip. A line of exactly `limit` bytes is accepted.
///
/// # Errors
/// Returns any I/O error of the reader other than [`ErrorKind::Interrupted`].
pub(crate) fn read_frame<R: BufRead + ?Sized>(
    reader: &mut R,
    limit: usize,
    buf: &mut Vec<u8>,
) -> io::Result<Frame> {
    buf.clear();
    let mut overflowed = false;
    let mut saw_bytes = false;
    loop {
        let chunk = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if chunk.is_empty() {
            return Ok(if overflowed {
                Frame::TooLong
            } else if saw_bytes {
                Frame::Line
            } else {
                Frame::Eof
            });
        }
        saw_bytes = true;
        let newline = chunk.iter().position(|&byte| byte == b'\n');
        let content = newline.unwrap_or(chunk.len());
        if !overflowed {
            if buf.len() + content > limit {
                overflowed = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&chunk[..content]);
            }
        }
        reader.consume(newline.map_or(content, |index| index + 1));
        if newline.is_some() {
            return Ok(if overflowed {
                Frame::TooLong
            } else {
                Frame::Line
            });
        }
    }
}

/// Unit tests for line splitting, the length bound and resynchronisation.
#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor, Read};

    use super::*;

    /// Reads every frame of `input` with the given limit and reader capacity.
    fn frames(input: &[u8], limit: usize, capacity: usize) -> Vec<(Frame, Vec<u8>)> {
        let mut reader = BufReader::with_capacity(capacity, Cursor::new(input.to_vec()));
        let mut buf = Vec::new();
        let mut out = Vec::new();
        loop {
            let frame = read_frame(&mut reader, limit, &mut buf).unwrap();
            if frame == Frame::Eof {
                return out;
            }
            out.push((frame, buf.clone()));
        }
    }

    /// Plain lines are split on newlines and the terminator is not part of the line.
    #[test]
    fn splits_lines() {
        let got = frames(b"one\ntwo\n\nthree\n", 100, 8192);
        let lines: Vec<&[u8]> = got.iter().map(|(_, line)| line.as_slice()).collect();
        assert_eq!(lines, [&b"one"[..], b"two", b"", b"three"]);
        assert!(got.iter().all(|(frame, _)| *frame == Frame::Line));
    }

    /// A final line without a newline is still delivered, then the stream ends.
    #[test]
    fn delivers_an_unterminated_last_line() {
        let got = frames(b"a\nlast", 100, 8192);
        assert_eq!(
            got,
            [
                (Frame::Line, b"a".to_vec()),
                (Frame::Line, b"last".to_vec())
            ]
        );
    }

    /// An empty stream is an immediate end.
    #[test]
    fn empty_input_is_eof() {
        assert!(frames(b"", 10, 8192).is_empty());
    }

    /// A carriage return is left for the caller, so `\r\n` lines stay recoverable.
    #[test]
    fn leaves_carriage_returns_in_place() {
        let got = frames(b"ab\r\n", 100, 8192);
        assert_eq!(got, [(Frame::Line, b"ab\r".to_vec())]);
    }

    /// A line of exactly the limit is accepted; one byte more is refused.
    #[test]
    fn enforces_the_limit_exactly() {
        let got = frames(b"12345\n123456\n12345\n", 5, 8192);
        assert_eq!(
            got,
            [
                (Frame::Line, b"12345".to_vec()),
                (Frame::TooLong, Vec::new()),
                (Frame::Line, b"12345".to_vec()),
            ]
        );
    }

    /// The bound and the resynchronisation hold for every reader capacity, including 1 byte.
    #[test]
    fn works_across_chunk_boundaries() {
        let input = b"ab\nabcdefghij\nxyz\n\nabcdefghijklmnop";
        let expected = [
            (Frame::Line, b"ab".to_vec()),
            (Frame::TooLong, Vec::new()),
            (Frame::Line, b"xyz".to_vec()),
            (Frame::Line, Vec::new()),
            (Frame::TooLong, Vec::new()),
        ];
        for capacity in 1..=20 {
            assert_eq!(frames(input, 4, capacity), expected, "capacity {capacity}");
        }
    }

    /// An oversized last line without a newline is still reported as too long.
    #[test]
    fn oversized_unterminated_line_is_too_long() {
        assert_eq!(frames(b"abcdefgh", 3, 8192), [(Frame::TooLong, Vec::new())]);
    }

    /// A limit of zero accepts only empty lines.
    #[test]
    fn zero_limit_accepts_only_empty_lines() {
        let got = frames(b"\nx\n\n", 0, 8192);
        assert_eq!(
            got,
            [
                (Frame::Line, Vec::new()),
                (Frame::TooLong, Vec::new()),
                (Frame::Line, Vec::new())
            ]
        );
    }

    /// The buffer capacity never grows past the limit plus one reader chunk, however long the line.
    #[test]
    fn does_not_buffer_an_oversized_line() {
        let mut input = vec![b'x'; 1_000_000];
        input.push(b'\n');
        input.extend_from_slice(b"ok\n");
        let mut reader = BufReader::with_capacity(4096, Cursor::new(input));
        let mut buf = Vec::new();
        assert_eq!(
            read_frame(&mut reader, 1024, &mut buf).unwrap(),
            Frame::TooLong
        );
        assert!(buf.capacity() <= 1024 + 4096, "capacity {}", buf.capacity());
        assert_eq!(
            read_frame(&mut reader, 1024, &mut buf).unwrap(),
            Frame::Line
        );
        assert_eq!(buf, b"ok");
    }

    /// A reader that fails once with `Interrupted` before every chunk, then delivers data.
    struct Flaky {
        /// Bytes still to deliver.
        data: Vec<u8>,
        /// Whether the next `fill_buf` must fail with `Interrupted`.
        interrupt: bool,
    }

    impl Read for Flaky {
        /// Copies from the remaining data.
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = out.len().min(self.data.len());
            out[..n].copy_from_slice(&self.data[..n]);
            self.data.drain(..n);
            Ok(n)
        }
    }

    impl BufRead for Flaky {
        /// Fails with `Interrupted` every other call.
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.interrupt {
                self.interrupt = false;
                return Err(io::Error::from(ErrorKind::Interrupted));
            }
            Ok(&self.data)
        }

        /// Drops consumed bytes and arms the next interruption.
        fn consume(&mut self, amount: usize) {
            self.data.drain(..amount);
            self.interrupt = true;
        }
    }

    /// Interrupted reads are retried transparently.
    #[test]
    fn retries_interrupted_reads() {
        let mut reader = Flaky {
            data: b"hello\nworld\n".to_vec(),
            interrupt: true,
        };
        let mut buf = Vec::new();
        assert_eq!(read_frame(&mut reader, 100, &mut buf).unwrap(), Frame::Line);
        assert_eq!(buf, b"hello");
        assert_eq!(read_frame(&mut reader, 100, &mut buf).unwrap(), Frame::Line);
        assert_eq!(buf, b"world");
        assert_eq!(read_frame(&mut reader, 100, &mut buf).unwrap(), Frame::Eof);
    }

    /// A reader that always fails.
    struct Broken;

    impl Read for Broken {
        /// Always fails.
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "gone"))
        }
    }

    impl BufRead for Broken {
        /// Always fails.
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "gone"))
        }

        /// Nothing to consume.
        fn consume(&mut self, _: usize) {}
    }

    /// Other I/O errors are returned unchanged.
    #[test]
    fn propagates_read_errors() {
        let mut buf = Vec::new();
        let error = read_frame(&mut Broken, 10, &mut buf).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    }
}
