// Minimal wry/WebKitGTK harness: run one scenario in one process.
// Usage: wv-harness <data-literal|data|surrogate|nav> <control|fix>

use std::borrow::Cow;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{http::Response, WebViewBuilder};

include!("origins.rs");
mod linux_block {
    include!("linux_block.rs");
}

macro_rules! log {
    ($($t:tt)*) => {{
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, $($t)*);
        let _ = o.flush();
    }};
}

/// Stands in for Tauri's `PlatformWebview`, whose `inner()` is the
/// `webkit2gtk::WebView`.
pub struct Platform(webkit2gtk::WebView);
impl Platform {
    fn inner(&self) -> webkit2gtk::WebView {
        self.0.clone()
    }
}

const DATA_LITERAL: &str =
    "data:text/html,<script>window.webkit.messageHandlers.ipc.postMessage('x')</script>";
const DATA_INSTRUMENTED: &str = "data:text/html,<script>var r='';try{r+='typeof_webkit='+typeof(window.webkit);r+=';typeof_messageHandlers='+typeof(window.webkit&&window.webkit.messageHandlers);r+=';typeof_ipc='+typeof(window.webkit&&window.webkit.messageHandlers&&window.webkit.messageHandlers.ipc)}catch(e){r+=';probe_threw:'+e}try{window.webkit.messageHandlers.ipc.postMessage('x');r+=';postMessage=ok'}catch(e){r+=';postMessage=threw:'+e}document.title='RESULT:'+r</script>";

const SURROGATE_PAGE: &str = r#"<!doctype html><title>start</title><script>
var r = '';
try {
  r += 'typeof_webkit=' + typeof (window.webkit);
  r += ';typeof_messageHandlers=' + typeof (window.webkit && window.webkit.messageHandlers);
  r += ';typeof_ipc=' + typeof (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.ipc);
} catch (e) { r += ';probe_threw:' + e; }
try { window.webkit.messageHandlers.ipc.postMessage('\uD800'); r += ';postMessage=ok'; }
catch (e) { r += ';postMessage=threw:' + e; }
document.title = 'RESULT:' + r;
</script>"#;

const NAV_PAGE: &str = r#"<!doctype html><html><head><title>nav-start</title><script>
var loads = {};
function L(el) { loads[el.id] = (loads[el.id] || 0) + 1; }
var msgs = [];
window.addEventListener('message', function (e) { msgs.push(String(e.data)); });
function href(el) {
  try {
    var w = el.contentWindow;
    if (!w) return 'no-contentWindow';
    return w.location.href;
  } catch (e) { return 'cross-origin'; }
}
</script></head><body>
<iframe id="f_tauri" src="tauri://localhost/iframe" onload="L(this)"></iframe>
<iframe id="f_inner" src="/inner" onload="L(this)"></iframe>
<object id="o_tauri" data="tauri://localhost/object" type="text/html" onload="L(this)"></object>
<embed id="e_tauri" src="tauri://localhost/embed" type="text/html" onload="L(this)">
<iframe id="f_example" src="https://example.com/" onload="L(this)"></iframe>
<iframe id="f_xorigin" src="http://127.0.0.1:PORTB/xorigin" onload="L(this)"></iframe>
<iframe id="f_about" src="about:blank" onload="L(this)"></iframe>
<iframe id="f_data" src="data:text/html,<p>data-frame</p><script>parent.postMessage('loaded:data-frame','*')</script>" onload="L(this)"></iframe>
<script>
var blobUrl = URL.createObjectURL(new Blob(["<p>blob-frame</p><script>parent.postMessage('loaded:blob-frame','*')<\/script>"], {type: 'text/html'}));
var fb = document.createElement('iframe');
fb.id = 'f_blob';
fb.onload = function () { L(fb); };
fb.src = blobUrl;
document.body.appendChild(fb);
setTimeout(function () {
  var ids = ['f_tauri', 'f_inner', 'o_tauri', 'e_tauri', 'f_example', 'f_xorigin', 'f_about', 'f_data', 'f_blob'];
  var out = {};
  ids.forEach(function (id) {
    var el = document.getElementById(id);
    out[id] = { href: href(el), onload: loads[id] || 0 };
  });
  document.title = 'RESULT:' + JSON.stringify({ frames: out, messages: msgs });
  setTimeout(function () {
    location.href = 'tauri://localhost/top';
    setTimeout(function () { document.title = 'AFTER_TOP:still_on=' + location.href; }, 2500);
  }, 500);
}, 5000);
</script></body></html>"#;

const INNER_PAGE: &str = r#"<!doctype html><body><p>inner</p>
<iframe id="n" src="tauri://localhost/nested"></iframe>
<script>
setTimeout(function () {
  var s;
  try { s = document.getElementById('n').contentWindow.location.href; } catch (e) { s = 'cross-origin'; }
  parent.postMessage('inner:nested_href=' + s, '*');
}, 3000);
</script></body>"#;

