//! Native lifetime/composition. No renderer owns network, credentials or media I/O.
mod accounts;
mod assets;
mod content_images;
mod files;
mod fixtures;
mod preferences;
mod updates;

use crate::presentation::{
    Desktop, InitialView, Overlay, input,
    probes::Probe,
    theme::{Appearance, Palette},
};
use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
use std::borrow::Cow;
use std::time::Duration;

struct Options {
    scenario: Option<String>,
    palette: Palette,
    appearance: Appearance,
    scale: u16,
    font_metrics: bool,
    autostart: bool,
    preserve_preferences: bool,
    locale: Option<talk_core::preferences::LocalePreference>,
}
fn options() -> Result<Option<Options>, &'static str> {
    let mut result = Options {
        scenario: None,
        palette: Palette::Blue,
        appearance: Appearance::System,
        scale: 100,
        font_metrics: false,
        autostart: false,
        preserve_preferences: false,
        locale: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--licenses" => {
                println!(
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                    include_str!("../../../LICENSE"),
                    include_str!("../NOTICE.md"),
                    include_str!("../assets/fonts/GoogleSans-LICENSE.txt"),
                    include_str!("../assets/fonts/GoogleSans-TRADEMARKS.txt"),
                    include_str!("../assets/GPUI-LICENSE-APACHE"),
                    include_str!("../assets/icons/LUCIDE-LICENSE"),
                    include_str!("../assets/sounds/Apache-2.0.txt"),
                    include_str!("../assets/sounds/CC-BY-4.0.txt"),
                    include_str!("../assets/sounds/CC-BY-SA-4.0.txt")
                );
                return Ok(None);
            }
            "--help" | "-h" => {
                println!(
                    "Talk native desktop (implementation preview)\nRun without arguments for onboarding. Saved accounts and browser login use the account owner.\n\nExplicit UI development only:\n  --ui-scenario NAME\n  --ui-theme blue|sage\n  --ui-appearance system|light|dark\n  --ui-scale 100|125|150|200\n\nScenarios: {}",
                    fixtures::SCENARIOS.join(", ")
                );
                return Ok(None);
            }
            "--ui-preserve-preferences" => result.preserve_preferences = true,
            "--autostart" => result.autostart = true,
            "--ui-locale" => {
                result.locale = Some(match args.next().as_deref() {
                    Some("en") => talk_core::preferences::LocalePreference::English,
                    Some("de") => talk_core::preferences::LocalePreference::German,
                    Some("es") => talk_core::preferences::LocalePreference::Spanish,
                    Some("system") => talk_core::preferences::LocalePreference::System,
                    _ => return Err("Locale must be system, en, de or es"),
                })
            }
            "--ui-font-metrics" => result.font_metrics = true,
            "--ui-scenario" => {
                let value = args.next().ok_or("Missing UI scenario")?;
                if !fixtures::SCENARIOS.contains(&value.as_str()) {
                    return Err("Unknown UI scenario; see --help");
                }
                result.scenario = Some(value);
            }
            "--ui-theme" => {
                result.palette = match args.next().as_deref() {
                    Some("blue") => Palette::Blue,
                    Some("sage") => Palette::Sage,
                    _ => return Err("UI theme must be blue or sage"),
                }
            }
            "--ui-appearance" => {
                result.appearance = match args.next().as_deref() {
                    Some("system") => Appearance::System,
                    Some("light") => Appearance::Light,
                    Some("dark") => Appearance::Dark,
                    _ => return Err("Appearance must be system, light or dark"),
                }
            }
            "--ui-scale" => {
                result.scale = match args.next().as_deref() {
                    Some("100") => 100,
                    Some("125") => 125,
                    Some("150") => 150,
                    Some("200") => 200,
                    _ => return Err("Scale must be 100, 125, 150 or 200"),
                }
            }
            _ => return Err("Unknown option; see --help"),
        }
    }
    Ok(Some(result))
}
pub fn run() {
    let options = match options() {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let services = match preferences::Services::start(options.scenario.is_none()) {
        Ok(services) => services,
        Err(error) => {
            eprintln!("Settings worker could not start: {error:?}");
            return;
        }
    };
    let application = Application::new().with_assets(assets::Assets);
    application.on_reopen(|cx| {
        for window in cx.windows() {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
        cx.activate(true);
    });
    application.run(move |cx: &mut App| {
        let quit_services = services.clone();
        cx.on_app_quit(move |_| {
            quit_services.request_shutdown();
            let services = quit_services.clone();
            async move {
                while !services.stopped() {
                    gpui::Timer::after(Duration::from_millis(20)).await;
                }
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            while services.preferences.snapshot().load == talk_core::preferences::LoadState::Loading
            {
                gpui::Timer::after(Duration::from_millis(20)).await;
            }
            // Explicit development options are still real preference writes, in
            // the harness's isolated computer directory. Ordinary launch never
            // overwrites saved choices with command-line defaults.
            if options.scenario.is_some() && !options.preserve_preferences {
                let snapshot = services.preferences.snapshot();
                let mut values = snapshot.active.clone();
                values.palette = options.palette;
                values.appearance = options.appearance;
                values.scale = match options.scale {
                    125 => talk_core::preferences::DisplayScale::Percent125,
                    150 => talk_core::preferences::DisplayScale::Percent150,
                    200 => talk_core::preferences::DisplayScale::Percent200,
                    _ => talk_core::preferences::DisplayScale::Percent100,
                };
                values.locale = options
                    .locale
                    .unwrap_or(talk_core::preferences::LocalePreference::German);
                let _ =
                    services
                        .preferences
                        .submit(talk_core::preferences::PreferenceCommand::Set {
                            generation: snapshot.generation,
                            request: talk_core::preferences::RequestId(1),
                            expected_revision: snapshot.revision,
                            values,
                        });
            }
            let _ = cx.update(|cx| open_desktop(options, services, cx));
        })
        .detach();
    });
}

fn open_desktop(options: Options, services: std::rc::Rc<preferences::Services>, cx: &mut App) {
    if let Err(error) = cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/GoogleSans-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/GoogleSans-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/GoogleSans-Bold.ttf")),
    ]) {
        eprintln!("Bundled Google Sans unavailable; using native font fallback: {error}");
    }
    input::bind_keys(cx);
    let saved = services.preferences.snapshot();
    let hidden = talk_core::preferences::starts_hidden(&saved.active, options.autostart);
    let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
    let development = options.scenario.is_some();
    let overlay = match options.scenario.as_deref() {
        Some("settings") => Some(Overlay::Settings),
        Some("details") => Some(Overlay::RoomInfo),
        Some("participants") => Some(Overlay::Participants),
        Some("files") => Some(Overlay::Files),
        Some("thread") => Some(Overlay::Thread),
        Some("search") => Some(Overlay::Search),
        Some("create34" | "create35") => Some(Overlay::Create),
        Some("discover") => Some(Overlay::Discover),
        Some("accounts" | "focus-accounts") => Some(Overlay::Accounts),
        _ => None,
    };
    let prejoin = matches!(
        options.scenario.as_deref(),
        Some("prejoin" | "prejoin-voice")
    );
    let probe = match options.scenario.as_deref() {
        Some("controls") => Probe::Controls,
        Some("focus-accounts") => Probe::Accounts,
        Some("focus-rooms" | "focus-files") => Probe::Focus,
        Some("chat-motion") => Probe::Messages,
        _ => Probe::None,
    };
    let fixture_images = if services.accounts.is_none() {
        fixtures::content_images(options.scenario.as_deref())
    } else {
        Vec::new()
    };
    let port: Box<dyn talk_core::ui::presentation::NativeDesktopPort> =
        if let Some(accounts) = &services.accounts {
            Box::new(accounts::Port(accounts.clone()))
        } else {
            Box::new(fixtures::DevelopmentPort::new(options.scenario.as_deref()))
        };
    let bridge = Box::new(preferences::Bridge {
        notification_worker_request: None,
        update_last: None,
        services: services.clone(),
        cursor: 0,
        device_cursor: 0,
        permission_cursor: 0,
        notification_request: None,
        notification_result: talk_core::preferences::notifications::NotificationState::Idle,
    });
    let window=cx.open_window(WindowOptions{
            window_bounds:Some(WindowBounds::Windowed(bounds)),
            // A minimized native window remains recoverable from the taskbar.
            show:true,focus:!hidden,
            window_min_size:Some(size(px(720.),px(580.))),
            titlebar:Some(TitlebarOptions{title:Some(if development{"Talk — FICTITIOUS DATA · UI development"}else{"Talk"}.into()),appears_transparent:false,..Default::default()}),
            ..Default::default()
        },move|window,cx|{
        if options.font_metrics {
            for weight in [gpui::FontWeight::NORMAL,gpui::FontWeight::MEDIUM,gpui::FontWeight::BOLD] {
                let mut font=gpui::font("Google Sans");font.weight=weight;
                let id=cx.text_system().resolve_font(&font);
                let sample="Produktteam — Grüße 09:41";
                let run=gpui::TextRun{len:sample.len(),font,color:gpui::rgb(0x26354b).into(),background_color:None,underline:None,strikethrough:None};
                let line=window.text_system().shape_line(sample.into(),px(14.),&[run],None);
                println!("NATIVE_FONT weight={weight:?} id={id:?} ascent={:?} descent={:?} width={:?}",cx.text_system().ascent(id,px(14.)),cx.text_system().descent(id,px(14.)),line.width);
            }
        }
            cx.new(|cx|Desktop::new(port,InitialView{probe,development,palette:saved.active.palette,appearance:saved.active.appearance,overlay,prejoin,onboarding:options.scenario.as_deref()==Some("ready"),scale:saved.active.scale.percent()},bridge,window,cx))
        });
    match window {
        Ok(window) => {
            if hidden {
                let _ = window.update(cx, |_, window, _| window.minimize_window());
            }
            let view = window
                .update(cx, |_, _, cx| cx.entity().downgrade())
                .expect("new native window");
            // Sample the coalesced slot at the provider's cadence while Devices is open.
            // Otherwise use 10 Hz; this never dispatches capture or I/O directly.
            let preview_services = services.clone();
            cx.spawn(async move |cx| {
                // Only explicit disconnected development scenarios supply these
                // authored rasters. The same bounded decoder runs off the UI thread.
                for job in fixture_images {
                    let account=job.account;
                    let key=job.key;
                    if let Some((image,_))=cx.background_executor().spawn(async move {content_images::decode(&job)}).await {
                        let _=view.update(cx,|view,cx| {
                            view.refresh(cx);
                            view.set_content_images(Some(account),vec![(key,gpui::ImageSource::Render(image))],cx);
                        });
                    }
                }
                let mut dialogs = files::Dialogs::new();
                let mut surface = None;
                let mut sequence = 0;
                let mut interval = Duration::from_millis(100);
                let mut was_background = true;
                loop {
                    gpui::Timer::after(interval).await;
                    if let Some(accounts) = &preview_services.accounts {
                        let _ = cx.update(|cx| dialogs.poll(accounts, cx));
                    }
                    if let Some(accounts) = &preview_services.accounts {
                        let job=accounts.borrow_mut().take_content_image();
                        if let Some(job)=job {
                            let copy=job.clone();
                            let decoded=cx.background_executor().spawn(async move { content_images::decode(&copy) }).await;
                            let accepted=accounts.borrow_mut().finish_content_image(&job,decoded.as_ref().map(|(_,bytes)|*bytes));
                            if accepted && let Some((image,_))=decoded {
                                let _=view.update(cx,|view,cx| {
                                    view.refresh(cx);
                                    view.set_content_images(Some(job.account),vec![(job.key,gpui::ImageSource::Render(image))],cx);
                                });
                            }
                        }
                        if let Some(link)=accounts.borrow_mut().take_cloud_link() {
                            let _=cx.update(|cx|cx.open_url(link.as_str()));
                        }
                    }
                    let (requested, devices_open, background) =
                        match window.update(cx, |view, window, cx| {
                            view.refresh(cx);
                            view.observe_message_visibility(window);
                            (
                                view.preview_surface(),
                                view.devices_open(),
                                !window.is_window_active(),
                            )
                        }) {
                            Ok(surface) => surface,
                            Err(_) => break,
                        };
                    preview_services.poll_notifications(background);
                    if was_background && !background {
                        preview_services.preferences.refresh_native_interaction();
                    } else if !was_background && background {
                        preview_services.preferences.cancel_emoji_picker();
                    }
                    was_background = background;
                    let update_action = preview_services.install.borrow_mut().poll(&preview_services);
                    if let Some(action) = update_action {
                        let relaunch = match action {
                            updates::Action::Relaunch(path) => Some(path),
                            updates::Action::ExplainFailure { executable, uncertain } => {
                                let preferences = preview_services.preferences.snapshot();
                                let locale = match preferences.active.locale {
                                    talk_core::preferences::LocalePreference::System => preferences.observed_system_locale,
                                    value => value.resolve(&[]),
                                };
                                let message = crate::presentation::localization::text(locale, "Das Update konnte nicht abgeschlossen werden.");
                                let detail = crate::presentation::localization::text(locale, if uncertain {
                                    "Der Paketmanager hat die Installation nicht bestätigt. Beende die App und prüfe die Installation, bevor du sie erneut öffnest."
                                } else if executable.is_some() {
                                    "Die vorherige Version wurde geprüft und kann erneut gestartet werden."
                                } else {
                                    "Beende die App und öffne sie erneut, bevor du weiterarbeitest."
                                });
                                let restart = crate::presentation::localization::text(locale, "Vorherige Version starten");
                                let quit = crate::presentation::localization::text(locale, "App beenden");
                                let answers = if executable.is_some() {
                                    vec![gpui::PromptButton::ok(restart), gpui::PromptButton::cancel(quit)]
                                } else {
                                    vec![gpui::PromptButton::ok(quit)]
                                };
                                let prompt = window.update(cx, |_, window, cx| window.prompt(
                                    gpui::PromptLevel::Critical, message, Some(detail), &answers, cx));
                                match prompt {
                                    Ok(prompt) => match prompt.await {
                                        Ok(0) if executable.is_some() => executable,
                                        Ok(_) => {
                                            let _ = cx.update(|cx| cx.quit());
                                            break;
                                        }
                                        Err(_) => None,
                                    },
                                    Err(_) => None,
                                }
                            }
                        };
                        if let Some(executable) = relaunch {
                            let result = cx.background_executor().spawn(async move {
                                updates::relaunch(executable)
                            }).await;
                            if result.is_ok() {
                                let _ = cx.update(|cx| cx.quit());
                                break;
                            }
                            preview_services.install.borrow_mut().relaunch_failed();
                        }
                    }
                    interval = Duration::from_millis(if devices_open { 33 } else { 100 });
                    if requested != surface {
                        surface = requested;
                        sequence = 0;
                    }
                    let capture = preview_services.media.capture();
                    let call_scope = capture.as_ref().and_then(|c| c.scope());
                    let mut frames = Vec::with_capacity(16);
                    for _ in 0..16 {
                        if let Some(frame) = capture.as_ref().and_then(|c| c.take_video()) {
                            frames.push(frame);
                        } else {
                            break;
                        }
                    }
                    let images = if frames.is_empty() {
                        Vec::new()
                    } else {
                        cx.background_executor()
                            .spawn(async move {
                                frames
                                    .into_iter()
                                    .filter_map(|frame| {
                                        let participant = frame.participant;
                                        preferences::call_image(frame)
                                            .map(|image| (participant, image))
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .await
                    };
                    let current = preview_services.media.capture().and_then(|c| c.scope());
                    let _ = view.update(cx, |view, cx| {
                        if current == call_scope {
                            view.set_call_images(
                                current,
                                images
                                    .into_iter()
                                    .map(|(id, image)| (id, gpui::ImageSource::Render(image)))
                                    .collect(),
                                cx,
                            );
                        }
                    });
                    if let Some(surface) = surface
                        && let Ok(devices) = &preview_services.devices
                        && let Some(frame) = devices.latest_frame(surface, sequence)
                    {
                        sequence = frame.sequence();
                        let image = cx
                            .background_executor()
                            .spawn(async move { preferences::camera_image(&frame) })
                            .await;
                        let _ = view.update(cx, |view, cx| {
                            if view.preview_surface() == Some(surface) {
                                view.set_camera_image(image.map(gpui::ImageSource::Render), cx);
                            }
                        });
                    }
                }
            })
            .detach();
        }
        Err(error) => {
            eprintln!("Native window could not open: {error}");
            cx.quit();
        }
    }
    cx.on_window_closed(move |cx| {
        if cx.windows().is_empty() {
            services.request_shutdown();
            let services = services.clone();
            cx.spawn(async move |cx| {
                while !services.stopped() {
                    gpui::Timer::after(Duration::from_millis(20)).await;
                }
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        }
    })
    .detach();
    if !hidden {
        cx.activate(true);
    }
}
