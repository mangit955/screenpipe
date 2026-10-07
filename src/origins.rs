// Copied verbatim from owned_browser.rs.
/// The app's own origins, which Tauri grants the app's IPC permissions.
struct AppOrigins {
    /// The dev server, which Tauri treats as the app's origin in dev builds.
    dev_server: Option<url::Url>,
    /// The app's custom protocols run over http hosts, as on Windows. See
    /// [`is_windows_app_url`].
    windows: bool,
}

/// Whether the owned browser may load `url`. Tauri grants IPC to the app's own
/// origins: its custom protocols (`tauri:`, `ipc:`, `asset:`; see
/// [`is_windows_app_url`] for Windows) and the dev server. A page that reached
/// one would run with the app's capabilities, so only web URLs outside those
/// origins pass. Iframes need `about:`, `data:` and `blob:`; a `blob:` URL
/// carries the origin of the page that created it.
fn is_web_page_url(url: &url::Url, origins: &AppOrigins) -> bool {
    match url.scheme() {
        "http" | "https" => {
            !(origins.windows && is_windows_app_url(url))
                && origins
                    .dev_server
                    .as_ref()
                    .is_none_or(|dev| dev.origin() != url.origin())
        }
        "about" | "data" => true,
        "blob" => match url::Url::parse(url.path()) {
            Ok(creator) => {
                matches!(creator.scheme(), "http" | "https") && is_web_page_url(&creator, origins)
            }
            // Opaque creator such as a sandboxed frame: `blob:null/<id>`.
            Err(_) => true,
        },
        _ => false,
    }
}

/// Whether an http(s) URL reaches the app's custom protocols on Windows.
/// Tauri trusts `<protocol>.localhost` over http and https; every `.localhost`
/// host is blocked so a protocol added later is covered too. wry serves a
/// protocol at any URL starting with `http://<protocol>.`, compared as a raw
/// string, so `http://tauri.example.com/` and `http://tauri.localhost@example.com/`
/// count. That routing is http-only because the app leaves `useHttpsScheme`
/// off, so `https://tauri.app` is a normal site.
fn is_windows_app_url(url: &url::Url) -> bool {
    url.host_str()
        .is_some_and(|host| host.ends_with(".localhost"))
        || url.as_str().strip_prefix("http://").is_some_and(|rest| {
            ["tauri", "ipc", "asset"].iter().any(|protocol| {
                rest.strip_prefix(protocol)
                    .is_some_and(|rest| rest.starts_with('.'))
            })
        })
}
