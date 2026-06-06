//! Fixed 9P2000.L message overheads used to size payloads against `msize`.
//!
//! A negotiated `msize` bounds the whole message, so the data-carrying body of
//! `Rread`, `Rreaddir`, and `Twrite` must leave room for the surrounding frame
//! header and fixed fields. Both the server (sizing replies) and a client
//! (sizing `Tread`/`Twrite`/`Treaddir` requests) need the same numbers, so they
//! live in the shared protocol crate.

/// Bytes consumed by an `Rread` frame before its data payload.
///
/// Layout: 4-byte size + 1-byte type + 2-byte tag + 4-byte count.
pub const RREAD_HEADER_LEN: u32 = 11;

/// Bytes consumed by an `Rreaddir` frame before its directory-entry payload.
///
/// Layout: 4-byte size + 1-byte type + 2-byte tag + 4-byte count.
pub const RREADDIR_HEADER_LEN: u32 = 11;

/// Bytes consumed by a `Twrite` frame before its data payload.
///
/// Layout: 4-byte size + 1-byte type + 2-byte tag + 4-byte fid + 8-byte offset +
/// 4-byte count. A client must keep `Twrite` data within
/// `msize - RWRITE_HEADER_LEN`.
pub const RWRITE_HEADER_LEN: u32 = 23;

/// Reported `iounit` reserve subtracted from `msize` on a successful open.
///
/// The server advertises `msize - RLOPEN_OVERHEAD` as the suggested per-request
/// transfer unit, leaving headroom for request and reply framing.
pub const RLOPEN_OVERHEAD: u32 = 24;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_lengths_match_wire_field_layouts() {
        // size(4) + type(1) + tag(2) + count(4)
        assert_eq!(RREAD_HEADER_LEN, 4 + 1 + 2 + 4);
        assert_eq!(RREADDIR_HEADER_LEN, 4 + 1 + 2 + 4);
        // size(4) + type(1) + tag(2) + fid(4) + offset(8) + count(4)
        assert_eq!(RWRITE_HEADER_LEN, 4 + 1 + 2 + 4 + 8 + 4);
    }
}
