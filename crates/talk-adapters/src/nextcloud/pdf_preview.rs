//! The app-bundled macOS PDF renderer accepts only finite stdin, never URLs or
//! paths. One process is retained until exit/reap, including caller cancellation.
use talk_core::ports::ErrorKind;
use talk_core::protocol::content::{ImageBytes, MAX_IMAGE_BYTES};

const HEADER_LIMIT: usize = 32;
fn decode_output(output: Vec<u8>) -> Result<ImageBytes, ErrorKind> {
    if output.len() > MAX_IMAGE_BYTES + HEADER_LIMIT {
        return Err(ErrorKind::TooLarge);
    }
    let end = output
        .iter()
        .take(HEADER_LIMIT)
        .position(|byte| *byte == b'\n')
        .ok_or(ErrorKind::Malformed)?;
    let pages = std::str::from_utf8(&output[..end])
        .ok()
        .and_then(|header| header.strip_prefix("NMT-PDF-1 "))
        .filter(|pages| !pages.is_empty() && pages.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|pages| pages.parse::<u32>().ok())
        .filter(|pages| (1..=10_000).contains(pages))
        .ok_or(ErrorKind::Malformed)?;
    let bytes = output.get(end + 1..).ok_or(ErrorKind::Malformed)?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(ErrorKind::Malformed);
    }
    ImageBytes::first_pdf_page(bytes.to_vec(), pages).map_err(|_| ErrorKind::TooLarge)
}

#[cfg(target_os = "macos")]
pub(super) async fn render(
    bytes: Vec<u8>,
    large: bool,
    request: talk_core::lifecycle::Cancellation,
    account: talk_core::lifecycle::Cancellation,
    timeout: std::time::Duration,
) -> Result<ImageBytes, ErrorKind> {
    static SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    if bytes.len() > MAX_IMAGE_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err(ErrorKind::TooLarge);
    }
    let permit = SLOT.try_acquire().map_err(|_| ErrorKind::Busy)?;
    let executable = std::env::current_exe().map_err(|_| ErrorKind::Unsupported)?;
    let directory = executable
        .parent()
        .filter(|directory| {
            directory.file_name().is_some_and(|name| name == "MacOS")
                && directory
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .is_some_and(|name| name == "Contents")
        })
        .ok_or(ErrorKind::Unsupported)?;
    let helper = directory.join("talk-native-helper");
    run_owned(helper, bytes, large, timeout, [request, account], permit).await
}

#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
async fn run_owned(
    helper: std::path::PathBuf,
    bytes: Vec<u8>,
    large: bool,
    timeout: std::time::Duration,
    cancellations: [talk_core::lifecycle::Cancellation; 2],
    permit: tokio::sync::SemaphorePermit<'static>,
) -> Result<ImageBytes, ErrorKind> {
    use talk_core::lifecycle::Cancellation;
    struct CancelOnDrop(Cancellation);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            self.0.cancel();
        }
    }
    let lifetime = CancelOnDrop(Cancellation::default());
    let cancelled = lifetime.0.clone();
    // This finite owner survives a dropped caller only to kill/reap its child.
    // It retains the sole process slot throughout cleanup; a cancelled UI task
    // cannot admit a second decoder while the first is still running.
    let task = tokio::spawn(async move {
        let _permit = permit;
        let [request, account] = cancellations;
        run(helper, bytes, large, timeout, [cancelled, request, account]).await
    });
    task.await.map_err(|_| ErrorKind::Unsupported)?
}

