//! `Content-Length` framing for the LSP stdio transport.

use std::fmt;
use std::io::{self, BufRead, Write};

pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;

#[derive(Debug)]
pub enum FramingError {
    Io(io::Error),
    HeaderTooLarge,
    InvalidHeader(String),
    MissingContentLength,
    DuplicateContentLength,
    InvalidContentLength(String),
    MessageTooLarge { length: usize, maximum: usize },
    UnexpectedHeaderEof,
    UnexpectedBodyEof { expected: usize, received: usize },
}

impl fmt::Display for FramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::HeaderTooLarge => formatter.write_str("LSP header line exceeds the size limit"),
            Self::InvalidHeader(header) => write!(formatter, "invalid LSP header: {header}"),
            Self::MissingContentLength => formatter.write_str("missing Content-Length header"),
            Self::DuplicateContentLength => formatter.write_str("duplicate Content-Length header"),
            Self::InvalidContentLength(value) => {
                write!(formatter, "invalid Content-Length value: {value}")
            }
            Self::MessageTooLarge { length, maximum } => write!(
                formatter,
                "LSP message length {length} exceeds the {maximum}-byte limit"
            ),
            Self::UnexpectedHeaderEof => {
                formatter.write_str("unexpected EOF while reading LSP headers")
            }
            Self::UnexpectedBodyEof { expected, received } => write!(
                formatter,
                "unexpected EOF after {received} of {expected} LSP body bytes"
            ),
        }
    }
}

impl std::error::Error for FramingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for FramingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Reads one framed message. EOF before a new header is a clean end of stream.
///
/// # Errors
///
/// Returns typed errors for malformed headers, invalid or excessive lengths,
/// truncated frames, and underlying I/O failures.
pub fn read_message(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, FramingError> {
    let mut content_length = None;
    let mut read_any_header_bytes = false;
    loop {
        let mut line = Vec::new();
        let count = reader.read_until(b'\n', &mut line)?;
        if count == 0 {
            return if read_any_header_bytes {
                Err(FramingError::UnexpectedHeaderEof)
            } else {
                Ok(None)
            };
        }
        read_any_header_bytes = true;
        if line.len() > MAX_HEADER_LINE_BYTES {
            return Err(FramingError::HeaderTooLarge);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.is_empty() {
            break;
        }
        let header = std::str::from_utf8(&line)
            .map_err(|_| FramingError::InvalidHeader(String::from("non-UTF-8 header")))?;
        let Some((name, value)) = header.split_once(':') else {
            return Err(FramingError::InvalidHeader(header.to_owned()));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(FramingError::DuplicateContentLength);
            }
            let value = value.trim();
            let length = value
                .parse::<usize>()
                .map_err(|_| FramingError::InvalidContentLength(value.to_owned()))?;
            if length > MAX_MESSAGE_BYTES {
                return Err(FramingError::MessageTooLarge {
                    length,
                    maximum: MAX_MESSAGE_BYTES,
                });
            }
            content_length = Some(length);
        }
    }

    let length = content_length.ok_or(FramingError::MissingContentLength)?;
    let mut body = vec![0; length];
    let mut received = 0;
    while received < length {
        let count = reader.read(&mut body[received..])?;
        if count == 0 {
            return Err(FramingError::UnexpectedBodyEof {
                expected: length,
                received,
            });
        }
        received += count;
    }
    Ok(Some(body))
}

/// Writes one LSP message with canonical CRLF headers.
///
/// # Errors
///
/// Returns an underlying writer error.
pub fn write_message(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)
}

#[cfg(test)]
mod tests {
    use super::{FramingError, MAX_MESSAGE_BYTES, read_message, write_message};
    use std::io::{BufReader, Cursor};

    #[test]
    fn reads_multiple_frames_across_small_buffer_boundaries() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, br#"{"jsonrpc":"2.0","id":1}"#).expect("first frame");
        write_message(&mut bytes, br#"{"jsonrpc":"2.0","id":2}"#).expect("second frame");
        let mut reader = BufReader::with_capacity(1, Cursor::new(bytes));
        assert_eq!(
            read_message(&mut reader).expect("first read"),
            Some(br#"{"jsonrpc":"2.0","id":1}"#.to_vec())
        );
        assert_eq!(
            read_message(&mut reader).expect("second read"),
            Some(br#"{"jsonrpc":"2.0","id":2}"#.to_vec())
        );
        assert_eq!(read_message(&mut reader).expect("clean EOF"), None);
    }

    #[test]
    fn rejects_invalid_duplicate_and_excessive_lengths() {
        let mut invalid = Cursor::new(b"Content-Length: nope\r\n\r\n".as_slice());
        assert!(matches!(
            read_message(&mut invalid),
            Err(FramingError::InvalidContentLength(value)) if value == "nope"
        ));

        let mut duplicate =
            Cursor::new(b"Content-Length: 1\r\ncontent-length: 1\r\n\r\nx".as_slice());
        assert!(matches!(
            read_message(&mut duplicate),
            Err(FramingError::DuplicateContentLength)
        ));

        let oversized = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1);
        let mut oversized = Cursor::new(oversized.into_bytes());
        assert!(matches!(
            read_message(&mut oversized),
            Err(FramingError::MessageTooLarge { .. })
        ));
    }

    #[test]
    fn distinguishes_clean_eof_from_truncated_headers_and_bodies() {
        let mut empty = Cursor::new(Vec::<u8>::new());
        assert!(read_message(&mut empty).expect("clean EOF").is_none());

        let mut header = Cursor::new(b"Content-Length: 2\r\n".as_slice());
        assert!(matches!(
            read_message(&mut header),
            Err(FramingError::UnexpectedHeaderEof)
        ));

        let mut body = Cursor::new(b"Content-Length: 3\r\n\r\nab".as_slice());
        assert!(matches!(
            read_message(&mut body),
            Err(FramingError::UnexpectedBodyEof {
                expected: 3,
                received: 2
            })
        ));
    }
}
