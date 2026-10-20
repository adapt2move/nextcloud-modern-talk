//! The main-thread restart transaction. The release worker owns files and
//! replacement; this owner seals calls and waits for application work to drain.
use super::preferences::Services;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use talk_adapters::updates::InstallReservation;
use talk_core::preferences::{
    LoadState, RuntimeState, SaveState,
    updates::{UpdateError, UpdateSnapshot, UpdateState},
};

#[derive(Default)]
pub(super) enum Install {
    #[default]
    Idle,
    Draining {
        reservation: Option<InstallReservation>,
        executable: PathBuf,
        since: Instant,
    },
    Applying {
        executable: PathBuf,
    },
    Launching,
    Failed {
        error: UpdateError,
        executable: Option<PathBuf>,
        prompted: bool,
    },
}

pub(super) enum Action {
    Relaunch(PathBuf),
    ExplainFailure {
        executable: Option<PathBuf>,
        uncertain: bool,
    },
}

impl Install {
    pub fn begin(&mut self, services: &Services, generation: u64) -> Result<(), UpdateError> {
        if !matches!(self, Self::Idle) {
            return Err(UpdateError::Busy);
        }
        let preferences = services.preferences.snapshot();
        if preferences.runtime != RuntimeState::Running || preferences.save != SaveState::Idle {
            return Err(UpdateError::Busy);
        }
        let updater = services.updates.as_ref().map_err(|error| *error)?;
        let reservation = updater.reserve_install(generation, || {
            if let Some(accounts) = &services.accounts {
                accounts.borrow_mut().seal_for_update()?;
            }
            Ok(())
        })?;
        let executable = match updater.relaunch_path(&reservation) {
            Ok(path) => path,
            Err(error) => {
                let _ = updater.abort_install(reservation);
                if let Some(accounts) = &services.accounts {
                    accounts.borrow_mut().abort_update_admission();
                }
                return Err(error);
            }
        };
        *self = Self::Draining {
            reservation: Some(reservation),
            executable,
            since: Instant::now(),
        };
        services.request_application_shutdown();
        Ok(())
    }

    pub fn poll(&mut self, services: &Services) -> Option<Action> {
        let updater = services.updates.as_ref().ok()?;
        match self {
            Self::Idle => {
                let snapshot = services.preferences.snapshot();
                if matches!(
                    snapshot.load,
                    LoadState::Loaded | LoadState::Migrated | LoadState::DefaultsBecauseAbsent
                ) {
                    // Never arm from provisional defaults or a failed preference
                    // load. A rejected edit must not switch the update channel.
                    let _ = updater.set_channel(snapshot.confirmed.update_channel);
                }
                None
            }
            Self::Draining {
                reservation,
                executable,
                since,
            } => {
                if services.application_stopped() {
                    let token = reservation.take()?;
                    let result = updater.begin_install(token);
                    *self = match result {
                        Ok(()) => Self::Applying {
                            executable: executable.clone(),
                        },
                        Err(error) => Self::Failed {
                            error,
                            executable: None,
                            prompted: false,
                        },
                    };
                } else if since.elapsed() > Duration::from_secs(30) {
                    if let Some(token) = reservation.take() {
                        let _ = updater.abort_install(token);
                    }
                    // Do not replace binaries while a native/account owner is
                    // still draining, and never kill those owners to meet a timer.
                    *self = Self::Failed {
                        error: UpdateError::Busy,
                        executable: None,
                        prompted: false,
                    };
                }
                None
            }
            Self::Applying { executable } => match updater.snapshot().state {
                UpdateState::Installed(_) => {
                    let action = Action::Relaunch(executable.clone());
                    *self = Self::Launching;
                    Some(action)
                }
                UpdateState::Failed(error) => {
                    *self = Self::Failed {
                        error,
                        executable: updater.relaunch_after_failure(),
                        prompted: false,
                    };
                    None
                }
                _ => None,
            },
            Self::Failed {
                executable,
                prompted,
                error,
            } if !*prompted => {
                *prompted = true;
                Some(Action::ExplainFailure {
                    executable: executable.clone(),
                    uncertain: *error == UpdateError::InstallUncertain,
                })
            }
            Self::Failed { .. } | Self::Launching => None,
        }
    }

    pub fn project(&self, snapshot: &mut UpdateSnapshot) {
        if let Self::Failed { error, .. } = self {
            snapshot.state = UpdateState::Failed(*error);
        }
    }

    pub fn relaunch_failed(&mut self) {
        *self = Self::Failed {
            error: UpdateError::Unavailable,
            executable: None,
            prompted: false,
        };
    }
}

/// This runs on the background executor only after installation or verified
/// rollback. Development/auto-start arguments must not leak into the new process.
pub(super) fn relaunch(path: PathBuf) -> Result<(), UpdateError> {
    use std::process::{Command, Stdio};
    if !path.is_absolute() {
        return Err(UpdateError::UnsupportedInstall);
    }
    let directory = path.parent().ok_or(UpdateError::UnsupportedInstall)?;
    let _child = Command::new(&path)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| UpdateError::Unavailable)?;
    Ok(())
}
