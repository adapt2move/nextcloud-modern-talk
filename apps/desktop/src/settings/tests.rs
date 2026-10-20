use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use talk_core::bounded::Text;
use talk_core::preferences::{
    Appearance, Locale, LocalePreference, NotificationDelivery, NotificationPreferences, Palette,
    SidebarDensity,
    devices::{DeviceId, DeviceSelection},
    notifications::{NotificationCategory, NotificationEvent, SourceAvailability, plan},
};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "nmt-settings-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Applied;
impl StartupEffect for Applied {
    fn apply(&mut self, values: &LocalPreferences) -> StartupSnapshot {
        StartupSnapshot {
            registered: Some(values.launch_at_login),
            state: EffectState::Applied,
        }
    }
}
fn start(path: PathBuf) -> SettingsRuntime {
    SettingsRuntime::spawn(move || Ok((Box::new(storage::FileStore(path)), Box::new(Applied))))
        .unwrap()
}
fn wait(
    runtime: &SettingsRuntime,
    condition: impl Fn(&PreferenceSnapshot) -> bool,
) -> PreferenceSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = runtime.snapshot();
        if condition(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "state did not settle: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn ready(runtime: &SettingsRuntime) -> PreferenceSnapshot {
    wait(runtime, |s| {
        s.load != LoadState::Loading && s.startup.state != EffectState::Pending
    })
}
fn set(runtime: &SettingsRuntime, request: u64, values: LocalPreferences) -> Revision {
    let s = runtime.snapshot();
    runtime
        .submit(PreferenceCommand::Set {
            generation: s.generation,
            request: RequestId(request),
            expected_revision: s.revision,
            values,
        })
        .unwrap()
}
fn stop(runtime: &SettingsRuntime) {
    runtime.request_shutdown();
    wait(runtime, |s| s.runtime == RuntimeState::Stopped);
}

fn mixed_notifications() -> NotificationPreferences {
    NotificationPreferences {
        messages: NotificationDelivery {
            banners: false,
            sounds: true,
        },
        important: NotificationDelivery {
            banners: true,
            sounds: false,
        },
        incoming_calls: NotificationDelivery::OFF,
        calendar: NotificationDelivery {
            banners: true,
            sounds: true,
        },
    }
}

#[test]
fn file_restart_keeps_system_and_independent_sidebar_locale_devices() {
    let dir = Directory::new();
    let path = dir.0.join("preferences");
    let first = start(path.clone());
    assert_eq!(ready(&first).load, LoadState::DefaultsBecauseAbsent);
    let mut values = LocalPreferences {
        palette: Palette::Sage,
        sidebar_density: SidebarDensity::Compact,
        locale: LocalePreference::Spanish,
        notifications: mixed_notifications(),
        ..LocalPreferences::default()
    };
    values.devices.camera = DeviceSelection::Specific(DeviceId::new("camera-ä:2").unwrap());
    let revision = set(&first, 1, values.clone());
    wait(&first, |s| s.durable_revision == revision);
    stop(&first);
    let second = start(path);
    let loaded = ready(&second);
    assert_eq!(loaded.active, values);
    assert_eq!(loaded.active.appearance, Appearance::System);
    assert_eq!(loaded.load, LoadState::Loaded);
    assert_ne!(loaded.generation, first.snapshot().generation);
    stop(&second);
}

