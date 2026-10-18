//! Asking GitHub whether there is a newer release.
//!
//! This only ever *reports*. It does not download the app, replace the
//! running binary, or run an installer — see `Release::page_url`, which is
//! what the About page sends the user to. Self-replacement on Windows cannot
//! be done honestly from inside the process that is being replaced: the
//! running .exe is locked, so it takes a helper that waits for this process
//! to exit and then swaps the file, and a helper that overwrites an
//! executable is indistinguishable from the thing every malware scanner is
//! built to stop. The download is a few megabytes and the user already has a
//! browser.
//!
//! The check is deliberately dumb about what it accepts: a tag it cannot
//! parse is *not* an update, so a malformed release can never nag somebody
//! into reinstalling. A network failure is likewise not an update; it is a
//! sentence saying the check could not be made.

use std::time::Duration;

/// GitHub's API host for the releases endpoint.
///
/// `/releases/latest` and not `/releases` because the latter includes
/// pre-releases and drafts, and an unfinished tag must never be offered to
/// somebody who only wants the stable build.
const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/Video-Capture-Guide/vcg-deinterlacer-rust/releases/latest";

/// GitHub rejects API requests with no user agent, and asks that it identify
/// the application.
const USER_AGENT: &str = concat!("vcg-deinterlacer/", env!("CARGO_PKG_VERSION"));

/// A whole check, start to finish, must not outlast somebody's patience.
///
/// The About page runs this on a worker thread, so a slow answer does not
/// freeze the window — but an unbounded wait would leave the thread and the
/// spinner alive long after the user stopped caring.
const TIMEOUT: Duration = Duration::from_secs(10);

/// GitHub's release payload is large; only two fields are wanted.
const MAX_BODY: u64 = 256 * 1024;

/// What the check concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// This build is the newest published one, or newer than it.
    UpToDate,
    /// A newer release exists.
    Available(Release),
    /// The check could not be made. Holds a sentence for the screen — not a
    /// reason to warn anybody, since being offline is not a problem with the
    /// app.
    Failed(String),
}

/// One published release, reduced to what the screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag as published, e.g. `v2.1.0`.
    pub tag: String,
    /// The comparable form, with any leading `v` removed.
    pub version: String,
    /// Where a human goes to read about it and download it.
    pub page_url: String,
}

/// Ask GitHub what the latest release is, and compare it with `current`.
///
/// Blocking: call it on a worker thread.
pub fn check(current: &str) -> Outcome {
    let body = match fetch() {
        Ok(b) => b,
        Err(e) => return Outcome::Failed(e),
    };

    let Some(tag) = field(&body, "tag_name") else {
        return Outcome::Failed("GitHub's answer did not name a release".into());
    };

    let latest = tag.trim_start_matches('v').to_string();
    let Some(order) = compare(&latest, current.trim_start_matches('v')) else {
        // An unparseable tag is not an update. Saying "up to date" here is the
        // safe reading: the alternative is nagging somebody to reinstall on
        // the strength of a version string nobody can order.
        return Outcome::UpToDate;
    };

    if order == std::cmp::Ordering::Greater {
        Outcome::Available(Release {
            page_url: field(&body, "html_url").unwrap_or_else(|| {
                format!("{}/releases/latest", vcg_repo())
            }),
            tag,
            version: latest,
        })
    } else {
        Outcome::UpToDate
    }
}

fn vcg_repo() -> &'static str {
    "https://github.com/Video-Capture-Guide/vcg-deinterlacer-rust"
}

fn fetch() -> Result<String, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(USER_AGENT)
        // Let this function see the status code instead of having 4xx and 5xx
        // raised as transport errors. A repository with no releases yet
        // answers 404, and "no releases have been published yet" is a very
        // different sentence from "could not reach GitHub".
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();

    let mut response = agent
        .get(LATEST_RELEASE_API)
        // Asking for the documented media type keeps the shape of the answer
        // stable across GitHub's own API changes.
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| friendly(&e.to_string()))?;

    let status = response.status();
    if status == 404 {
        // A repository with no published releases answers 404, which is not
        // an error the user can act on and must not read like one.
        return Err("no releases have been published yet".into());
    }
    if status != 200 {
        return Err(format!("GitHub returned HTTP {status}"));
    }

    response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .map_err(|e| friendly(&e.to_string()))
}

