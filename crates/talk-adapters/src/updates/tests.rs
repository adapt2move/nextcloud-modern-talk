use super::*;

fn selection(
    releases: serde_json::Value,
    current: &str,
    channel: UpdateChannel,
) -> Option<(String, ReleaseInfo)> {
    select_release(
        &serde_json::to_vec(&releases).unwrap(),
        &Version::parse(current).unwrap(),
        channel,
    )
    .unwrap()
}
#[test]
fn channels_use_semver_precedence_and_never_offer_drafts_or_downgrades() {
    let releases = serde_json::json!([
        {"tag_name":"v0.2.0-rc.2","draft":false,"prerelease":true},
        {"tag_name":"v0.1.10","draft":false,"prerelease":false},
        {"tag_name":"v0.1.9","draft":false,"prerelease":false},
        {"tag_name":"v9.0.0","draft":true,"prerelease":false},
        {"tag_name":"v0.2.0-alpha.10","draft":false,"prerelease":false},
        {"tag_name":"v0.2.0-alpha.9","draft":false,"prerelease":true}
    ]);
    assert_eq!(
        selection(releases.clone(), "0.1.9", UpdateChannel::Stable)
            .unwrap()
            .0,
        "v0.1.10"
    );
    assert_eq!(
        selection(releases.clone(), "0.1.9", UpdateChannel::Prerelease)
            .unwrap()
            .0,
        "v0.2.0-rc.2"
    );
    assert!(selection(releases.clone(), "0.1.10", UpdateChannel::Stable).is_none());
    assert!(selection(releases, "1.0.0", UpdateChannel::Prerelease).is_none());
}
#[test]
fn alpha_release_flag_and_build_metadata_are_respected() {
    let releases = serde_json::json!([
        {"tag_name":"v0.0.1","draft":false,"prerelease":true},
        {"tag_name":"v0.0.0+new-build","draft":false,"prerelease":false}
    ]);
    assert!(selection(releases.clone(), "0.0.0+old-build", UpdateChannel::Stable).is_none());
    assert_eq!(
        selection(releases, "0.0.0", UpdateChannel::Prerelease)
            .unwrap()
            .0,
        "v0.0.1"
    );
}
#[test]
fn only_fixed_repository_version_urls_can_reach_browser() {
    for bad in [
        "../malicious",
        "v1.0.0?redirect=x",
        "https://elsewhere.test",
        "v1.0.0#fragment",
        "v1.0.0%2f",
        "",
    ] {
        assert!(release_url(bad).is_err(), "{bad}");
    }
    assert_eq!(
        release_url("v1.2.3-beta.2").unwrap(),
        format!("{RELEASES}v1.2.3-beta.2")
    );
    let releases = serde_json::json!([{ "tag_name":"v1.0.0","draft":false,"prerelease":false,"html_url":"https://elsewhere.test" }]);
    let (tag, _) = selection(releases, "0.0.1", UpdateChannel::Stable).unwrap();
    assert_eq!(release_url(&tag).unwrap(), format!("{RELEASES}v1.0.0"));
}
#[test]
fn incomplete_oversized_or_wrong_shape_responses_fail() {
    let current = Version::parse("0.0.1").unwrap();
    for malformed in [
        b"{}".as_slice(),
        b"[{}]",
        b"null",
        b"[",
        b"[{\"draft\":false,\"prerelease\":0,\"tag_name\":\"v1.0.0\"}]",
    ] {
        assert_eq!(
            select_release(malformed, &current, UpdateChannel::Stable),
            Err(UpdateError::InvalidResponse)
        );
    }
    assert_eq!(
        select_release(&vec![b' '; MAX_BODY + 1], &current, UpdateChannel::Stable),
        Err(UpdateError::InvalidResponse)
    );
    let many = serde_json::to_vec(&vec![serde_json::json!({"draft":true}); 101]).unwrap();
    assert_eq!(
        select_release(&many, &current, UpdateChannel::Stable),
        Err(UpdateError::InvalidResponse)
    );
}
#[test]
fn offline_start_and_stop_own_the_worker_without_network() {
    let owner = UpdateRuntime::start("0.0.1").unwrap();
    let snapshot = owner.snapshot();
    assert_eq!(snapshot.state, UpdateState::Idle);
    assert!(owner.poll(snapshot.revision).is_none());
    assert_eq!(owner.open_release(), Err(UpdateError::Unavailable));
    owner.request_shutdown();
    assert_eq!(
        owner.check(UpdateChannel::Stable),
        Err(UpdateError::Stopped)
    );
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while !owner.is_stopped() && std::time::Instant::now() < until {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(owner.is_stopped());
    assert_eq!(owner.snapshot().state, UpdateState::Stopped);
}
#[test]
fn shutdown_cancels_pending_http_future() {
    let owner = UpdateRuntime::start("0.0.1").unwrap();
    owner.request_shutdown();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime.block_on(interruptible::<()>(&owner.shared, std::future::pending()));
    assert_eq!(result, Err(UpdateError::Stopped));
}
