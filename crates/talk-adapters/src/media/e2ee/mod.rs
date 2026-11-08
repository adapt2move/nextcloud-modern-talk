//! Call end-to-end encryption compatible with Nextcloud Talk's web client.
pub(crate) mod exchange;
pub(crate) mod frame;

use std::sync::{Arc, Mutex};

/// What one peer connection needs: the local sender key shared by the whole call
/// and the key of the participant it receives from.
#[derive(Clone)]
pub(crate) struct Keys {
    pub(crate) send: Arc<Mutex<frame::Encryptor>>,
    pub(crate) receive: Arc<Mutex<frame::Decryptor>>,
}
impl Keys {
    pub(crate) fn new() -> Self {
        Self {
            send: Arc::new(Mutex::new(frame::Encryptor::default())),
            receive: Arc::new(Mutex::new(frame::Decryptor::default())),
        }
    }
}