/// Turn a transport error into something worth putting on a screen.
fn friendly(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        "GitHub did not answer in time".into()
    } else if lower.contains("dns") || lower.contains("resolve") {
        "could not reach github.com — check your internet connection".into()
    } else {
        format!("could not reach GitHub ({raw})")
    }
}

/// Pull one string field out of a JSON object.
///
/// Hand-rolled rather than pulling `serde_json` into this crate for two
/// fields. It looks for `"key"` followed by a colon and a quoted string, and
/// unescapes only what GitHub actually emits in a tag or a URL.
fn field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    loop {
        let at = json[from..].find(&needle)? + from;
        let rest = &json[at + needle.len()..];
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            // A *value* that happens to contain the key's text. Keep looking.
            from = at + needle.len();
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            from = at + needle.len();
            continue;
        };
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => return None,
                },
                other => out.push(other),
            }
        }
        return None;
    }
}

/// Order two dotted version strings.
///
/// `None` when either side is not a dotted number, which is the signal to do
/// nothing rather than to guess. A pre-release suffix on *either* side is
/// ignored for ordering and then broken the tie with: `2.1.0` is newer than
/// `2.1.0-dev`, which is the whole reason the running build is allowed to
/// call a same-numbered release an update.
fn compare(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let (a_nums, a_pre) = split(a)?;
    let (b_nums, b_pre) = split(b)?;

    let len = a_nums.len().max(b_nums.len());
    for i in 0..len {
        let x = a_nums.get(i).copied().unwrap_or(0);
        let y = b_nums.get(i).copied().unwrap_or(0);
        if x != y {
            return Some(x.cmp(&y));
        }
    }

    // Same numbers: a release with no suffix beats one with a suffix.
    Some(match (a_pre, b_pre) {
        (false, true) => std::cmp::Ordering::Greater,
        (true, false) => std::cmp::Ordering::Less,
        _ => std::cmp::Ordering::Equal,
    })
}

/// Split `1.2.3-dev` into its numbers and whether it carried a suffix.
fn split(v: &str) -> Option<(Vec<u32>, bool)> {
    let (core, pre) = match v.split_once(['-', '+']) {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut nums = Vec::new();
    for part in core.split('.') {
        nums.push(part.parse::<u32>().ok()?);
    }
    if nums.is_empty() { None } else { Some((nums, pre)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_number_is_newer() {
        assert_eq!(compare("2.1.0", "2.0.1"), Some(std::cmp::Ordering::Greater));
        assert_eq!(compare("2.0.1", "2.1.0"), Some(std::cmp::Ordering::Less));
        assert_eq!(compare("2.0.1", "2.0.1"), Some(std::cmp::Ordering::Equal));
    }

    #[test]
    fn missing_components_count_as_zero() {
        // `v3` and `3.0.0` are the same release named two ways.
        assert_eq!(compare("3", "3.0.0"), Some(std::cmp::Ordering::Equal));
        assert_eq!(compare("3.0.1", "3"), Some(std::cmp::Ordering::Greater));
    }

    #[test]
    fn a_release_beats_the_dev_build_of_the_same_number() {
        // This is the case that matters: the app ships as `2.0.1-dev` and the
        // published release is `v2.0.1`. Without this the user would never be
        // told their pre-release build has been superseded.
        assert_eq!(
            compare("2.0.1", "2.0.1-dev"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare("2.0.1-dev", "2.0.1"),
            Some(std::cmp::Ordering::Less)
        );
    }

    #[test]
    fn nonsense_versions_are_not_an_update() {
        assert_eq!(compare("nightly", "2.0.1"), None);
        assert_eq!(compare("2.0.1", "banana"), None);
    }

    #[test]
    fn fields_are_read_out_of_a_release_payload() {
        let body = r#"{"url":"https://api.github.com/x","tag_name":"v2.1.0",
            "name":"tag_name is not here","html_url":"https://github.com/o/r/releases/tag/v2.1.0"}"#;
        assert_eq!(field(body, "tag_name").as_deref(), Some("v2.1.0"));
        assert_eq!(
            field(body, "html_url").as_deref(),
            Some("https://github.com/o/r/releases/tag/v2.1.0")
        );
        assert_eq!(field(body, "absent"), None);
    }

    #[test]
    fn a_key_appearing_inside_a_value_is_skipped() {
        // The naive search finds the one inside the string value first.
        let body = r#"{"name":"mentions \"tag_name\" in passing","tag_name":"v9.9.9"}"#;
        assert_eq!(field(body, "tag_name").as_deref(), Some("v9.9.9"));
    }
}
