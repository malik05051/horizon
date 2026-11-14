//! The game builds this launcher installs: malik05051/horizon's `orbital-mod-api-N` releases (made by
//! .github/workflows/orbital-build.yml), Horizon with the mod capabilities Orbital Laser needs. They carry no signed SHA256SUMS, so each archive is
//! installed only with the SHA-256 pinned for it in `orbital-releases.txt`; anything else is refused.

/// Where the game's releases come from. The launcher's own updates still come from `backend::REPO`.
pub const GAME_REPO: &str = "malik05051/horizon";

/// The tag prefix of those releases: `orbital-mod-api-13`.
pub const TAG_PREFIX: &str = "orbital-mod-api-";

const PINNED: &str = include_str!("../orbital-releases.txt");

/// The build number in an `orbital-mod-api-N` tag, or None for any other tag.
pub fn build(tag: &str) -> Option<u64> {
    let n = tag.strip_prefix(TAG_PREFIX)?;
    n.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| n.parse().ok())?
}

/// The pinned SHA-256 of `asset` in release `tag`, or why it must not be installed.
pub fn digest(tag: &str, asset: &str) -> Result<String, String> {
    pinned_in(PINNED, tag, asset).ok_or_else(|| {
        format!("{tag} ({asset}) is not one of the builds this launcher knows; update the launcher to install it.")
    })
}

/// Whether this launcher can install `asset` from release `tag`.
pub fn is_pinned(tag: &str, asset: &str) -> bool {
    pinned_in(PINNED, tag, asset).is_some()
}

fn pinned_in(text: &str, tag: &str, asset: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .find_map(|l| {
            let mut parts = l.split_whitespace();
            let (t, a, hex) = (parts.next()?, parts.next()?, parts.next()?);
            (t == tag && a == asset).then(|| hex.to_ascii_lowercase())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_give_their_build_number() {
        assert_eq!(build("orbital-mod-api-12"), Some(12));
        assert_eq!(build("orbital-mod-api-"), None);
        assert_eq!(build("orbital-mod-api-1x"), None);
        assert_eq!(build("v0.2.63"), None);
    }

    #[test]
    fn every_pinned_line_is_a_tag_an_archive_and_a_sha256() {
        let mut lines = 0;
        for line in PINNED
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        {
            let parts: Vec<_> = line.split_whitespace().collect();
            assert_eq!(parts.len(), 3, "{line}");
            assert!(build(parts[0]).is_some(), "{line}");
            assert!(parts[1].starts_with("horizon-orbital-"), "{line}");
            assert!(
                parts[2].len() == 64 && parts[2].bytes().all(|b| b.is_ascii_hexdigit()),
                "{line}"
            );
            lines += 1;
        }
        assert!(lines > 0, "orbital-releases.txt pins no build");
    }

    #[test]
    fn only_pinned_builds_install() {
        let text = "# comment\norbital-mod-api-3 horizon-orbital-linux-x86_64.tar.gz ABCD\n";
        assert_eq!(
            pinned_in(
                text,
                "orbital-mod-api-3",
                "horizon-orbital-linux-x86_64.tar.gz"
            ),
            Some("abcd".into())
        );
        assert_eq!(
            pinned_in(
                text,
                "orbital-mod-api-4",
                "horizon-orbital-linux-x86_64.tar.gz"
            ),
            None
        );
        assert_eq!(
            pinned_in(text, "orbital-mod-api-3", "horizon-orbital-windows-x64.zip"),
            None
        );
        assert!(digest("orbital-mod-api-0", "horizon-orbital-linux-x86_64.tar.gz").is_err());
        assert!(is_pinned(
            "orbital-mod-api-13",
            "horizon-orbital-linux-x86_64.tar.gz"
        ));
    }
}
