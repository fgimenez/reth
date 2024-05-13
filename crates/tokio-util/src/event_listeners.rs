use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::broadcast::{self, Sender};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{error, warn};

const DEFAULT_BROADCAST_CHANNEL_SIZE: usize = 1000;

/// A bounded broadcast channel for a task.
#[derive(Debug)]
pub struct EventListeners<T> {
    /// The sender part of the broadcast channel
    sender: Sender<T>,
    /// The number of subscribers, needed because the broadcast sender doesn't track this
    subscriber_count: AtomicUsize,
}

impl<T: Clone> Clone for EventListeners<T> {
    fn clone(&self) -> Self {
        EventListeners {
            sender: self.sender.clone(),
            subscriber_count: AtomicUsize::new(self.subscriber_count.load(Ordering::SeqCst)),
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Default for EventListeners<T> {
    fn default() -> Self {
        Self::new(DEFAULT_BROADCAST_CHANNEL_SIZE)
    }
}

impl<T: Clone + Send + Sync + 'static> EventListeners<T> {
    /// Creates a new `EventListeners`.
    pub fn new(broadcast_channel_size: usize) -> Self {
        let (sender, _) = broadcast::channel(broadcast_channel_size);
        Self { sender, subscriber_count: 0.into() }
    }

    /// Broadcast sender setter.
    pub fn set_sender(&mut self, sender: Sender<T>) {
        self.sender = sender;
    }

    /// Broadcasts an event to all listeners.
    pub fn notify(&self, event: T) {
        match self.sender.send(event) {
            Ok(listener_count) => {
                if listener_count == 0 {
                    warn!("notification of network event with 0 listeners");
                }
            }
            Err(_) => error!("channel closed"),
        };
    }

    /// Sender cloner.
    pub fn clone_sender(&self) -> Sender<T> {
        self.sender.clone()
    }

    /// Adds a new event listener and returns the associated receiver.
    pub fn new_listener(&self) -> BroadcastStream<T> {
        self.subscriber_count.fetch_add(1, Ordering::Relaxed);
        BroadcastStream::new(self.sender.subscribe())
    }
}