#[test]
fn preferences_survive_process_restart() {
    const MODE: &str = "NMT_SETTINGS_TEST_PROCESS_MODE";
    const PATH: &str = "NMT_SETTINGS_TEST_PROCESS_PATH";
    let values = LocalPreferences {
        palette: Palette::Sage,
        appearance: Appearance::System,
        sidebar_density: SidebarDensity::Compact,
        locale: LocalePreference::German,
        notifications: mixed_notifications(),
        ..LocalPreferences::default()
    };
    if let Ok(mode) = std::env::var(MODE) {
        let runtime = start(PathBuf::from(std::env::var_os(PATH).unwrap()));
        let loaded = ready(&runtime);
        match mode.as_str() {
            "save" => {
                assert_eq!(loaded.load, LoadState::DefaultsBecauseAbsent);
                let revision = set(&runtime, 1, values);
                wait(&runtime, |s| s.durable_revision == revision);
            }
            "load" => {
                assert_eq!(loaded.load, LoadState::Loaded);
                assert_eq!(loaded.active, values);
            }
            _ => panic!("unexpected private test mode"),
        }
        stop(&runtime);
        return;
    }
    let dir = Directory::new();
    for mode in ["save", "load"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "settings::tests::preferences_survive_process_restart",
                "--exact",
            ])
            .env(MODE, mode)
            .env(PATH, dir.0.join("preferences"))
            .output()
            .unwrap();
        assert!(output.status.success(), "child {mode}: {output:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "native negative probe; run with explicit nonexistent DBus and Pulse session addresses"]
fn notifications_without_services_report_failure_and_stop() {
    use talk_core::preferences::notifications::{DeliveryOutcome, NotificationState};
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .unwrap()
            .contains("nmt-settings-unavailable")
    );
    assert!(
        std::env::var("PULSE_SERVER")
            .unwrap()
            .contains("nmt-settings-unavailable")
    );
    let runtime = notifications::NotificationRuntime::start(Default::default()).unwrap();
    runtime
        .submit(NotificationEvent::test(
            RequestId(1),
            NotificationCategory::Messages,
            Text::new("Settings test").unwrap(),
            Text::new("Owned native notification failure probe").unwrap(),
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let NotificationState::Finished {
            request,
            banner,
            sound,
        } = runtime.snapshot()
        {
            assert_eq!(request, RequestId(1));
            assert!(matches!(banner, DeliveryOutcome::Failed(_)));
            assert!(matches!(sound, DeliveryOutcome::Failed(_)));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    runtime.request_shutdown();
    while runtime.snapshot() != NotificationState::Stopped {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct ControlledStore {
    file: storage::FileStore,
    entered: mpsc::SyncSender<LocalPreferences>,
    release: mpsc::Receiver<Result<(), PreferenceError>>,
}
impl PreferenceStore for ControlledStore {
    fn load(&mut self) -> Result<(LocalPreferences, LoadState), PreferenceError> {
        self.file.load()
    }
    fn save(&mut self, values: &LocalPreferences) -> Result<(), PreferenceError> {
        self.entered.send(values.clone()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap()?;
        self.file.save(values)
    }
}
fn controlled(
    path: PathBuf,
) -> (
    SettingsRuntime,
    mpsc::Receiver<LocalPreferences>,
    mpsc::SyncSender<Result<(), PreferenceError>>,
) {
    let (entered, writes) = mpsc::sync_channel(1);
    let (release, permits) = mpsc::sync_channel(1);
    let runtime = SettingsRuntime::spawn(move || {
        Ok((
            Box::new(ControlledStore {
                file: storage::FileStore(path),
                entered,
                release: permits,
            }),
            Box::new(Applied),
        ))
    })
    .unwrap();
    ready(&runtime);
    (runtime, writes, release)
}
#[test]
fn failed_save_restores_confirmed_and_explicit_retry_retains_only_draft() {
    let dir = Directory::new();
    let path = dir.0.join("prefs");
    let (runtime, entered, release) = controlled(path.clone());
    let values = LocalPreferences {
        palette: Palette::Sage,
        notifications: mixed_notifications(),
        ..LocalPreferences::default()
    };
    let revision = set(&runtime, 1, values.clone());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    release
        .send(Err(PreferenceError::PermissionDenied))
        .unwrap();
    let failure = wait(&runtime, |s| matches!(s.save, SaveState::Failed { .. }));
    assert_eq!(failure.active, LocalPreferences::default());
    assert!(!path.exists());
    assert_eq!(failure.durable_revision, Revision(0));
    let retried = runtime
        .submit(PreferenceCommand::Retry {
            generation: failure.generation,
            request: RequestId(2),
            failed_revision: revision,
        })
        .unwrap();
    assert_eq!(
        entered.recv_timeout(Duration::from_secs(5)).unwrap(),
        values
    );
    release.send(Ok(())).unwrap();
    assert_eq!(
        wait(&runtime, |s| s.durable_revision == retried).confirmed,
        values
    );
    stop(&runtime);
    let reopened = start(path);
    assert_eq!(ready(&reopened).active, values);
    stop(&reopened);
}
#[test]
fn one_latest_slot_supersedes_intermediate_edit_and_old_failure_does_not_rollback() {
    let dir = Directory::new();
    let (runtime, entered, release) = controlled(dir.0.join("prefs"));
    let first = LocalPreferences {
        palette: Palette::Sage,
        ..LocalPreferences::default()
    };
    set(&runtime, 1, first);
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    set(
        &runtime,
        2,
        LocalPreferences {
            appearance: Appearance::Dark,
            ..LocalPreferences::default()
        },
    );
    let newest = LocalPreferences {
        locale: LocalePreference::German,
        sidebar_density: SidebarDensity::Compact,
        ..LocalPreferences::default()
    };
    let revision = set(&runtime, 3, newest.clone());
    release
        .send(Err(PreferenceError::StorageUnavailable))
        .unwrap();
    assert_eq!(
        entered.recv_timeout(Duration::from_secs(5)).unwrap(),
        newest
    );
    let pending = runtime.snapshot();
    assert_eq!(pending.active, newest);
    assert!(matches!(pending.save, SaveState::Pending { revision: r, .. } if r == revision));
    release.send(Ok(())).unwrap();
    wait(&runtime, |s| s.durable_revision == revision);
    stop(&runtime);
    assert!(entered.try_recv().is_err());
}
#[test]
fn shutdown_drains_running_and_latest_and_rejects_new_edits() {
    let dir = Directory::new();
    let (runtime, entered, release) = controlled(dir.0.join("prefs"));
    set(&runtime, 1, LocalPreferences::default());
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let values = LocalPreferences {
        appearance: Appearance::Dark,
        ..LocalPreferences::default()
    };
    let revision = set(&runtime, 2, values.clone());
    runtime.request_shutdown();
    let s = runtime.snapshot();
    assert_eq!(s.runtime, RuntimeState::Draining);
    assert_eq!(
        runtime.submit(PreferenceCommand::Set {
            generation: s.generation,
            request: RequestId(3),
            expected_revision: s.revision,
            values: values.clone()
        }),
        Err(PreferenceError::Closed)
    );
    release.send(Ok(())).unwrap();
    assert_eq!(
        entered.recv_timeout(Duration::from_secs(5)).unwrap(),
        values
    );
    release.send(Ok(())).unwrap();
    let stopped = wait(&runtime, |s| s.runtime == RuntimeState::Stopped);
    assert_eq!(stopped.confirmed, values);
    assert_eq!(stopped.durable_revision, revision);
}
struct DelayedLoad {
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
    file: storage::FileStore,
}
impl PreferenceStore for DelayedLoad {
    fn load(&mut self) -> Result<(LocalPreferences, LoadState), PreferenceError> {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        self.file.load()
    }
    fn save(&mut self, values: &LocalPreferences) -> Result<(), PreferenceError> {
        self.file.save(values)
    }
}
#[test]
fn late_load_does_not_replace_newer_pending_choice() {
    let dir = Directory::new();
    let path = dir.0.join("prefs");
    storage::FileStore(path.clone())
        .save(&LocalPreferences {
            palette: Palette::Sage,
            ..LocalPreferences::default()
        })
        .unwrap();
    let (entered, receiver) = mpsc::sync_channel(1);
    let (sender, release) = mpsc::sync_channel(1);
    let runtime = SettingsRuntime::spawn(move || {
        Ok((
            Box::new(DelayedLoad {
                entered,
                release,
                file: storage::FileStore(path),
            }),
            Box::new(Applied),
        ))
    })
    .unwrap();
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let newest = LocalPreferences {
        appearance: Appearance::Dark,
        ..LocalPreferences::default()
    };
    let revision = set(&runtime, 1, newest.clone());
    sender.send(()).unwrap();
    assert_eq!(
        wait(&runtime, |s| s.durable_revision == revision).active,
        newest
    );
    stop(&runtime);
}
#[test]
fn codec_bounds_corruption_and_conservative_v1_v2_migration() {
    let values = LocalPreferences::default();
    for version in [1, 2] {
        for banners in [false, true] {
            for sounds in [false, true] {
                let sidebar = if version == 1 {
                    "comfortable"
                } else {
                    "relaxed"
                };
                let mut legacy = format!(
                    "nextcloud-modern-talk-preferences {version}\npalette=blue\nappearance=system\nscale=100\nsidebar={sidebar}\nlocale=system\nlogin=0\nbackground=0\nsounds={}\nbanners={}\n",
                    u8::from(sounds),
                    u8::from(banners)
                );
                let mut expected = values.clone();
                expected.launch_at_login = false;
                if version == 2 {
                    legacy.push_str(
                        "microphone=default\nspeaker=default\ncamera=id:63616d6572612d32\n",
                    );
                    expected.devices.camera =
                        DeviceSelection::Specific(DeviceId::new("camera-2").unwrap());
                }
                let old = NotificationDelivery { banners, sounds };
                expected.notifications = NotificationPreferences {
                    messages: old,
                    important: old,
                    incoming_calls: old,
                    calendar: NotificationDelivery::OFF,
                };
                assert_eq!(
                    storage::decode(legacy.as_bytes()).unwrap(),
                    (expected.clone(), LoadState::Migrated)
                );
                let v3 = storage::encode(&expected);
                assert!(v3.starts_with("nextcloud-modern-talk-preferences 6\n"));
                assert_eq!(
                    storage::decode(v3.as_bytes()).unwrap(),
                    (expected, LoadState::Loaded)
                );
            }
        }
    }
    let v3 = storage::encode(&values);
    assert!(storage::decode(format!("{v3}palette=blue\n").as_bytes()).is_err());
    assert!(storage::decode(v3.replace("calendar_sounds=0\n", "").as_bytes()).is_err());
    assert!(storage::decode(format!("{v3}sounds=0\n").as_bytes()).is_err());
    assert_eq!(
        storage::decode(v3.replace("preferences 6", "preferences 99").as_bytes()),
        Err(PreferenceError::UnsupportedVersion)
    );
    assert_eq!(
        storage::decode(&[b'x'; storage::MAX_FILE_BYTES + 1]),
        Err(PreferenceError::TooLarge)
    );
    let dir = Directory::new();
    let path = dir.0.join("prefs");
    fs::write(&path, "broken").unwrap();
    let runtime = start(path.clone());
    assert_eq!(
        ready(&runtime).load,
        LoadState::Failed(PreferenceError::InvalidData)
    );
    assert_eq!(fs::read(&path).unwrap(), b"broken");
    stop(&runtime);
}
#[test]
fn real_registration_add_remove_and_effect_failure_does_not_undo_disk_commit() {
    let dir = Directory::new();
    let registration = dir.0.join("autostart/app.desktop");
    let mut effect = platform::StartupRegistration {
        path: registration.clone(),
        executable: std::env::current_exe().unwrap(),
        format: platform::StartupFormat::DesktopEntry,
    };
    let on = LocalPreferences {
        launch_at_login: true,
        ..LocalPreferences::default()
    };
    assert_eq!(effect.apply(&on).state, EffectState::Applied);
    let text = fs::read_to_string(&registration).unwrap();
    assert!(text.contains("--autostart"));
    assert_eq!(
        effect
            .apply(&LocalPreferences {
                launch_at_login: false,
                ..LocalPreferences::default()
            })
            .registered,
        Some(false)
    );
    assert!(!registration.exists());
    fs::write(&registration, "foreign file").unwrap();
    let path = dir.0.join("prefs");
    let runtime =
        SettingsRuntime::spawn(move || Ok((Box::new(storage::FileStore(path)), Box::new(effect))))
            .unwrap();
    ready(&runtime);
    let rev = set(&runtime, 1, on.clone());
    let s = wait(&runtime, |s| s.durable_revision == rev);
    assert_eq!(s.confirmed, on);
    assert_eq!(s.save, SaveState::Idle);
    assert!(matches!(s.startup.state, EffectState::Failed(_)));
    fs::remove_file(registration).unwrap();
    runtime
        .submit(PreferenceCommand::RetryEffects {
            generation: s.generation,
            request: RequestId(2),
            expected_revision: rev,
        })
        .unwrap();
    wait(&runtime, |s| s.startup.state == EffectState::Applied);
    assert_eq!(runtime.snapshot().durable_revision, rev);
    stop(&runtime);
}
#[test]
fn system_language_fallback_and_notifications_apply_independent_policy() {
    assert_eq!(
        LocalePreference::System.resolve(&["fr-FR", "de-DE"]),
        Locale::German
    );
    assert_eq!(LocalePreference::System.resolve(&["xx"]), Locale::English);
    assert_eq!(LocalePreference::Spanish.resolve(&["de"]), Locale::Spanish);
    let event = NotificationEvent::test(
        RequestId(1),
        NotificationCategory::Messages,
        Text::new("Talk").unwrap(),
        Text::new("Synthetic event").unwrap(),
    );
    let mut prefs = talk_core::preferences::NotificationPreferences::default();
    assert!(plan(prefs, &event, SourceAvailability::Available).banner);
    prefs.messages.sounds = false;
    assert!(!plan(prefs, &event, SourceAvailability::Available).sound);
    assert!(plan(prefs, &event, SourceAvailability::Available).banner);
    let mut visible = event;
    visible.minimized = false;
    assert!(!plan(prefs, &visible, SourceAvailability::Available).banner);
    visible.eligible = false;
    prefs.messages.sounds = true;
    assert!(!plan(prefs, &visible, SourceAvailability::Available).sound);
}

#[test]
fn navigation_roundtrip_capacity_and_partial_page_never_evict() {
    use talk_core::preferences::{QuickSwitcherPreference, navigation::*};
    let account = NavigationAccountKey::new(&format!("a1:{:032x}:{:032x}", 1, 2)).unwrap();
    let other = NavigationAccountKey::new(&format!("a1:{:032x}:{:032x}", 1, 3)).unwrap();
    let support = RemoteNavigationSupport::Unsupported;
    let mut values = LocalPreferences {
        quick_switcher: QuickSwitcherPreference::Off,
        ..LocalPreferences::default()
    };
    let group = NavigationGroupId::new(9).unwrap();
    values
        .navigation
        .apply(
            account.clone(),
            support,
            NavigationEdit::UpsertGroup(NavigationGroup {
                id: group,
                name: Text::new("Team | Außendienst").unwrap(),
                order: 4,
                collapsed: true,
            }),
        )
        .unwrap();
    values
        .navigation
        .apply(
            account.clone(),
            support,
            NavigationEdit::SetSectionCollapsed {
                section: NavigationSection::Conferences,
                collapsed: true,
            },
        )
        .unwrap();
    for i in 0..100 {
        values
            .navigation
            .apply(
                account.clone(),
                support,
                NavigationEdit::PlaceRoom {
                    room: NavigationRoomKey::new(&format!("r1:{i:032x}")).unwrap(),
                    group: Some(group),
                    order: i,
                },
            )
            .unwrap();
    }
    let saved = values.clone();
    assert_eq!(
        values.navigation.apply(
            account.clone(),
            support,
            NavigationEdit::PlaceRoom {
                room: NavigationRoomKey::new(&format!("r1:{:032x}", 101)).unwrap(),
                group: None,
                order: 0
            }
        ),
        Err(NavigationError::Capacity)
    );
    assert_eq!(values, saved);
    let first = NavigationRoomKey::new(&format!("r1:{:032x}", 0)).unwrap();
    values
        .navigation
        .apply(
            account.clone(),
            support,
            NavigationEdit::SetFavorite {
                room: first.clone(),
                favorite: Some(true),
            },
        )
        .unwrap();
    assert_eq!(
        values
            .navigation
            .account(&account)
            .unwrap()
            .placements()
            .len(),
        100
    );
    assert_eq!(
        values.navigation.apply(
            other.clone(),
            RemoteNavigationSupport::Unknown,
            NavigationEdit::SetFavorite {
                room: first.clone(),
                favorite: Some(true)
            }
        ),
        Err(NavigationError::RemoteSupportUnknown)
    );
    assert!(values.navigation.account(&other).is_none());
    values
        .navigation
        .apply(
            other.clone(),
            support,
            NavigationEdit::PlaceRoom {
                room: first,
                group: None,
                order: 8,
            },
        )
        .unwrap();
    let encoded = storage::encode(&values);
    assert_eq!(
        storage::decode(encoded.as_bytes()).unwrap(),
        (values.clone(), LoadState::Loaded)
    );
    let repeated = encoded.clone()
        + encoded
            .lines()
            .find(|l| l.starts_with("layout_room="))
            .unwrap()
        + "\n";
    assert_eq!(
        storage::decode(repeated.as_bytes()),
        Err(PreferenceError::InvalidData)
    );
    values
        .navigation
        .apply(account.clone(), support, NavigationEdit::RemoveGroup(group))
        .unwrap();
    assert_eq!(
        values
            .navigation
            .account(&account)
            .unwrap()
            .placements()
            .len(),
        100
    );
    assert!(
        values
            .navigation
            .account(&account)
            .unwrap()
            .placements()
            .iter()
            .all(|p| p.group.is_none())
    );
    assert_eq!(
        values
            .navigation
            .account(&other)
            .unwrap()
            .placements()
            .len(),
        1
    );
    assert!(!QuickSwitcherPreference::Auto.effective(1));
    assert!(QuickSwitcherPreference::Auto.effective(2));
    assert!(!QuickSwitcherPreference::Off.effective(8));
    assert!(QuickSwitcherPreference::On.effective(1));
}

#[test]
fn schema_three_migrates_without_activating_quick_switch_or_losing_choices() {
    use talk_core::preferences::QuickSwitcherPreference;
    let values = LocalPreferences {
        notifications: mixed_notifications(),
        launch_at_login: true,
        ..LocalPreferences::default()
    };
    let old = storage::encode(&values)
        .replace("preferences 6", "preferences 3")
        .replace("quick_switcher=auto\n", "")
        .replace("update_channel=stable\n", "")
        .replace("onboarding_resume=none\n", "");
    let (migrated, state) = storage::decode(old.as_bytes()).unwrap();
    assert_eq!(state, LoadState::Migrated);
    assert_eq!(migrated, values);
    assert_eq!(migrated.quick_switcher, QuickSwitcherPreference::Auto);
}

#[test]
fn update_channel_persists_and_migrates_without_changing_other_preferences() {
    use talk_core::preferences::UpdateChannel;
    let mut values = LocalPreferences {
        launch_at_login: true,
        notifications: mixed_notifications(),
        ..LocalPreferences::default()
    };
    let old = storage::encode(&values)
        .replace("preferences 6", "preferences 4")
        .replace("update_channel=stable\n", "")
        .replace("onboarding_resume=none\n", "");
    assert_eq!(
        storage::decode(old.as_bytes()).unwrap(),
        (values.clone(), LoadState::Migrated)
    );
    values.update_channel = UpdateChannel::Prerelease;
    let dir = Directory::new();
    let mut file = storage::FileStore(dir.0.join("preferences"));
    file.save(&values).unwrap();
    assert_eq!(file.load().unwrap(), (values, LoadState::Loaded));
    assert!(
        storage::decode(
            format!(
                "{}update_channel=stable\n",
                storage::encode(&LocalPreferences::default())
            )
            .as_bytes()
        )
        .is_err()
    );
}

#[test]
fn restart_handoff_is_durable_bounded_and_old_autostart_false_is_preserved() {
    use talk_core::preferences::{OnboardingResume, navigation::NavigationAccountKey};
    let dir = Directory::new();
    let path = dir.0.join("preferences");
    let account = NavigationAccountKey::new(
        "a1:00000000000000000000000000000001:00000000000000000000000000000002",
    )
    .unwrap();
    let marker = OnboardingResume {
        account,
        expires_at_unix_seconds: 100_000,
    };
    assert!(marker.is_current(100_000));
    assert!(!marker.is_current(100_001));
    assert!(!marker.is_current(0));
    let mut values = LocalPreferences {
        launch_at_login: false,
        onboarding_resume: Some(marker.clone()),
        ..LocalPreferences::default()
    };
    assert!(LocalPreferences::default().launch_at_login);
    let mut store = storage::FileStore(path);
    store.save(&values).unwrap();
    assert_eq!(store.load().unwrap().0, values);
    values.onboarding_resume = None;
    store.save(&values).unwrap();
    assert_eq!(store.load().unwrap().0.onboarding_resume, None);
    assert!(!store.load().unwrap().0.launch_at_login);
    let previous = storage::encode(&values)
        .replace("preferences 6", "preferences 5")
        .replace("onboarding_resume=none\n", "");
    let (restored, state) = storage::decode(previous.as_bytes()).unwrap();
    assert_eq!(state, LoadState::Migrated);
    assert_eq!(restored, values);
    for invalid in [
        "none\nonboarding_resume=none",
        "12:not-a-key",
        "-1:a1:bad:bad",
    ] {
        assert!(
            storage::decode(
                storage::encode(&values)
                    .replace(
                        "onboarding_resume=none",
                        &format!("onboarding_resume={invalid}")
                    )
                    .as_bytes()
            )
            .is_err()
        );
    }
}
