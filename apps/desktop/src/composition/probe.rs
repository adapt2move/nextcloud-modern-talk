//! Explicit development control, owned by composition. No transport/credential
//! bytes enter presentation or output. The external runner owns the deadline.
use super::*;
#[cfg(debug_assertions)]
pub(super) fn attach(
    window: gpui::WindowHandle<Desktop>,
    profile: std::path::PathBuf,
    cx: &mut App,
) {
    let _ = window.update(cx, |view, _, _| view.enable_latency_probe());
    cx.spawn(async move |cx| {
        let mut previous = String::new();
        for _ in 0..180_000 {
            gpui::Timer::after(Duration::from_millis(20)).await;
            let path = profile.join("probe.command");
            let input = cx
                .background_executor()
                .spawn(async move {
                    use std::io::Read;
                    let file = std::fs::File::open(path).ok()?;
                    let mut bytes = Vec::with_capacity(129);
                    file.take(129).read_to_end(&mut bytes).ok()?;
                    (bytes.len() <= 128).then_some(bytes)
                })
                .await;
            let Some(input) = input.and_then(|b| String::from_utf8(b).ok()) else {
                continue;
            };
            if previous == input {
                continue;
            }
            previous = input.clone();
            let mut fields = input.split_whitespace();
            let Some(sequence) = fields.next().and_then(|v| v.parse::<u64>().ok()) else {
                continue;
            };
            let Some(action) = fields.next() else {
                continue;
            };
            if action == "quit" {
                let _ = cx.update(|cx| cx.quit());
                break;
            }
            let result = window.update(cx, |view, window, cx| {
                view.latency_probe_action(action, window, cx)
            });
            match result {
                Ok(status) => println!("DEV_ACK {sequence} {status}"),
                Err(_) => break,
            }
        }
    })
    .detach();
}
#[cfg(not(debug_assertions))]
pub(super) fn attach(_: gpui::WindowHandle<Desktop>, _: std::path::PathBuf, _: &mut App) {}
