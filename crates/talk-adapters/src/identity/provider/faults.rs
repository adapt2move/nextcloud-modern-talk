//! Unit-test-only barriers and legacy provider compatibility fixtures.

#[cfg(target_os = "linux")]
pub fn legacy_create(
    item: talk_core::identity::QualifiedItem,
    password: &str,
) -> super::Result<()> {
    use keyring_core::api::CredentialStoreApi;
    let store = zbus_secret_service_keyring_store::Store::new().map_err(|_| {
        super::error::storage(
            item.provider.backend,
            talk_core::identity::StorageFailure::Other,
        )
    })?;
    let modifiers = std::collections::HashMap::from([("target", "default")]);
    let entry = store
        .build(
            "org.adapt2move.nextcloud-modern-talk",
            &super::key(item),
            Some(&modifiers),
        )
        .map_err(|_| {
            super::error::storage(
                item.provider.backend,
                talk_core::identity::StorageFailure::Other,
            )
        })?;
    entry
        .set_password(&super::encode(item, password)?)
        .map_err(|_| {
            super::error::storage(
                item.provider.backend,
                talk_core::identity::StorageFailure::Other,
            )
        })
}
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[derive(Default)]
pub struct Gate {
    pub entered: AtomicBool,
    pub completed: AtomicBool,
    released: Mutex<bool>,
    changed: Condvar,
}
impl Gate {
    pub fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
    fn wait(self: Arc<Self>) -> Completion {
        self.entered.store(true, Ordering::Release);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
        drop(released);
        Completion(self)
    }
}
pub struct Completion(Arc<Gate>);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.completed.store(true, Ordering::Release);
    }
}
#[derive(Default)]
pub struct Faults {
    pub not_written: AtomicBool,
    pub settled: std::sync::atomic::AtomicUsize,
    gates: Mutex<std::collections::VecDeque<Arc<Gate>>>,
    replies: Mutex<Option<Arc<ReplyGate>>>,
    #[cfg(target_os = "linux")]
    preflight: Mutex<Option<Arc<Gate>>>,
    suppression: Mutex<Option<Arc<Gate>>>,
    reads: Mutex<Option<Arc<Gate>>>,
}
#[derive(Default)]
pub struct ReplyGate {
    pub entered: AtomicBool,
    released: AtomicBool,
    changed: tokio::sync::Notify,
}
impl ReplyGate {
    pub fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.changed.notify_one();
    }
    async fn wait(&self) {
        self.entered.store(true, Ordering::Release);
        while !self.released.load(Ordering::Acquire) {
            self.changed.notified().await;
        }
    }
}
impl Faults {
    pub fn arm_read(&self) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        assert!(self.reads.lock().unwrap().replace(gate.clone()).is_none());
        gate
    }
    pub fn before_read(&self) -> Option<Completion> {
        self.reads.lock().unwrap().take().map(Gate::wait)
    }
    #[cfg(unix)]
    pub fn arm_suppression(&self) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        assert!(
            self.suppression
                .lock()
                .unwrap()
                .replace(gate.clone())
                .is_none()
        );
        gate
    }
    pub fn before_suppression(&self) -> Option<Completion> {
        let gate = self.suppression.lock().unwrap().take();
        gate.map(Gate::wait)
    }
    pub fn arm_reply(&self) -> Arc<ReplyGate> {
        let gate = Arc::new(ReplyGate::default());
        assert!(self.replies.lock().unwrap().replace(gate.clone()).is_none());
        gate
    }
    pub async fn after_reply(&self) {
        let gate = self.replies.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.wait().await;
        }
    }
    #[cfg(target_os = "linux")]
    pub fn arm_preflight(&self) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        assert!(
            self.preflight
                .lock()
                .unwrap()
                .replace(gate.clone())
                .is_none()
        );
        gate
    }
    #[cfg(target_os = "linux")]
    pub fn after_preflight(&self) -> Option<Completion> {
        let gate = self.preflight.lock().unwrap().take();
        gate.map(Gate::wait)
    }
    pub fn arm(&self) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        let mut gates = self.gates.lock().unwrap();
        assert!(gates.len() < 2);
        gates.push_back(gate.clone());
        gate
    }
    pub fn before_write(&self) -> Option<Completion> {
        let gate = self.gates.lock().unwrap().pop_front();
        gate.map(Gate::wait)
    }
}
