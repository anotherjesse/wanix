use std::collections::VecDeque;

use wanix_fs::FsResult;

pub(crate) fn read_from_queue(queue: &mut VecDeque<u8>, buf: &mut [u8]) -> FsResult<usize> {
    let len = queue.len().min(buf.len());
    for slot in buf.iter_mut().take(len) {
        *slot = queue
            .pop_front()
            .expect("queue contains at least len bytes");
    }
    Ok(len)
}
