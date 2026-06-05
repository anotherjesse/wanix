//! 9P wire protocol helpers.
//!
//! This module is intentionally below Wanix filesystem policy: callers can
//! split byte streams into tagged 9P frames and decode typed payloads before
//! the Rust port grows a full 9P server backed by Wanix namespaces.

mod attrs;
mod codec;
mod control;
mod frame;
mod io;
mod message;
mod metadata;
mod mutation;
mod node;
mod session;
mod types;
mod walk;

use codec::{
    PayloadCursor, decode_data_frame, expect_message_type, push_counted_data, push_dir_entry,
    push_u32, push_u64,
};

pub use self::io::*;
pub use attrs::*;
pub use control::*;
pub use frame::{P9Error, P9Frame, P9FrameBuffer, p9_declared_size, p9_tag_from_frame_bytes};
pub use message::*;
pub use metadata::*;
pub use mutation::*;
pub use node::*;
pub use session::*;
pub use types::*;
pub use walk::*;

const P9_U8_FIELD_LEN: usize = 1;
const P9_U16_FIELD_LEN: usize = 2;
const P9_U32_FIELD_LEN: usize = 4;
const P9_U64_FIELD_LEN: usize = 8;
const P9_QID_FIELD_LEN: usize = P9_U8_FIELD_LEN + P9_U32_FIELD_LEN + P9_U64_FIELD_LEN;
const P9_COUNTED_DATA_PREFIX_LEN: usize = P9_U32_FIELD_LEN;
const P9_TREADDIR_PAYLOAD_LEN: usize = P9_U32_FIELD_LEN + P9_U64_FIELD_LEN + P9_U32_FIELD_LEN;
const P9_DIRENTRY_FIXED_LEN: usize =
    P9_QID_FIELD_LEN + P9_U64_FIELD_LEN + P9_U8_FIELD_LEN + P9_U16_FIELD_LEN;

/// Builds a `Treaddir` frame.
#[must_use]
pub fn p9_treaddir(tag: u16, fid: u32, offset: u64, count: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_TREADDIR_PAYLOAD_LEN);
    push_u32(&mut payload, fid);
    push_u64(&mut payload, offset);
    push_u32(&mut payload, count);
    P9Frame::new(P9_TREADDIR, tag, payload)
}

/// Builds an `Rreaddir` frame.
///
/// # Errors
///
/// Returns an error when an entry name cannot fit in a 9P string field or the
/// encoded entry stream cannot fit in a 9P u32 count field.
pub fn p9_rreaddir(tag: u16, entries: &[P9DirEntry]) -> Result<P9Frame, P9Error> {
    let mut data = Vec::new();
    for entry in entries {
        push_dir_entry(&mut data, entry)?;
    }
    let mut payload = Vec::with_capacity(P9_COUNTED_DATA_PREFIX_LEN + data.len());
    push_counted_data(&mut payload, &data)?;
    Ok(P9Frame::new(P9_RREADDIR, tag, payload))
}

/// Returns the encoded byte length of one 9P2000.L directory entry record.
///
/// # Errors
///
/// Returns an error when the entry name cannot fit in a 9P string field.
pub fn p9_dir_entry_encoded_len(entry: &P9DirEntry) -> Result<usize, P9Error> {
    let name_len = u16::try_from(entry.name.len()).map_err(|_| P9Error::StringTooLong {
        len: entry.name.len(),
    })? as usize;
    Ok(P9_DIRENTRY_FIXED_LEN + name_len)
}

/// Builds a `Tclunk` frame.
#[must_use]
pub fn p9_tclunk(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U32_FIELD_LEN);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TCLUNK, tag, payload)
}

/// Builds an `Rclunk` frame.
#[must_use]
pub fn p9_rclunk(tag: u16) -> P9Frame {
    P9Frame::new(P9_RCLUNK, tag, Vec::new())
}