#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
async fn run(
    helper: std::path::PathBuf,
    bytes: Vec<u8>,
    large: bool,
    timeout: std::time::Duration,
    cancellations: [talk_core::lifecycle::Cancellation; 3],
) -> Result<ImageBytes, ErrorKind> {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if cancellations.iter().any(|cancel| cancel.is_cancelled()) {
        return Err(ErrorKind::Cancelled);
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ErrorKind::TooLarge);
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err(ErrorKind::Malformed);
    }
    if timeout.is_zero() {
        return Err(ErrorKind::Deadline);
    }
    let mut child = tokio::process::Command::new(helper)
        .args(["pdf-preview", if large { "large" } else { "thumbnail" }])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ErrorKind::Unsupported)?;
    let input = child.stdin.take();
    let output = child.stdout.take();
    let operation = async {
        let mut input = input.ok_or(ErrorKind::Unsupported)?;
        let output = output.ok_or(ErrorKind::Unsupported)?;
        let write = async {
            input
                .write_all(&bytes)
                .await
                .map_err(|_| ErrorKind::Unsupported)?;
            input.shutdown().await.map_err(|_| ErrorKind::Unsupported)?;
            drop(input);
            Ok::<_, ErrorKind>(())
        };
        let read = async {
            let mut data = Vec::new();
            output
                .take((MAX_IMAGE_BYTES + HEADER_LIMIT + 1) as u64)
                .read_to_end(&mut data)
                .await
                .map_err(|_| ErrorKind::Unsupported)?;
            decode_output(data)
        };
        let (_, image) = tokio::try_join!(write, read)?;
        let status = child.wait().await.map_err(|_| ErrorKind::Unsupported)?;
        if !status.success() {
            return Err(ErrorKind::Unsupported);
        }
        Ok(image)
    };
    let cancellation = async {
        loop {
            if cancellations.iter().any(|cancel| cancel.is_cancelled()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancellation => Err(ErrorKind::Cancelled),
        result = tokio::time::timeout(timeout, operation) => result.unwrap_or(Err(ErrorKind::Deadline)),
    };
    if result.is_err() {
        let _ = child.start_kill();
    }
    // Reap even on a malformed reply, timeout, lost account or dropped caller.
    let _ = child.wait().await;
    if cancellations.iter().any(|cancel| cancel.is_cancelled()) {
        return Err(ErrorKind::Cancelled);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_pdf_result_requires_bounded_png_and_observed_page_count() {
        let png = b"\x89PNG\r\n\x1a\n";
        let value = decode_output([b"NMT-PDF-1 3\n".as_slice(), png].concat()).unwrap();
        assert_eq!(value.document_pages(), Some(3));
        assert_eq!(value.as_slice(), png);
        for bad in [
            b"NMT-PDF-1 0\n".as_slice(),
            b"NMT-PDF-1 10001\n",
            b"NMT-PDF-1 +2\n",
            b"2\n",
        ] {
            assert!(decode_output([bad, png].concat()).is_err());
        }
        assert!(decode_output(b"NMT-PDF-1 1\nnot a PNG".to_vec()).is_err());
        assert!(decode_output(vec![0; MAX_IMAGE_BYTES + HEADER_LIMIT + 1]).is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod process_tests {
    use super::*;
    use std::{
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::Duration,
    };
    use talk_core::lifecycle::Cancellation;

    fn helper(script: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("renderer.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (directory, path)
    }
    async fn pid(path: &Path) -> u32 {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(path.with_extension("sh.pid"))
                    && let Ok(pid) = value.parse::<u32>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("helper started")
    }
    fn reaped(pid: u32) {
        assert!(
            !PathBuf::from(format!("/proc/{pid}")).exists(),
            "child was killed and reaped"
        );
    }
    fn input() -> Vec<u8> {
        b"%PDF-1.7\nfixture".to_vec()
    }
    fn cancellations() -> [Cancellation; 3] {
        std::array::from_fn(|_| Cancellation::default())
    }

    #[tokio::test]
    async fn helper_success_malformed_and_output_limit_use_production_io() {
        let (_directory, path) =
            helper("cat >/dev/null\nprintf 'NMT-PDF-1 2\\n\\211PNG\\r\\n\\032\\n'");
        let image = run(path, input(), true, Duration::from_secs(2), cancellations())
            .await
            .unwrap();
        assert_eq!(image.document_pages(), Some(2));
        let (_directory, path) = helper("cat >/dev/null\nprintf 'invalid'");
        assert_eq!(
            run(
                path,
                input(),
                false,
                Duration::from_secs(2),
                cancellations()
            )
            .await
            .unwrap_err(),
            ErrorKind::Malformed
        );
        let (_directory, path) = helper("cat >/dev/null\nhead -c 4194337 /dev/zero");
        assert_eq!(
            run(
                path,
                input(),
                false,
                Duration::from_secs(2),
                cancellations()
            )
            .await
            .unwrap_err(),
            ErrorKind::TooLarge
        );
    }

    #[tokio::test]
    async fn helper_timeout_and_cancellation_kill_and_reap_even_during_input_write() {
        for cancel in [false, true] {
            let (_directory, path) = helper("printf '%s' \"$$\" > \"$0.pid\"\nexec sleep 10");
            let cancels = cancellations();
            let trigger = cancels[1].clone();
            let mut bytes = input();
            bytes.resize(1024 * 1024, b' ');
            let process = tokio::spawn(run(
                path.clone(),
                bytes,
                true,
                Duration::from_millis(500),
                cancels,
            ));
            let pid = pid(&path).await;
            if cancel {
                trigger.cancel();
            }
            assert_eq!(
                process.await.unwrap().unwrap_err(),
                if cancel {
                    ErrorKind::Cancelled
                } else {
                    ErrorKind::Deadline
                }
            );
            reaped(pid);
        }
    }

    #[tokio::test]
    async fn dropped_caller_retains_single_process_slot_until_child_is_reaped() {
        static SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        let (_directory, path) = helper("printf '%s' \"$$\" > \"$0.pid\"\nexec sleep 10");
        let process = tokio::spawn(run_owned(
            path.clone(),
            input(),
            true,
            Duration::from_secs(5),
            [Cancellation::default(), Cancellation::default()],
            SLOT.try_acquire().unwrap(),
        ));
        let pid = pid(&path).await;
        assert!(SLOT.try_acquire().is_err());
        process.abort();
        assert!(process.await.unwrap_err().is_cancelled());
        let _released = tokio::time::timeout(Duration::from_secs(2), SLOT.acquire())
            .await
            .unwrap()
            .unwrap();
        reaped(pid);
    }
}
