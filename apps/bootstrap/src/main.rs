//! Headless composition diagnostic. This is not the native client executable.
mod presentation;

use talk_adapters::identity::UnconfiguredIdentity;
use talk_adapters::media::UnconfiguredMedia;
use talk_adapters::nextcloud::UnconfiguredNextcloud;
use talk_core::application::Bootstrap;

fn main() {
    let mut application = Bootstrap::new(
        UnconfiguredNextcloud,
        UnconfiguredIdentity,
        UnconfiguredMedia,
    );
    presentation::describe(&mut application);
}