/// Decodes a `Treaddir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Treaddir` or the payload is
/// malformed.
pub fn p9_decode_treaddir(frame: &P9Frame) -> Result<P9ReadDir, P9Error> {
    expect_message_type(frame, P9_TREADDIR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let request = read_readdir_request(&mut cursor)?;
    cursor.finish()?;
    Ok(request)
}

fn read_readdir_request(cursor: &mut PayloadCursor<'_>) -> Result<P9ReadDir, P9Error> {
    let fid = cursor.read_u32()?;
    let offset = cursor.read_u64()?;
    let count = cursor.read_u32()?;
    Ok(P9ReadDir { fid, offset, count })
}

/// Decodes an `Rreaddir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rreaddir` or the payload is
/// malformed.
pub fn p9_decode_rreaddir(frame: &P9Frame) -> Result<Vec<P9DirEntry>, P9Error> {
    expect_message_type(frame, P9_RREADDIR)?;
    let data = decode_data_frame(frame, P9_RREADDIR)?;
    let mut cursor = PayloadCursor::new(&data);
    let mut entries = Vec::new();
    while cursor.remaining_len() > 0 {
        entries.push(cursor.read_dir_entry()?);
    }
    Ok(entries)
}

/// Decodes a `Tclunk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tclunk` or the payload is
/// malformed.
pub fn p9_decode_tclunk(frame: &P9Frame) -> Result<P9Clunk, P9Error> {
    expect_message_type(frame, P9_TCLUNK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Clunk { fid })
}

/// Decodes an `Rclunk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rclunk` or the payload is not
/// empty.
pub fn p9_decode_rclunk(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RCLUNK)?;
    PayloadCursor::new(frame.payload()).finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_is_declared() {
        assert!(!crate::CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn version_frame_round_trips_9p2000_l() {
        let frame = p9_tversion(P9_NOTAG, 131_072, P9_VERSION_9P2000_L).unwrap();

        assert_eq!(frame.message_type(), P9_TVERSION);
        assert_eq!(frame.tag(), P9_NOTAG);

        let bytes = frame.encode().unwrap();
        assert_eq!(&bytes[..4], &21_u32.to_le_bytes());
        assert_eq!(bytes[4], P9_TVERSION);
        assert_eq!(p9_tag_from_frame_bytes(&bytes).unwrap(), P9_NOTAG);

        let decoded = P9Frame::decode(&bytes).unwrap();
        assert_eq!(decoded, frame);
        assert_eq!(
            p9_decode_tversion(&decoded).unwrap(),
            P9Version {
                msize: 131_072,
                version: P9_VERSION_9P2000_L.to_owned()
            }
        );
    }

    #[test]
    fn rversion_uses_same_payload_shape_as_tversion() {
        let frame = p9_rversion(7, 65_536, "9P2000").unwrap();
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(
            p9_decode_rversion(&decoded).unwrap(),
            P9Version {
                msize: 65_536,
                version: "9P2000".to_owned()
            }
        );
        assert_eq!(
            p9_decode_tversion(&decoded).unwrap_err(),
            P9Error::UnexpectedMessageType {
                expected: P9_TVERSION,
                actual: P9_RVERSION
            }
        );
    }

    #[test]
    fn version_frame_round_trips_google_2_extension() {
        let frame = p9_tversion(P9_NOTAG, 131_072, P9_VERSION_9P2000_L_GOOGLE_2).unwrap();
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(
            p9_decode_tversion(&decoded).unwrap(),
            P9Version {
                msize: 131_072,
                version: P9_VERSION_9P2000_L_GOOGLE_2.to_owned()
            }
        );
    }

    #[test]
    fn frame_decode_accepts_unknown_message_types() {
        let frame = P9Frame::new(250, 9, vec![1, 2, 3]);
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(decoded.message_type(), 250);
        assert_eq!(decoded.tag(), 9);
        assert_eq!(decoded.payload(), &[1, 2, 3]);
        assert_eq!(p9_message_type_name(250), None);
    }

    #[test]
    fn frame_decode_rejects_invalid_sizes() {
        assert_eq!(
            P9Frame::decode(&[1, 2, 3]).unwrap_err(),
            P9Error::ShortFrame { len: 3 }
        );

        let mut too_small = Vec::new();
        too_small.extend_from_slice(&6_u32.to_le_bytes());
        too_small.extend_from_slice(&[P9_TVERSION, 1, 0]);
        assert_eq!(
            P9Frame::decode(&too_small).unwrap_err(),
            P9Error::InvalidFrameSize { size: 6 }
        );

        let mut mismatch = Vec::new();
        mismatch.extend_from_slice(&8_u32.to_le_bytes());
        mismatch.extend_from_slice(&[P9_TVERSION, 1, 0]);
        assert_eq!(
            P9Frame::decode(&mismatch).unwrap_err(),
            P9Error::FrameSizeMismatch {
                declared: 8,
                actual: 7
            }
        );
    }

    #[test]
    fn frame_buffer_splits_partial_and_multiple_frames() {
        let first = p9_tversion(1, 8192, P9_VERSION_9P2000_L)
            .unwrap()
            .encode()
            .unwrap();
        let second = p9_rversion(1, 8192, P9_VERSION_9P2000_L)
            .unwrap()
            .encode()
            .unwrap();
        let mut buffer = P9FrameBuffer::new();

        assert!(buffer.push(&first[..3]).unwrap().is_empty());
        assert_eq!(buffer.buffered_len(), 3);

        let frames = buffer.push(&first[3..]).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].message_type(), P9_TVERSION);
        assert_eq!(buffer.buffered_len(), 0);

        let mut combined = Vec::new();
        combined.extend_from_slice(&first);
        combined.extend_from_slice(&second);
        let frames = buffer.push(&combined).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].message_type(), P9_TVERSION);
        assert_eq!(frames[1].message_type(), P9_RVERSION);
    }

    #[test]
    fn version_payload_rejects_malformed_strings_and_trailing_bytes() {
        let truncated = P9Frame::new(P9_TVERSION, 1, vec![1, 0, 0, 0, 4, 0, b'9']);
        assert_eq!(
            p9_decode_tversion(&truncated).unwrap_err(),
            P9Error::UnexpectedEof {
                needed: 4,
                remaining: 1
            }
        );

        let invalid_utf8 = P9Frame::new(P9_TVERSION, 1, vec![1, 0, 0, 0, 1, 0, 0xff]);
        assert_eq!(
            p9_decode_tversion(&invalid_utf8).unwrap_err(),
            P9Error::InvalidUtf8
        );

        let trailing = P9Frame::new(P9_TVERSION, 1, vec![1, 0, 0, 0, 0, 0, 99]);
        assert_eq!(
            p9_decode_tversion(&trailing).unwrap_err(),
            P9Error::TrailingPayload { count: 1 }
        );
    }

    #[test]
    fn core_message_type_names_are_known() {
        assert_eq!(p9_message_type_name(P9_RLERROR), Some("Rlerror"));
        assert_eq!(p9_message_type_name(P9_TSTATFS), Some("Tstatfs"));
        assert_eq!(p9_message_type_name(P9_RSTATFS), Some("Rstatfs"));
        assert_eq!(p9_message_type_name(P9_TLOPEN), Some("Tlopen"));
        assert_eq!(p9_message_type_name(P9_RLOPEN), Some("Rlopen"));
        assert_eq!(p9_message_type_name(P9_TLCREATE), Some("Tlcreate"));
        assert_eq!(p9_message_type_name(P9_RLCREATE), Some("Rlcreate"));
        assert_eq!(p9_message_type_name(P9_TSYMLINK), Some("Tsymlink"));
        assert_eq!(p9_message_type_name(P9_RSYMLINK), Some("Rsymlink"));
        assert_eq!(p9_message_type_name(P9_TMKNOD), Some("Tmknod"));
        assert_eq!(p9_message_type_name(P9_RMKNOD), Some("Rmknod"));
        assert_eq!(p9_message_type_name(P9_TRENAME), Some("Trename"));
        assert_eq!(p9_message_type_name(P9_RRENAME), Some("Rrename"));
        assert_eq!(p9_message_type_name(P9_TREADLINK), Some("Treadlink"));
        assert_eq!(p9_message_type_name(P9_RREADLINK), Some("Rreadlink"));
        assert_eq!(p9_message_type_name(P9_TGETATTR), Some("Tgetattr"));
        assert_eq!(p9_message_type_name(P9_RGETATTR), Some("Rgetattr"));
        assert_eq!(p9_message_type_name(P9_TSETATTR), Some("Tsetattr"));
        assert_eq!(p9_message_type_name(P9_RSETATTR), Some("Rsetattr"));
        assert_eq!(p9_message_type_name(P9_TXATTRWALK), Some("Txattrwalk"));
        assert_eq!(p9_message_type_name(P9_RXATTRWALK), Some("Rxattrwalk"));
        assert_eq!(p9_message_type_name(P9_TXATTRCREATE), Some("Txattrcreate"));
        assert_eq!(p9_message_type_name(P9_RXATTRCREATE), Some("Rxattrcreate"));
        assert_eq!(p9_message_type_name(P9_TREADDIR), Some("Treaddir"));
        assert_eq!(p9_message_type_name(P9_RREADDIR), Some("Rreaddir"));
        assert_eq!(p9_message_type_name(P9_TFSYNC), Some("Tfsync"));
        assert_eq!(p9_message_type_name(P9_RFSYNC), Some("Rfsync"));
        assert_eq!(p9_message_type_name(P9_TLOCK), Some("Tlock"));
        assert_eq!(p9_message_type_name(P9_RLOCK), Some("Rlock"));
        assert_eq!(p9_message_type_name(P9_TGETLOCK), Some("Tgetlock"));
        assert_eq!(p9_message_type_name(P9_RGETLOCK), Some("Rgetlock"));
        assert_eq!(p9_message_type_name(P9_TLINK), Some("Tlink"));
        assert_eq!(p9_message_type_name(P9_RLINK), Some("Rlink"));
        assert_eq!(p9_message_type_name(P9_TMKDIR), Some("Tmkdir"));
        assert_eq!(p9_message_type_name(P9_RMKDIR), Some("Rmkdir"));
        assert_eq!(p9_message_type_name(P9_TRENAMEAT), Some("Trenameat"));
        assert_eq!(p9_message_type_name(P9_RRENAMEAT), Some("Rrenameat"));
        assert_eq!(p9_message_type_name(P9_TUNLINKAT), Some("Tunlinkat"));
        assert_eq!(p9_message_type_name(P9_RUNLINKAT), Some("Runlinkat"));
        assert_eq!(p9_message_type_name(P9_TVERSION), Some("Tversion"));
        assert_eq!(p9_message_type_name(P9_RVERSION), Some("Rversion"));
        assert_eq!(p9_message_type_name(P9_TAUTH), Some("Tauth"));
        assert_eq!(p9_message_type_name(P9_RAUTH), Some("Rauth"));
        assert_eq!(p9_message_type_name(P9_TATTACH), Some("Tattach"));
        assert_eq!(p9_message_type_name(P9_RATTACH), Some("Rattach"));
        assert_eq!(p9_message_type_name(P9_TFLUSH), Some("Tflush"));
        assert_eq!(p9_message_type_name(P9_RFLUSH), Some("Rflush"));
        assert_eq!(p9_message_type_name(P9_TWALK), Some("Twalk"));
        assert_eq!(p9_message_type_name(P9_RWALK), Some("Rwalk"));
        assert_eq!(p9_message_type_name(P9_TREAD), Some("Tread"));
        assert_eq!(p9_message_type_name(P9_RREAD), Some("Rread"));
        assert_eq!(p9_message_type_name(P9_TWRITE), Some("Twrite"));
        assert_eq!(p9_message_type_name(P9_RWRITE), Some("Rwrite"));
        assert_eq!(p9_message_type_name(P9_TCLUNK), Some("Tclunk"));
        assert_eq!(p9_message_type_name(P9_RCLUNK), Some("Rclunk"));
        assert_eq!(p9_message_type_name(P9_TREMOVE), Some("Tremove"));
        assert_eq!(p9_message_type_name(P9_RREMOVE), Some("Rremove"));
        assert_eq!(p9_message_type_name(P9_TFLUSHF), Some("Tflushf"));
        assert_eq!(p9_message_type_name(P9_RFLUSHF), Some("Rflushf"));
        assert_eq!(p9_message_type_name(P9_TWALKGETATTR), Some("Twalkgetattr"));
        assert_eq!(p9_message_type_name(P9_RWALKGETATTR), Some("Rwalkgetattr"));
        assert_eq!(p9_message_type_name(250), None);
    }

    #[test]
    fn lerror_round_trips_linux_errno() {
        let frame = p9_rlerror(3, 2);
        let bytes = frame.encode().unwrap();

        assert_eq!(&bytes[..4], &11_u32.to_le_bytes());
        assert_eq!(bytes[4], P9_RLERROR);
        assert_eq!(
            p9_decode_rlerror(&P9Frame::decode(&bytes).unwrap()).unwrap(),
            P9Lerror { ecode: 2 }
        );
    }

    #[test]
    fn statfs_round_trips_9p2000_l_payload() {
        let frame = p9_tstatfs(4, 10).encode().unwrap();
        assert_eq!(&frame[..4], &11_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TSTATFS);
        assert_eq!(
            p9_decode_tstatfs(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9StatFs { fid: 10 }
        );

        let stat = P9FsStat {
            fs_type: 0x0102_1997,
            block_size: 4096,
            blocks: 1,
            blocks_free: 2,
            blocks_available: 3,
            files: 4,
            files_free: 5,
            fsid: 6,
            name_length: 255,
        };
        let response = p9_rstatfs(4, stat).encode().unwrap();
        assert_eq!(&response[..4], &67_u32.to_le_bytes());
        assert_eq!(response[4], P9_RSTATFS);
        assert_eq!(
            p9_decode_rstatfs(&P9Frame::decode(&response).unwrap()).unwrap(),
            stat
        );
    }

    #[test]
    fn fsync_round_trips_empty_response() {
        let frame = p9_tfsync(4, 10).encode().unwrap();
        assert_eq!(&frame[..4], &11_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TFSYNC);
        assert_eq!(
            p9_decode_tfsync(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Fsync { fid: 10 }
        );

        let response = p9_rfsync(4).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RFSYNC);
        p9_decode_rfsync(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn lock_round_trips_status_response() {
        let lock = P9Lock {
            lock_type: P9_LOCK_TYPE_WRITE,
            start: 11,
            length: 22,
            proc_id: 33,
            client_id: "linux-node".to_owned(),
        };
        let frame = p9_tlock(5, 10, 1, &lock).unwrap().encode().unwrap();
        assert_eq!(&frame[..4], &48_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TLOCK);
        assert_eq!(
            p9_decode_tlock(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9LockRequest {
                fid: 10,
                flags: 1,
                lock
            }
        );

        let response = p9_rlock(5, P9_LOCK_STATUS_OK).encode().unwrap();
        assert_eq!(&response[..4], &8_u32.to_le_bytes());
        assert_eq!(response[4], P9_RLOCK);
        assert_eq!(
            p9_decode_rlock(&P9Frame::decode(&response).unwrap()).unwrap(),
            P9_LOCK_STATUS_OK
        );
    }

    #[test]
    fn getlock_round_trips_no_conflict_payload() {
        let request_lock = P9Lock {
            lock_type: P9_LOCK_TYPE_READ,
            start: 44,
            length: 55,
            proc_id: 66,
            client_id: "client-a".to_owned(),
        };
        let frame = p9_tgetlock(6, 11, &request_lock).unwrap().encode().unwrap();
        assert_eq!(&frame[..4], &42_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TGETLOCK);
        assert_eq!(
            p9_decode_tgetlock(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9GetLockRequest {
                fid: 11,
                lock: request_lock
            }
        );

        let response_lock = P9Lock {
            lock_type: P9_LOCK_TYPE_UNLOCK,
            start: 44,
            length: 55,
            proc_id: 0,
            client_id: String::new(),
        };
        let response = p9_rgetlock(6, &response_lock).unwrap().encode().unwrap();
        assert_eq!(&response[..4], &30_u32.to_le_bytes());
        assert_eq!(response[4], P9_RGETLOCK);
        assert_eq!(
            p9_decode_rgetlock(&P9Frame::decode(&response).unwrap()).unwrap(),
            response_lock
        );
    }

    #[test]
    fn auth_round_trips_9p2000_l_payload() {
        let frame = p9_tauth(4, 10, "root", "main", 1000).unwrap();
        let bytes = frame.encode().unwrap();

        assert_eq!(bytes[4], P9_TAUTH);
        assert_eq!(
            p9_decode_tauth(&P9Frame::decode(&bytes).unwrap()).unwrap(),
            P9Auth {
                afid: 10,
                uname: "root".to_owned(),
                aname: "main".to_owned(),
                n_uname: 1000
            }
        );

        let qid = qid(0, 0x0102_0304, 0x0506_0708_090a_0b0c);
        let response = p9_rauth(4, qid);
        assert_eq!(
            p9_decode_rauth(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            qid
        );
    }

    #[test]
    fn attach_round_trips_9p2000_l_payload() {
        let frame = p9_tattach(4, 10, P9_NOFID, "root", "", 1000).unwrap();
        let bytes = frame.encode().unwrap();

        assert_eq!(bytes[4], P9_TATTACH);
        assert_eq!(
            p9_decode_tattach(&P9Frame::decode(&bytes).unwrap()).unwrap(),
            P9Attach {
                fid: 10,
                afid: P9_NOFID,
                uname: "root".to_owned(),
                aname: String::new(),
                n_uname: 1000
            }
        );

        let qid = qid(0x80, 0x0102_0304, 0x0506_0708_090a_0b0c);
        let response = p9_rattach(4, qid);
        assert_eq!(
            p9_decode_rattach(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            qid
        );
    }

    #[test]
    fn flush_round_trips_oldtag_and_empty_response() {
        let frame = p9_tflush(6, 42).encode().unwrap();
        assert_eq!(&frame[..4], &9_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TFLUSH);
        assert_eq!(
            p9_decode_tflush(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Flush { oldtag: 42 }
        );

        let response = p9_rflush(6).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RFLUSH);
        p9_decode_rflush(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn flushf_round_trips_fid_and_empty_response() {
        let frame = p9_tflushf(7, 11).encode().unwrap();
        assert_eq!(&frame[..4], &11_u32.to_le_bytes());
        assert_eq!(frame[4], P9_TFLUSHF);
        assert_eq!(
            p9_decode_tflushf(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9FlushF { fid: 11 }
        );

        let response = p9_rflushf(7).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RFLUSHF);
        p9_decode_rflushf(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn walk_round_trips_names_and_qids() {
        let frame = p9_twalk(5, 10, 11, &["bin", "sh"]).unwrap();
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(
            p9_decode_twalk(&decoded).unwrap(),
            P9Walk {
                fid: 10,
                newfid: 11,
                names: vec!["bin".to_owned(), "sh".to_owned()]
            }
        );

        let first = qid(0x80, 0, 1);
        let second = qid(0, 0, 2);
        let response = p9_rwalk(5, &[first, second]).unwrap();
        assert_eq!(
            p9_decode_rwalk(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            vec![first, second]
        );
    }

    #[test]
    fn walkgetattr_round_trips_names_attrs_and_qids() {
        let frame = p9_twalkgetattr(5, 10, 11, &["bin", "sh"]).unwrap();
        let decoded = P9Frame::decode(&frame.encode().unwrap()).unwrap();

        assert_eq!(decoded.message_type(), P9_TWALKGETATTR);
        assert_eq!(
            p9_decode_twalkgetattr(&decoded).unwrap(),
            P9Walk {
                fid: 10,
                newfid: 11,
                names: vec!["bin".to_owned(), "sh".to_owned()]
            }
        );

        let first = qid(0x80, 0, 1);
        let second = qid(0, 0, 2);
        let attr = P9Attr {
            valid: 0x200,
            qid: second,
            mode: 0o100755,
            uid: 3,
            gid: 4,
            nlink: 5,
            rdev: 6,
            size: 7,
            block_size: 8,
            blocks: 9,
            atime_seconds: 10,
            atime_nanoseconds: 11,
            mtime_seconds: 12,
            mtime_nanoseconds: 13,
            ctime_seconds: 14,
            ctime_nanoseconds: 15,
            btime_seconds: 16,
            btime_nanoseconds: 17,
            generation: 18,
            data_version: 19,
        };
        let response =
            p9_rwalkgetattr(5, attr.valid, &P9AttrBody::from(&attr), &[first, second]).unwrap();
        assert_eq!(&response.encode().unwrap()[..4], &175_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rwalkgetattr(&P9Frame::decode(&response.encode().unwrap()).unwrap()).unwrap(),
            P9WalkGetAttrResponse {
                valid: attr.valid,
                attr: P9AttrBody::from(&attr),
                qids: vec![first, second]
            }
        );
    }

    #[test]
    fn lopen_round_trips_linux_flags_and_iounit() {
        let frame = p9_tlopen(6, 22, 0x8000).encode().unwrap();
        assert_eq!(
            p9_decode_tlopen(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Open {
                fid: 22,
                flags: 0x8000
            }
        );

        let qid = qid(0, 7, 9);
        let response = p9_rlopen(6, qid, 8192).encode().unwrap();
        assert_eq!(
            p9_decode_rlopen(&P9Frame::decode(&response).unwrap()).unwrap(),
            (qid, 8192)
        );
    }

    #[test]
    fn lcreate_round_trips_name_flags_mode_gid_and_iounit() {
        let frame = p9_tlcreate(12, 22, "new.txt", 0x241, 0o100664, 1000)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&frame[..4], &32_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tlcreate(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Create {
                fid: 22,
                name: "new.txt".to_owned(),
                flags: 0x241,
                mode: 0o100664,
                gid: 1000
            }
        );

        let qid = qid(0, 7, 9);
        let response = p9_rlcreate(12, qid, 8192).encode().unwrap();
        assert_eq!(&response[..4], &24_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rlcreate(&P9Frame::decode(&response).unwrap()).unwrap(),
            (qid, 8192)
        );
    }

    #[test]
    fn symlink_and_readlink_round_trip() {
        let symlink = p9_tsymlink(16, 1, "link.txt", "target.txt", 1000)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&symlink[..4], &37_u32.to_le_bytes());
        assert_eq!(symlink[4], P9_TSYMLINK);
        assert_eq!(
            p9_decode_tsymlink(&P9Frame::decode(&symlink).unwrap()).unwrap(),
            P9Symlink {
                dir_fid: 1,
                name: "link.txt".to_owned(),
                target: "target.txt".to_owned(),
                gid: 1000
            }
        );

        let qid = qid(0x02, 0, 44);
        let symlink_response = p9_rsymlink(16, qid).encode().unwrap();
        assert_eq!(&symlink_response[..4], &20_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rsymlink(&P9Frame::decode(&symlink_response).unwrap()).unwrap(),
            qid
        );

        let readlink = p9_treadlink(17, 2).encode().unwrap();
        assert_eq!(&readlink[..4], &11_u32.to_le_bytes());
        assert_eq!(
            p9_decode_treadlink(&P9Frame::decode(&readlink).unwrap()).unwrap(),
            P9ReadLink { fid: 2 }
        );

        let readlink_response = p9_rreadlink(17, "target.txt").unwrap().encode().unwrap();
        assert_eq!(&readlink_response[..4], &19_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rreadlink(&P9Frame::decode(&readlink_response).unwrap()).unwrap(),
            "target.txt"
        );
    }

    #[test]
    fn mknod_round_trips_special_file_payload() {
        let frame = p9_tmknod(18, 1, "tty0", 0o020620, 4, 0, 5)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(frame[4], P9_TMKNOD);
        assert_eq!(
            p9_decode_tmknod(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Mknod {
                dir_fid: 1,
                name: "tty0".to_owned(),
                mode: 0o020620,
                major: 4,
                minor: 0,
                gid: 5
            }
        );

        let qid = qid(0, 0, 44);
        let response = p9_rmknod(18, qid).encode().unwrap();
        assert_eq!(&response[..4], &20_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rmknod(&P9Frame::decode(&response).unwrap()).unwrap(),
            qid
        );
    }

    #[test]
    fn getattr_round_trips_fixed_9p2000_l_payload() {
        let frame = p9_tgetattr(11, 77, 0x1234_5678_90ab_cdef).encode().unwrap();
        assert_eq!(&frame[..4], &19_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tgetattr(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9GetAttr {
                fid: 77,
                request_mask: 0x1234_5678_90ab_cdef
            }
        );

        let attr = P9Attr {
            valid: 0x200,
            qid: qid(0x80, 1, 2),
            mode: 0o040755,
            uid: 3,
            gid: 4,
            nlink: 5,
            rdev: 6,
            size: 7,
            block_size: 8,
            blocks: 9,
            atime_seconds: 10,
            atime_nanoseconds: 11,
            mtime_seconds: 12,
            mtime_nanoseconds: 13,
            ctime_seconds: 14,
            ctime_nanoseconds: 15,
            btime_seconds: 16,
            btime_nanoseconds: 17,
            generation: 18,
            data_version: 19,
        };
        let response = p9_rgetattr(11, &attr).encode().unwrap();
        assert_eq!(&response[..4], &160_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rgetattr(&P9Frame::decode(&response).unwrap()).unwrap(),
            attr
        );
    }

    #[test]
    fn setattr_round_trips_fixed_9p2000_l_payload() {
        let attr = P9SetAttr {
            permissions: 0o600,
            uid: 1000,
            gid: 1001,
            size: 44,
            atime_seconds: 5,
            atime_nanoseconds: 6,
            mtime_seconds: 7,
            mtime_nanoseconds: 8,
        };
        let valid = P9_SETATTR_PERMISSIONS
            | P9_SETATTR_SIZE
            | P9_SETATTR_ATIME
            | P9_SETATTR_ATIME_NOT_SYSTEM_TIME
            | P9_SETATTR_MTIME
            | P9_SETATTR_MTIME_NOT_SYSTEM_TIME;
        let frame = p9_tsetattr(12, 77, valid, &attr).encode().unwrap();
        assert_eq!(&frame[..4], &67_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tsetattr(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9SetAttrRequest {
                fid: 77,
                valid,
                attr
            }
        );

        let response = p9_rsetattr(12).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        p9_decode_rsetattr(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn xattrwalk_and_xattrcreate_round_trip() {
        let walk = p9_txattrwalk(30, 10, 11, "user.foo")
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(walk[4], P9_TXATTRWALK);
        assert_eq!(
            p9_decode_txattrwalk(&P9Frame::decode(&walk).unwrap()).unwrap(),
            P9XattrWalk {
                fid: 10,
                newfid: 11,
                name: "user.foo".to_owned()
            }
        );

        let walk_response = p9_rxattrwalk(30, 12).encode().unwrap();
        assert_eq!(
            p9_decode_rxattrwalk(&P9Frame::decode(&walk_response).unwrap()).unwrap(),
            12
        );

        let create = p9_txattrcreate(31, 10, "user.foo", 12, 1)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(create[4], P9_TXATTRCREATE);
        assert_eq!(
            p9_decode_txattrcreate(&P9Frame::decode(&create).unwrap()).unwrap(),
            P9XattrCreate {
                fid: 10,
                name: "user.foo".to_owned(),
                attr_size: 12,
                flags: 1
            }
        );

        let create_response = p9_rxattrcreate(31).encode().unwrap();
        p9_decode_rxattrcreate(&P9Frame::decode(&create_response).unwrap()).unwrap();
    }

    #[test]
    fn readdir_round_trips_offsets_types_and_counted_stream() {
        let frame = p9_treaddir(10, 66, 2, 4096).encode().unwrap();
        assert_eq!(
            p9_decode_treaddir(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9ReadDir {
                fid: 66,
                offset: 2,
                count: 4096
            }
        );

        let entries = vec![
            P9DirEntry {
                qid: qid(0x80, 1, 2),
                offset: 1,
                dirent_type: 4,
                name: "bin".to_owned(),
            },
            P9DirEntry {
                qid: qid(0, 3, 4),
                offset: 2,
                dirent_type: 8,
                name: "hello.txt".to_owned(),
            },
        ];
        assert_eq!(p9_dir_entry_encoded_len(&entries[0]).unwrap(), 27);
        assert_eq!(p9_dir_entry_encoded_len(&entries[1]).unwrap(), 33);

        let response = p9_rreaddir(10, &entries).unwrap();
        let encoded = response.encode().unwrap();
        assert_eq!(&encoded[..4], &71_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rreaddir(&P9Frame::decode(&encoded).unwrap()).unwrap(),
            entries
        );
    }

    #[test]
    fn mkdir_renameat_and_unlinkat_round_trip() {
        let mkdir = p9_tmkdir(13, 1, "new-dir", 0o040755, 1000)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&mkdir[..4], &28_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tmkdir(&P9Frame::decode(&mkdir).unwrap()).unwrap(),
            P9Mkdir {
                dir_fid: 1,
                name: "new-dir".to_owned(),
                mode: 0o040755,
                gid: 1000
            }
        );
        let dir_qid = qid(0x80, 0, 44);
        let mkdir_response = p9_rmkdir(13, dir_qid).encode().unwrap();
        assert_eq!(&mkdir_response[..4], &20_u32.to_le_bytes());
        assert_eq!(
            p9_decode_rmkdir(&P9Frame::decode(&mkdir_response).unwrap()).unwrap(),
            dir_qid
        );

        let rename = p9_trenameat(14, 1, "old.txt", 2, "new.txt")
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&rename[..4], &33_u32.to_le_bytes());
        assert_eq!(
            p9_decode_trenameat(&P9Frame::decode(&rename).unwrap()).unwrap(),
            P9RenameAt {
                old_dir_fid: 1,
                old_name: "old.txt".to_owned(),
                new_dir_fid: 2,
                new_name: "new.txt".to_owned()
            }
        );
        let rename_response = p9_rrenameat(14).encode().unwrap();
        assert_eq!(&rename_response[..4], &7_u32.to_le_bytes());
        p9_decode_rrenameat(&P9Frame::decode(&rename_response).unwrap()).unwrap();

        let unlink = p9_tunlinkat(15, 2, "gone", 0x200)
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&unlink[..4], &21_u32.to_le_bytes());
        assert_eq!(
            p9_decode_tunlinkat(&P9Frame::decode(&unlink).unwrap()).unwrap(),
            P9UnlinkAt {
                dir_fid: 2,
                name: "gone".to_owned(),
                flags: 0x200
            }
        );
        let unlink_response = p9_runlinkat(15).encode().unwrap();
        assert_eq!(&unlink_response[..4], &7_u32.to_le_bytes());
        p9_decode_runlinkat(&P9Frame::decode(&unlink_response).unwrap()).unwrap();
    }

    #[test]
    fn legacy_rename_round_trips_fid_dir_fid_and_name() {
        let rename = p9_trename(20, 2, 1, "renamed.txt")
            .unwrap()
            .encode()
            .unwrap();
        assert_eq!(&rename[..4], &28_u32.to_le_bytes());
        assert_eq!(rename[4], P9_TRENAME);
        assert_eq!(
            p9_decode_trename(&P9Frame::decode(&rename).unwrap()).unwrap(),
            P9Rename {
                fid: 2,
                dir_fid: 1,
                name: "renamed.txt".to_owned()
            }
        );

        let response = p9_rrename(20).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RRENAME);
        p9_decode_rrename(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn legacy_remove_round_trips_fid_and_empty_response() {
        let remove = p9_tremove(122, 3).encode().unwrap();
        assert_eq!(&remove[..4], &11_u32.to_le_bytes());
        assert_eq!(remove[4], P9_TREMOVE);
        assert_eq!(
            p9_decode_tremove(&P9Frame::decode(&remove).unwrap()).unwrap(),
            P9Remove { fid: 3 }
        );

        let response = p9_rremove(122).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        assert_eq!(response[4], P9_RREMOVE);
        p9_decode_rremove(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn link_round_trips_target_fid_and_name() {
        let link = p9_tlink(70, 1, 2, "hard.txt").unwrap().encode().unwrap();
        assert_eq!(link[4], P9_TLINK);
        assert_eq!(
            p9_decode_tlink(&P9Frame::decode(&link).unwrap()).unwrap(),
            P9Link {
                dir_fid: 1,
                fid: 2,
                name: "hard.txt".to_owned()
            }
        );

        let response = p9_rlink(70).encode().unwrap();
        assert_eq!(&response[..4], &7_u32.to_le_bytes());
        p9_decode_rlink(&P9Frame::decode(&response).unwrap()).unwrap();
    }

    #[test]
    fn read_and_write_round_trip_offsets_counts_and_data() {
        let read = p9_tread(7, 33, 0x0102_0304_0506_0708, 4096)
            .encode()
            .unwrap();
        assert_eq!(
            p9_decode_tread(&P9Frame::decode(&read).unwrap()).unwrap(),
            P9Read {
                fid: 33,
                offset: 0x0102_0304_0506_0708,
                count: 4096
            }
        );

        let data = b"hello 9p";
        let read_response = p9_rread(7, data).unwrap().encode().unwrap();
        assert_eq!(
            p9_decode_rread(&P9Frame::decode(&read_response).unwrap()).unwrap(),
            data
        );

        let write = p9_twrite(8, 44, 99, data).unwrap().encode().unwrap();
        assert_eq!(
            p9_decode_twrite(&P9Frame::decode(&write).unwrap()).unwrap(),
            P9Write {
                fid: 44,
                offset: 99,
                data: data.to_vec()
            }
        );

        let write_response = p9_rwrite(8, data.len() as u32).encode().unwrap();
        assert_eq!(
            p9_decode_rwrite(&P9Frame::decode(&write_response).unwrap()).unwrap(),
            data.len() as u32
        );
    }

    #[test]
    fn clunk_round_trips_empty_response() {
        let frame = p9_tclunk(9, 55).encode().unwrap();
        assert_eq!(
            p9_decode_tclunk(&P9Frame::decode(&frame).unwrap()).unwrap(),
            P9Clunk { fid: 55 }
        );

        let response = p9_rclunk(9).encode().unwrap();
        assert_eq!(
            p9_decode_rclunk(&P9Frame::decode(&response).unwrap()).unwrap(),
            ()
        );
    }

    #[test]
    fn typed_decoders_reject_trailing_payloads() {
        let mut payload = Vec::from(p9_tlopen(1, 2, 3).payload());
        payload.push(99);
        let frame = P9Frame::new(P9_TLOPEN, 1, payload);

        assert_eq!(
            p9_decode_tlopen(&frame).unwrap_err(),
            P9Error::TrailingPayload { count: 1 }
        );
    }

    #[test]
    fn counted_data_decoders_reject_short_payloads() {
        let frame = P9Frame::new(P9_RREAD, 1, vec![4, 0, 0, 0, b'a']);

        assert_eq!(
            p9_decode_rread(&frame).unwrap_err(),
            P9Error::UnexpectedEof {
                needed: 4,
                remaining: 1
            }
        );
    }

    fn qid(qid_type: u8, version: u32, path: u64) -> P9Qid {
        P9Qid {
            qid_type,
            version,
            path,
        }
    }
}
