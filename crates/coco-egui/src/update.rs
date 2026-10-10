//! Update check against the GitHub releases API: once per launch (unless
//! `check_for_updates` is off, `config.rs`) and on Help > "Check for
//! Updates…", a background thread fetches the latest published release and
//! compares its tag with the built version. Nothing is downloaded or
//! installed; a newer release shows as a notice in the welcome panel
//! (`manager/welcome.rs`) and a line in the About window (`about.rs`), both
//! linking to the release page.
//!
//! `GET /repos/{owner}/{repo}/releases/latest` returns the newest release
//! that is neither a draft nor a prerelease; its `tag_name` is `vX.Y.Z`
//! (the `release` skill's tag format) and `html_url` is its release page.
//! GitHub refuses requests without a `User-Agent` and allows 60
//! unauthenticated requests per hour per IP.

use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;

/// Label of every menu item that starts a check.
pub(crate) const MENU_LABEL: &str = "Check for Updates…";

/// The GitHub API endpoint for the latest published release.
pub(crate) const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/sperano/cocovm/releases/latest";

/// Every release page lives under this prefix; an `html_url` outside it is
/// refused rather than offered as a link.
const RELEASE_PAGE_PREFIX: &str = "https://github.com/sperano/cocovm/releases/";

/// GitHub's recommended media type for REST API responses.
const ACCEPT: &str = "application/vnd.github+json";

const USER_AGENT: &str = concat!("cocovm/", env!("CARGO_PKG_VERSION"));

/// Upper bound on the whole request, so a stalled connection cannot keep
/// "Checking for updates…" on screen indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Release tags are the version with this prefix (`v0.7.9`).
const TAG_PREFIX: char = 'v';

/// A published release, as far as the notice needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub(crate) version: semver::Version,
    /// The release's GitHub page: notes and downloads.
    pub(crate) page_url: String,
}

/// The fields read from the API's release object; the rest are ignored.
#[derive(serde::Deserialize)]
struct ReleaseJson {
    tag_name: String,
    html_url: String,
}

/// Parse the API's release object.
pub(crate) fn parse_release(json: &str) -> Result<Release, String> {
    let release: ReleaseJson =
        serde_json::from_str(json).map_err(|e| format!("unexpected release data: {e}"))?;
    let tag = release.tag_name.as_str();
    let version = semver::Version::parse(tag.strip_prefix(TAG_PREFIX).unwrap_or(tag))
        .map_err(|e| format!("release tag '{tag}' is not a version: {e}"))?;
    if !release.html_url.starts_with(RELEASE_PAGE_PREFIX) {
        return Err(format!(
            "release page '{}' is not a CoCoVM release",
            release.html_url
        ));
    }
    Ok(Release {
        version,
        page_url: release.html_url,
    })
}

/// The version this binary was built as.
pub(crate) fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package versions are semver")
}

/// Fetch and parse the latest release from `url` (an API endpoint shaped
/// like [`LATEST_RELEASE_URL`]). Blocking; run it off the UI thread.
pub(crate) fn fetch_latest(url: &str) -> Result<Release, String> {
    let mut response = ureq::get(url)
        .header("Accept", ACCEPT)
        .header("User-Agent", USER_AGENT)
        .config()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .call()
        .map_err(|e| e.to_string())?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_release(&body)
}

/// Where the check stands.
#[derive(Debug, Default)]
pub(crate) enum Status {
    /// No check has run this session.
    #[default]
    Idle,
    /// A check is running on its thread; its result arrives here.
    Checking(mpsc::Receiver<Result<Release, String>>),
    UpToDate,
    Available(Release),
    Failed(String),
}

/// The session's update check: its status, whether the user asked for it,
/// and whether the welcome notice was dismissed.
#[derive(Debug)]
pub(crate) struct UpdateCheck {
    /// API endpoint to query; [`LATEST_RELEASE_URL`] outside tests.
    url: String,
    status: Status,
    /// Whether the last check came from the menu. A failure is shown only
    /// then: an offline launch should not greet the user with an error.
    requested: bool,
    /// The welcome notice's Dismiss: hides it for the rest of the session.
    dismissed: bool,
}

impl UpdateCheck {
    pub(crate) fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            status: Status::Idle,
            requested: false,
            dismissed: false,
        }
    }

    /// Start a check on a background thread, unless one is already running.
    /// `requested` marks a check the user asked for from the menu, which
    /// also brings back a dismissed notice.
    pub(crate) fn start(&mut self, ctx: &egui::Context, requested: bool) {
        if requested {
            self.requested = true;
            self.dismissed = false;
        }
        if matches!(self.status, Status::Checking(_)) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let url = self.url.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_latest(&url));
            ctx.request_repaint();
        });
        self.status = Status::Checking(rx);
    }

    /// Fold a finished check's result into [`Self::status`].
    pub(crate) fn poll(&mut self) {
        let Status::Checking(rx) = &self.status else {
            return;
        };
        self.status = match rx.try_recv() {
            Ok(Ok(release)) if release.version > current_version() => Status::Available(release),
            Ok(Ok(_)) => Status::UpToDate,
            Ok(Err(e)) => {
                tracing::info!("update check failed: {e}");
                Status::Failed(e)
            }
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Status::Failed("update check thread exited unexpectedly".to_string())
            }
        };
    }

    pub(crate) fn status(&self) -> &Status {
        &self.status
    }

    /// Whether the user asked for the last check (see [`Self::requested`]).
    pub(crate) fn requested(&self) -> bool {
        self.requested
    }

    /// The newer release the welcome panel should announce, if any.
    pub(crate) fn notice(&self) -> Option<&Release> {
        match &self.status {
            Status::Available(release) if !self.dismissed => Some(release),
            _ => None,
        }
    }

    pub(crate) fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Seed a finished status without a network round trip.
    #[cfg(test)]
    pub(crate) fn set_status(&mut self, status: Status) {
        self.status = status;
    }
}

#[cfg(test)]
#[path = "update_test.rs"]
pub(crate) mod tests;
