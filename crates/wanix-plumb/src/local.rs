//! [`LocalPlumbPort`]: an in-process, single-node `#plumb` backend.
//!
//! This is the reference [`PlumbPort`]: a `publish` fans the message out to
//! every currently-subscribed stream on the same topic, in the same process. It
//! has no network and no durability, exactly matching the best-effort epidemic
//! contract — a subscription started after a publish never sees it. It backs the
//! device's tests and any single-node deployment where coordination stays local;
//! the cross-node form is `wanix-mesh`'s `GossipPlumbPort`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use wanix_fs::{FsError, FsResult};

use crate::port::{PlumbPort, PlumbStream};
use wanix_fs::LineBuffer;

/// An in-process `#plumb` backend that broadcasts to same-process subscribers.
#[derive(Default)]
pub struct LocalPlumbPort {
    topics: Mutex<BTreeMap<String, Vec<Weak<LineBuffer>>>>,
}

impl LocalPlumbPort {
    /// Creates an empty local port with no topics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, BTreeMap<String, Vec<Weak<LineBuffer>>>>> {
        self.topics
            .lock()
            .map_err(|_| FsError::Other("plumb local port lock poisoned".to_owned()))
    }

    /// Whether any subscription this port handed out is still live.
    ///
    /// A backend using a [`LocalPlumbPort`] as a per-topic fan-out (the gossip
    /// port does) uses this to decide when a topic is idle and may be evicted:
    /// once every `recv` stream it produced has dropped, the topic carries no
    /// local subscribers and its membership and pump task can be reclaimed.
    #[must_use]
    pub fn has_live_subscribers(&self) -> bool {
        let Ok(topics) = self.topics.lock() else {
            return false;
        };
        topics
            .values()
            .any(|subs| subs.iter().any(|weak| weak.strong_count() > 0))
    }
}

impl PlumbPort for LocalPlumbPort {
    fn publish(&self, topic: &str, line: &[u8]) -> FsResult<()> {
        let mut topics = self.lock()?;
        let Some(subscribers) = topics.get_mut(topic) else {
            // Best-effort: a publish to a topic with no live subscribers is a
            // no-op on the wire, never an error.
            return Ok(());
        };
        // Deliver to every live subscriber and prune any whose stream was dropped.
        subscribers.retain(|weak| match weak.upgrade() {
            Some(buffer) => {
                buffer.push(line);
                true
            }
            None => false,
        });
        // Evict the topic once its last subscriber has dropped, so a topic the
        // device touched once does not pin a map entry forever.
        if subscribers.is_empty() {
            topics.remove(topic);
        }
        Ok(())
    }

    fn subscribe(&self, topic: &str) -> FsResult<Box<dyn PlumbStream>> {
        // Bounded to a useful backlog of full `MAX_ENVELOPE_LEN` envelopes (16
        // messages): delivery is best-effort epidemic pub/sub, so a peer that
        // knows a topic name must not flood an idle subscriber into unbounded
        // growth — the oldest bytes drop, the same lossy semantics gossip
        // already has for a lagged subscriber.
        let buffer = Arc::new(LineBuffer::bounded(16 * crate::MAX_ENVELOPE_LEN));
        let mut topics = self.lock()?;
        // Sweep topics whose subscribers have all dropped before inserting, so
        // the map tracks only live subscriptions and cannot accumulate empty
        // entries as callers churn through topic names.
        topics.retain(|_, subscribers| {
            subscribers.retain(|weak| weak.strong_count() > 0);
            !subscribers.is_empty()
        });
        topics
            .entry(topic.to_owned())
            .or_default()
            .push(Arc::downgrade(&buffer));
        Ok(Box::new(LocalPlumbStream { buffer }))
    }
}

/// A [`LocalPlumbPort`] subscription: a blocking reader over a shared
/// [`LineBuffer`] the port pushes received lines into.
struct LocalPlumbStream {
    buffer: Arc<LineBuffer>,
}

impl PlumbStream for LocalPlumbStream {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.buffer.read(buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.buffer.read_ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_line(stream: &mut Box<dyn PlumbStream>) -> String {
        let mut buf = [0_u8; 256];
        let n = stream.read(&mut buf).unwrap();
        String::from_utf8(buf[..n].to_vec()).unwrap()
    }

    #[test]
    fn a_publish_fans_out_to_live_subscribers() {
        let port = LocalPlumbPort::new();
        let mut a = port.subscribe("build").unwrap();
        let mut b = port.subscribe("build").unwrap();
        port.publish("build", b"{\"kind\":\"x\"}\n").unwrap();
        assert_eq!(read_line(&mut a), "{\"kind\":\"x\"}\n");
        assert_eq!(read_line(&mut b), "{\"kind\":\"x\"}\n");
    }

    #[test]
    fn a_publish_with_no_subscribers_is_dropped() {
        let port = LocalPlumbPort::new();
        // No subscribers on this topic: best-effort drop, not an error.
        assert!(port.publish("empty", b"{\"kind\":\"x\"}\n").is_ok());
    }

    #[test]
    fn a_late_subscriber_misses_earlier_messages() {
        let port = LocalPlumbPort::new();
        port.publish("build", b"{\"kind\":\"early\"}\n").unwrap();
        let mut late = port.subscribe("build").unwrap();
        port.publish("build", b"{\"kind\":\"late\"}\n").unwrap();
        // The subscription only sees messages broadcast after it joined.
        assert_eq!(read_line(&mut late), "{\"kind\":\"late\"}\n");
    }

    #[test]
    fn dropped_subscriptions_do_not_accumulate_topics() {
        // A caller churning through many distinct topic names, dropping each
        // subscription, must not grow the topic map without bound: a later
        // subscribe sweeps the dead entries.
        let port = LocalPlumbPort::new();
        for i in 0..1000 {
            let stream = port.subscribe(&format!("topic-{i}")).unwrap();
            drop(stream);
        }
        // One more subscribe triggers the sweep; only its own live topic remains.
        let _live = port.subscribe("survivor").unwrap();
        let topic_count = port.topics.lock().unwrap().len();
        assert_eq!(
            topic_count, 1,
            "dead topics must be evicted; map held {topic_count} entries"
        );
    }

    #[test]
    fn a_publish_evicts_a_topic_whose_subscriber_dropped() {
        // Publishing to a topic whose only subscriber has dropped removes the
        // topic entry rather than leaving an empty subscriber list behind.
        let port = LocalPlumbPort::new();
        let stream = port.subscribe("gone").unwrap();
        drop(stream);
        port.publish("gone", b"{\"kind\":\"x\"}\n").unwrap();
        assert!(port.topics.lock().unwrap().is_empty());
    }
}
