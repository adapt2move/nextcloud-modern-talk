//! Private normal-call transport split. Clones share the account reservation,
//! cancellation and a bounded host-only cookie owner. No credential or generic
//! authenticated request constructor crosses the public API boundary.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct SharedCookies {
    revision: u64,
    cookies: Vec<SessionCookie>,
}

pub(crate) struct MeetingLifetime {
    pub(crate) cancellation: Cancellation,
    account: Arc<AccountCharge>,
    _slot: OwnedSemaphorePermit,
    active: Arc<AtomicBool>,
}
impl MeetingLifetime {
    pub(crate) fn close(&self) {
        self.cancellation.cancel();
    }
    pub(crate) fn validate(&self, context: &OperationContext) -> Result<(), PortError> {
        if self.cancellation.is_cancelled() {
            return Err(PortError {
                kind: ErrorKind::Cancelled,
                account: Some(self.account.account),
                retry: Retry::Never,
            });
        }
        if context.scope.account != self.account.account {
            return Err(PortError {
                kind: ErrorKind::WrongScope,
                account: Some(self.account.account),
                retry: Retry::Never,
            });
        }
        Ok(())
    }
}
impl Drop for MeetingLifetime {
    fn drop(&mut self) {
        self.close();
        self.active.store(false, Ordering::Release);
    }
}

impl Transport {
    pub(crate) fn open_meeting(
        &mut self,
        context: &OperationContext,
    ) -> Result<(Transport, Transport, Arc<MeetingLifetime>), PortError> {
        self.validate(context)?;
        if self.meeting_active.swap(true, Ordering::AcqRel) {
            return Err(self.error(ErrorKind::Busy));
        }
        let slot = match self.runtime.0.meetings.clone().try_acquire_owned() {
            Ok(slot) => slot,
            Err(_) => {
                self.meeting_active.store(false, Ordering::Release);
                return Err(self.error(ErrorKind::Busy));
            }
        };
        let cookies = self
            .shared_cookies
            .get_or_insert_with(|| {
                Arc::new(Mutex::new(SharedCookies {
                    revision: 0,
                    cookies: self.cookies.clone(),
                }))
            })
            .clone();
        let lifetime = Arc::new(MeetingLifetime {
            cancellation: Cancellation::default(),
            account: self._account_charge.clone(),
            _slot: slot,
            active: self.meeting_active.clone(),
        });
        self.meeting_lifetime = Arc::downgrade(&lifetime);
        let fork = || Transport {
            binding: AuthenticatedBinding {
                scope: self.binding.scope,
                host: self.binding.host,
                origin: ServerOrigin(self.binding.origin.0.clone()),
                login_name: self.binding.login_name.clone(),
                user: self.binding.user.clone(),
                password: self.binding.password.clone(),
                cancellation: self.binding.cancellation.clone(),
            },
            client: self.client.clone(),
            runtime: self.runtime.clone(),
            _account_charge: self._account_charge.clone(),
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_loop: std::sync::Weak::new(),
            #[cfg(any(test, feature = "backend-test"))]
            diagnostic_cookies: duplex::CookieOrder::default(),
            cookies: Vec::new(),
            shared_cookies: Some(cookies.clone()),
            cancel_account_on_drop: false,
            meeting_active: self.meeting_active.clone(),
            meeting_lifetime: Arc::downgrade(&lifetime),
        };
        Ok((fork(), fork(), lifetime))
    }
    pub(crate) fn close_meeting(&self) {
        if let Some(lifetime) = self.meeting_lifetime.upgrade() {
            lifetime.close();
        }
    }
    pub(crate) fn meeting_active(&self) -> bool {
        self.meeting_active.load(Ordering::Acquire)
    }
    pub(super) fn begin_shared_cookies(
        &mut self,
    ) -> Result<Option<(u64, Vec<SessionCookie>)>, PortError> {
        let Some(shared) = &self.shared_cookies else {
            return Ok(None);
        };
        let state = shared
            .lock()
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        self.cookies = state.cookies.clone();
        Ok(Some((state.revision, state.cookies.clone())))
    }
    pub(super) fn commit_shared_cookies(
        &self,
        before: Option<(u64, Vec<SessionCookie>)>,
    ) -> Result<(), PortError> {
        let Some((revision, original)) = before else {
            return Ok(());
        };
        let Some(shared) = &self.shared_cookies else {
            return Err(self.error(ErrorKind::Uncertain));
        };
        let mut state = shared
            .lock()
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        if self.cookies == original {
            return Ok(());
        }
        if state.revision != revision {
            // Identical concurrent session cookies may converge. A different
            // session or expiry never overwrites a newer ordinary response.
            if self.cookies == state.cookies {
                return Ok(());
            }
            self.close_meeting();
            return Err(self.error(ErrorKind::Uncertain));
        }
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| self.error(ErrorKind::Uncertain))?;
        state.cookies = self.cookies.clone();
        Ok(())
    }
}
