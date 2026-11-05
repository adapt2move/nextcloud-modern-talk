//! The single macOS window can be hidden without retiring its service owners.
use super::preferences::Services;
use crate::presentation::Desktop;
use crate::presentation::localization::text;
use gpui::{App, KeyBinding, Menu, MenuItem, Window, actions};
use std::rc::Rc;
use std::time::Duration;
use talk_core::preferences::{Locale, LocalePreference};

actions!(application, [CloseWindow, Quit]);

pub(super) fn install(services: Rc<Services>, cx: &mut App) -> Option<Locale> {
    let mut locale = None;
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        let close_services = services.clone();
        cx.on_action(move |_: &CloseWindow, cx| {
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, cx| hide_window(&close_services, window, cx));
            }
        });
        let quit_services = services.clone();
        cx.on_action(move |_: &Quit, cx| request_quit(quit_services.clone(), cx));
        refresh_menus(&services, &mut locale, cx);
    }
    locale
}

pub(super) fn retain_on_close(services: Rc<Services>, window: &Window, cx: &App) {
    if cfg!(target_os = "macos") {
        window.on_window_should_close(cx, move |window, cx| {
            // Keep the root view and its account/notification pump alive. Dock
            // activation unhides this same window, preserving drafts and bounds.
            hide_window(&services, window, cx);
            false
        });
    }
}

fn hide_window(services: &Services, window: &mut Window, cx: &mut App) {
    if let Some(accounts) = &services.accounts {
        // Closing is required teardown, independent of UI queue backpressure.
        // Its generation fence also rejects an original that finishes later.
        accounts.borrow_mut().close_content_preview();
    }
    if let Some(view) = window.root::<Desktop>().flatten() {
        view.update(cx, |view, cx| view.prepare_window_hide(window, cx));
    }
    cx.hide();
}

pub(super) fn refresh_menus(services: &Services, previous: &mut Option<Locale>, cx: &mut App) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let preferences = services.preferences.snapshot();
    let locale = match preferences.active.locale {
        LocalePreference::System => preferences.observed_system_locale,
        selected => selected.resolve(&[]),
    };
    if *previous == Some(locale) {
        return;
    }
    cx.set_menus(vec![
        Menu {
            name: "Talk".into(),
            items: vec![MenuItem::action(text(locale, "App beenden"), Quit)],
        },
        Menu {
            name: text(locale, "Datei").to_owned().into(),
            items: vec![MenuItem::action(
                text(locale, "Fenster schließen"),
                CloseWindow,
            )],
        },
    ]);
    *previous = Some(locale);
}

pub(super) fn request_quit(services: Rc<Services>, cx: &mut App) {
    if services.shutdown_requested() {
        return;
    }
    services.request_shutdown();
    // Drain before asking GPUI to terminate: its platform quit callback has a
    // shorter timeout than the service owners' existing five-second deadline.
    cx.spawn(async move |cx| {
        while services.waiting_for_shutdown() {
            gpui::Timer::after(Duration::from_millis(20)).await;
        }
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}
