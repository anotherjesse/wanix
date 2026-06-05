mod cursor;

pub(super) use cursor::PayloadCursor;

use super::{
    P9Attr, P9AttrBody, P9DirEntry, P9Error, P9Frame, P9FsStat, P9Lock, P9Qid, P9SetAttr, P9Version,
};

const P9_U16_FIELD_LEN: usize = 2;
const P9_U32_FIELD_LEN: usize = 4;
const P9_VERSION_PAYLOAD_PREFIX_LEN: usize = P9_U32_FIELD_LEN + P9_U16_FIELD_LEN;

pub(super) fn encode_version_payload(msize: u32, version: &str) -> Result<Vec<u8>, P9Error> {
    let version_bytes = version.as_bytes();
    let version_len = u16::try_from(version_bytes.len()).map_err(|_| P9Error::StringTooLong {
        len: version_bytes.len(),
    })?;
    let mut payload = Vec::with_capacity(P9_VERSION_PAYLOAD_PREFIX_LEN + version_bytes.len());
    payload.extend_from_slice(&msize.to_le_bytes());
    payload.extend_from_slice(&version_len.to_le_bytes());
    payload.extend_from_slice(version_bytes);
    Ok(payload)
}

pub(super) fn decode_version_frame(frame: &P9Frame, expected: u8) -> Result<P9Version, P9Error> {
    expect_message_type(frame, expected)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let msize = cursor.read_u32()?;
    let version = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9Version { msize, version })
}

pub(super) fn decode_qid_frame(frame: &P9Frame, expected: u8) -> Result<P9Qid, P9Error> {
    expect_message_type(frame, expected)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid = cursor.read_qid()?;
    cursor.finish()?;
    Ok(qid)
}

pub(super) fn decode_data_frame(frame: &P9Frame, expected: u8) -> Result<Vec<u8>, P9Error> {
    expect_message_type(frame, expected)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let data = cursor.read_counted_data()?;
    cursor.finish()?;
    Ok(data)
}

pub(super) fn expect_message_type(frame: &P9Frame, expected: u8) -> Result<(), P9Error> {
    if frame.message_type() == expected {
        Ok(())
    } else {
        Err(P9Error::UnexpectedMessageType {
            expected,
            actual: frame.message_type(),
        })
    }
}

pub(super) fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub(super) fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), P9Error> {
    let bytes = value.as_bytes();
    let len =
        u16::try_from(bytes.len()).map_err(|_| P9Error::StringTooLong { len: bytes.len() })?;
    push_u16(out, len);
    out.extend_from_slice(bytes);
    Ok(())
}

pub(super) fn push_qid(out: &mut Vec<u8>, qid: P9Qid) {
    out.push(qid.qid_type);
    push_u32(out, qid.version);
    push_u64(out, qid.path);
}

pub(super) fn push_fs_stat(out: &mut Vec<u8>, stat: P9FsStat) {
    push_u32(out, stat.fs_type);
    push_u32(out, stat.block_size);
    push_u64(out, stat.blocks);
    push_u64(out, stat.blocks_free);
    push_u64(out, stat.blocks_available);
    push_u64(out, stat.files);
    push_u64(out, stat.files_free);
    push_u64(out, stat.fsid);
    push_u32(out, stat.name_length);
}

pub(super) fn push_attr(out: &mut Vec<u8>, attr: &P9Attr) {
    push_u64(out, attr.valid);
    push_qid(out, attr.qid);
    push_attr_body(out, &P9AttrBody::from(attr));
}

pub(super) fn push_attr_body(out: &mut Vec<u8>, attr: &P9AttrBody) {
    push_u32(out, attr.mode);
    push_u32(out, attr.uid);
    push_u32(out, attr.gid);
    push_u64(out, attr.nlink);
    push_u64(out, attr.rdev);
    push_u64(out, attr.size);
    push_u64(out, attr.block_size);
    push_u64(out, attr.blocks);
    push_u64(out, attr.atime_seconds);
    push_u64(out, attr.atime_nanoseconds);
    push_u64(out, attr.mtime_seconds);
    push_u64(out, attr.mtime_nanoseconds);
    push_u64(out, attr.ctime_seconds);
    push_u64(out, attr.ctime_nanoseconds);
    push_u64(out, attr.btime_seconds);
    push_u64(out, attr.btime_nanoseconds);
    push_u64(out, attr.generation);
    push_u64(out, attr.data_version);
}

pub(super) fn push_set_attr(out: &mut Vec<u8>, attr: &P9SetAttr) {
    push_u32(out, attr.permissions);
    push_u32(out, attr.uid);
    push_u32(out, attr.gid);
    push_u64(out, attr.size);
    push_u64(out, attr.atime_seconds);
    push_u64(out, attr.atime_nanoseconds);
    push_u64(out, attr.mtime_seconds);
    push_u64(out, attr.mtime_nanoseconds);
}

pub(super) fn push_lock_range(out: &mut Vec<u8>, lock: &P9Lock) -> Result<(), P9Error> {
    push_u64(out, lock.start);
    push_u64(out, lock.length);
    push_u32(out, lock.proc_id);
    push_string(out, &lock.client_id)?;
    Ok(())
}

pub(super) fn push_dir_entry(out: &mut Vec<u8>, entry: &P9DirEntry) -> Result<(), P9Error> {
    push_qid(out, entry.qid);
    push_u64(out, entry.offset);
    out.push(entry.dirent_type);
    push_string(out, &entry.name)?;
    Ok(())
}

pub(super) fn push_counted_data(out: &mut Vec<u8>, data: &[u8]) -> Result<(), P9Error> {
    let len = u32::try_from(data.len()).map_err(|_| P9Error::DataTooLong { len: data.len() })?;
    push_u32(out, len);
    out.extend_from_slice(data);
    Ok(())
}
