use std::collections::BTreeMap;
use std::sync::Arc;

use crate::channel::PipeChannel;

/// Device-level allocation state: a monotonic id counter and the live channels.
#[derive(Default)]
pub(crate) struct DeviceState {
    pub(crate) next_id: u64,
    pub(crate) channels: BTreeMap<String, Arc<PipeChannel>>,
}
