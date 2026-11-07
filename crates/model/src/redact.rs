//! Text that is safe to print: endpoints without credentials, and bounded text from a node
//! or collector.

/// `url` with only its scheme, host and port, for example `wss://node.example:443`. User name,
/// password, path, query and fragment are dropped: any of them can hold an API key.
pub fn endpoint(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "(URL hidden)".into();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{scheme}://{host}")
}

/// Most characters of text from a node or collector that are printed.
pub const MAX_UNTRUSTED_CHARS: usize = 512;

/// Text from a node or collector, cut to [`MAX_UNTRUSTED_CHARS`] characters, with control
/// characters such as terminal escapes and line breaks replaced by spaces.
pub fn untrusted(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .by_ref()
        .take(MAX_UNTRUSTED_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_keep_only_scheme_host_and_port() {
        for (url, shown) in [
            (
                "wss://user:secret@node.example:443/key?apikey=k#f",
                "wss://node.example:443",
            ),
            ("https://node.example/v1/abcdef", "https://node.example"),
            ("http://127.0.0.1:4318", "http://127.0.0.1:4318"),
            ("http://[::1]:9944?token=t", "http://[::1]:9944"),
            ("secret@node", "(URL hidden)"),
        ] {
            assert_eq!(endpoint(url), shown);
        }
        let long = "é\u{1b}[31m".repeat(200);
        let cut = untrusted(&long);
        assert_eq!(cut.chars().count(), MAX_UNTRUSTED_CHARS + 1);
        assert!(cut.ends_with('…') && !cut.contains('\u{1b}'));
    }
}
