use std::sync::{Arc, RwLock};

/// A channel for sending single state messages.
/// Only stores the latest message, overwriting any previous unread message.
pub struct SingleStateChannel;

impl SingleStateChannel {
    /// Creates a new single state channel and returns the sender and receiver.
    pub fn new<T>() -> (Sender<T>, Receiver<T>) {
        let inner = Arc::new(RwLock::new(None));
        let sender = Sender {
            inner: inner.clone(),
        };
        let receiver = Receiver { inner };
        (sender, receiver)
    }
}

/// The sending half of a single state channel.
pub struct Sender<T> {
    inner: Arc<RwLock<Option<T>>>,
}

impl<T> Sender<T> {
    /// Sends a message, overwriting any previous unread message.
    pub fn send(&self, msg: T) {
        loop {
            // println!("Sending message");
            match self.inner.write() {
                Ok(mut guard) => {
                    *guard = Some(msg);
                    break;
                }
                Err(_) => {
                    #[cfg(target_arch = "wasm32")]
                    std::hint::spin_loop();
                    #[cfg(not(target_arch = "wasm32"))]
                    std::thread::yield_now();
                }
            }
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender {
            inner: self.inner.clone(),
        }
    }
}

/// The receiving half of a single state channel.
pub struct Receiver<T> {
    inner: Arc<RwLock<Option<T>>>,
}

impl<T> Receiver<T> {
    /// Receives a message from the channel, yielding if no message is available.
    pub fn recv(&self) -> Option<T> {
        loop {
            // println!("Receiving message");
            match self.inner.write() {
                Ok(mut guard) => return guard.take(),
                Err(_) => {
                    println!("Failed to receive message");
                    #[cfg(target_arch = "wasm32")]
                    std::hint::spin_loop();
                    #[cfg(not(target_arch = "wasm32"))]
                    std::thread::yield_now();
                }
            }
        }
    }
}