const XORIGIN_PAGE: &str =
    r#"<!doctype html><p>xorigin</p><script>parent.postMessage('loaded:xorigin-frame','*')</script>"#;

const PROTO_PAGE: &str = r#"<!doctype html><title>proto</title><p>proto</p><script>
try { parent.postMessage('proto-page-loaded:' + location.href, '*'); } catch (e) {}
if (window === top) { document.title = 'TOP_LOADED:' + location.href; }
</script>"#;

fn page(path: &str, port_b: u16) -> Option<String> {
    match path {
        "/surrogate" => Some(SURROGATE_PAGE.to_string()),
        "/nav" => Some(NAV_PAGE.replace("PORTB", &port_b.to_string())),
        "/inner" => Some(INNER_PAGE.to_string()),
        "/xorigin" => Some(XORIGIN_PAGE.to_string()),
        _ => None,
    }
}

fn spawn_server(name: &'static str, listener: TcpListener, port_b: u16) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    match s.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let req = String::from_utf8_lossy(&buf);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                log!("HTTP-HIT server={name} path={path}");
                let (status, body) = match page(&path, port_b) {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", "not found".to_string()),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            });
        }
    });
}

fn main() -> wry::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).cloned().unwrap_or_default();
    let mode = args.get(2).cloned().unwrap_or_default();
    let fix = match mode.as_str() {
        "fix" => true,
        "control" => false,
        _ => panic!("mode must be control or fix"),
    };

    let listener_a = TcpListener::bind("127.0.0.1:0").unwrap();
    let listener_b = TcpListener::bind("127.0.0.1:0").unwrap();
    let port_a = listener_a.local_addr().unwrap().port();
    let port_b = listener_b.local_addr().unwrap().port();
    spawn_server("A", listener_a, port_b);
    spawn_server("B", listener_b, port_b);

    let (url, secs) = match scenario.as_str() {
        "data-literal" => (DATA_LITERAL.to_string(), 6),
        "data" => (DATA_INSTRUMENTED.to_string(), 6),
        "surrogate" => (format!("http://127.0.0.1:{port_a}/surrogate"), 6),
        "nav" => (format!("http://127.0.0.1:{port_a}/nav"), 14),
        _ => panic!("unknown scenario"),
    };
    log!("START scenario={scenario} mode={mode} url={url}");

    log!("STEP event_loop_new");
    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    log!("STEP window_build");
    let window = WindowBuilder::new()
        .with_title("wv-harness")
        .build(&event_loop)
        .unwrap();
    log!("STEP window_built");

    let origins = AppOrigins {
        dev_server: None,
        windows: false,
    };

    let mut builder = WebViewBuilder::new()
        .with_url(url)
        // Tauri gives every webview an ipc handler.
        .with_ipc_handler(|req| {
            log!(
                "IPC-HANDLER-CALLED uri={} body_len={} body={:?}",
                req.uri(),
                req.body().len(),
                req.body()
            )
        })
        .with_document_title_changed_handler(|title| log!("TITLE {title}"))
        // Mirrors tauri-runtime-wry 2.11.2's wrapper around on_navigation;
        // control logs and allows everything.
        .with_navigation_handler(move |url: String| {
            let allow = if fix {
                url.parse::<url::Url>()
                    .map(|u| is_web_page_url(&u, &origins))
                    .unwrap_or(true)
            } else {
                true
            };
            log!(
                "NAV decision={} url={url}",
                if allow { "allow" } else { "deny" }
            );
            allow
        });
    for scheme in ["tauri", "ipc", "asset"] {
        builder = builder.with_custom_protocol(scheme.to_string(), move |_id, req| {
            log!("PROTO-HIT scheme={scheme} uri={}", req.uri());
            Response::builder()
                .header("Content-Type", "text/html")
                .body(Cow::Borrowed(PROTO_PAGE.as_bytes()))
                .unwrap()
        });
    }

    log!("STEP build_gtk");
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        builder.build_gtk(vbox)?
    };

    log!("STEP built");
    if fix {
        use wry::WebViewExtUnix;
        let platform = Platform(webview.webview());
        linux_block::unregister_ipc(&platform);
        log!("FIX applied: unregistered script message handler 'ipc'");
    }

    let deadline = Instant::now() + Duration::from_secs(secs);
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        let _ = proxy.send_event(());
    });
    log!("STEP run");
    event_loop.run(move |event, _, control_flow| {
        let _keep = (&webview, &window);
        *control_flow = ControlFlow::WaitUntil(deadline);
        let due = matches!(event, Event::UserEvent(()))
            || matches!(event, Event::NewEvents(StartCause::ResumeTimeReached { .. }))
            || Instant::now() >= deadline;
        if due {
            log!("SURVIVED deadline reached, exiting 0");
            *control_flow = ControlFlow::ExitWithCode(0);
        }
    });
}
