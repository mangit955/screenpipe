//! WebView2 check of an embedded-browser navigation filter.
//!
//! Usage: wincheck <scenario> <mode> <out_dir>
//!   mode filter  : filter block applied right after the webview is built
//!   mode control : no filter block (web messages enabled, no frame filter)
//!   mode late    : filter block applied 1 s after start
//!   mode nonav   : filter block applied, no top-level navigation handler
//!
//! Every webview gets an ipc handler and the `tauri`, `ipc`, `asset` custom
//! protocols, as Tauri builds every webview.

use std::{
    borrow::Cow,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use webview2_com::{
    take_pwstr, CapturePreviewCompletedHandler, FrameCreatedEventHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, ProcessFailedEventHandler, WebMessageReceivedEventHandler,
};
use windows::core::Interface;
use wry::{WebContext, WebViewBuilder, WebViewExtWindows};

// ===== BEGIN VERBATIM COPY: AppOrigins, is_web_page_url, is_windows_app_url =====
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

// ===== END VERBATIM COPY =====

fn origins() -> AppOrigins {
    AppOrigins {
        dev_server: None,
        windows: true,
    }
}

/// Stand-in for Tauri's `PlatformWebview` so the copied block below is unchanged:
/// `platform.controller()` returns wry's controller (`WebViewExtWindows::controller`).
struct Platform(ICoreWebView2Controller);
impl Platform {
    fn controller(&self) -> ICoreWebView2Controller {
        self.0.clone()
    }
}

fn apply_filter_block(platform: &Platform) {
    let origins = origins();
// ===== BEGIN VERBATIM COPY: #[cfg(windows)] block of add_child_webview =====
        #[cfg(windows)]
        unsafe {
            use webview2_com::{take_pwstr, NavigationStartingEventHandler};
            if let Ok(webview) = platform.controller().CoreWebView2() {
                if let Ok(settings) = webview.Settings() {
                    let _ = settings.SetIsWebMessageEnabled(false);
                }
                // `on_navigation` only sees top-level loads on Windows. WebKit
                // also sends iframes through it, so filter them here to match.
                let handler = NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    // A URI that can't be read or parsed is not allowed.
                    let mut uri = windows_core::PWSTR::null();
                    let allowed = args.Uri(&mut uri).is_ok()
                        && url::Url::parse(&take_pwstr(uri))
                            .is_ok_and(|url| is_web_page_url(&url, &origins));
                    args.SetCancel(!allowed)
                }));
                let mut token = 0;
                let _ = webview.add_FrameNavigationStarting(&handler, &mut token);
            }
        }
// ===== END VERBATIM COPY =====
    log("[harness] filter block applied (SetIsWebMessageEnabled(false) + FrameNavigationStarting filter)");
}

// ---------------------------------------------------------------- logging

static LOG: OnceLock<Mutex<File>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static PROTOCOL_HITS: AtomicUsize = AtomicUsize::new(0);
static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NAV_IDS: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());

fn log(line: impl AsRef<str>) {
    let ms = START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);
    let line = format!("[{ms:>5}ms] {}", line.as_ref());
    {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    if let Some(file) = LOG.get() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

fn short(uri: &str) -> String {
    if uri.len() > 160 {
        let mut end = 160;
        while !uri.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...(len {})", &uri[..end], uri.len())
    } else {
        uri.to_string()
    }
}

fn record(kind: &str, uri: &str, cancelled: bool) {
    RECORDS.lock().unwrap().push(format!(
        "{kind}:{}:{}",
        if cancelled { "CANCEL" } else { "allow" },
        short(uri)
    ));
}

fn remember(id: u64, uri: &str) {
    NAV_IDS.lock().unwrap().push((id, short(uri)));
}

fn lookup(id: u64) -> String {
    NAV_IDS
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(i, _)| *i == id)
        .map(|(_, u)| u.clone())
        .unwrap_or_default()
}

fn web_error_name(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> &'static str {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN => "UNKNOWN",
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED => "OPERATION_CANCELED",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "HOST_NAME_NOT_RESOLVED",
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "CANNOT_CONNECT",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED => "CONNECTION_ABORTED",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "CONNECTION_RESET",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "DISCONNECTED",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "SERVER_UNREACHABLE",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "TIMEOUT",
        COREWEBVIEW2_WEB_ERROR_STATUS_UNEXPECTED_ERROR => "UNEXPECTED_ERROR",
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => "REDIRECT_FAILED",
        _ => "OTHER",
    }
}

// ---------------------------------------------------------------- test pages

fn sample_pdf() -> Vec<u8> {
    let content = "1 0 0 rg 72 560 468 120 re f\nBT /F1 40 Tf 1 1 1 rg 90 605 Td (PDF RENDERED OK) Tj ET\nBT /F1 24 Tf 0 0 0 rg 72 500 Td (wincheck sample page) Tj ET\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for off in offsets {
        tail.push_str(&format!("{off:010} 00000 n \n"));
    }
    tail.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    pdf.extend_from_slice(tail.as_bytes());
    pdf
}

fn html(title: &str, body: &str) -> (&'static str, Vec<u8>) {
    (
        "text/html; charset=utf-8",
        format!(
            "<!doctype html><html><head><meta charset=utf-8><title>{title}</title><style>\
             body{{font:14px sans-serif;background:#eef}} iframe,object,embed{{width:300px;height:140px;border:2px solid #333;margin:4px;background:#fff}}\
             </style></head><body><b>{title}</b><br>{body}</body></html>"
        )
        .into_bytes(),
    )
}

fn top_nav_page(target: &str) -> String {
    format!(
        "<p>navigating to {target}</p><script>setTimeout(()=>{{fetch('/log?s10_navigating');\
         location.href={target:?};setTimeout(()=>fetch('/log?s10_still_on_page_after_2s'),2000)}},500)</script>"
    )
}

fn route(path: &str) -> Option<(&'static str, Vec<u8>)> {
    let path = path.split('?').next().unwrap_or(path);
    Some(match path {
        "/s1.html" => html("s1 iframe", r#"<iframe src="http://tauri.localhost/iframe"></iframe>"#),
        "/s2.html" => html("s2 nested", r#"<iframe src="/inner.html" style="width:600px;height:300px"></iframe>"#),
        "/inner.html" => html("inner", r#"<iframe src="http://tauri.localhost/nested"></iframe>"#),
        "/s3.html" => html("s3 object", r#"<object data="http://tauri.localhost/object" type="text/html"></object>"#),
        "/s4.html" => html("s4 embed", r#"<embed src="http://tauri.localhost/embed" type="text/html">"#),
        "/s5.html" => html(
            "s5 srcdoc",
            r#"<iframe style="width:600px;height:300px" srcdoc="<p>srcdoc frame</p><iframe src='http://tauri.localhost/srcdoc'></iframe>"></iframe>"#,
        ),
        "/s6.html" => html(
            "s6 tauri.example.com and userinfo",
            r#"<iframe src="http://tauri.example.com/"></iframe><iframe src="http://tauri.localhost@example.com/"></iframe>"#,
        ),
        "/s7.html" => html(
            "s7 must stay allowed",
            r#"<iframe src="https://tauri.app/"></iframe><iframe src="https://ipc.org/"></iframe>
<iframe src="https://example.com/"></iframe><iframe src="about:blank"></iframe>
<iframe src="data:text/html,hi"></iframe><iframe id="b"></iframe>
<script>const u=URL.createObjectURL(new Blob(['<p>blob frame ok</p>'],{type:'text/html'}));
fetch('/log?blob_url='+encodeURIComponent(u));document.getElementById('b').src=u;</script>"#,
        ),
        "/s8a.html" => html(
            "s8a pdf embed",
            r#"<embed src="/sample.pdf" type="application/pdf" style="width:900px;height:560px">"#,
        ),
        "/s8b.html" => html(
            "s8b pdf iframe",
            r#"<iframe src="/sample.pdf" style="width:900px;height:560px"></iframe>"#,
        ),
        "/s10a.html" => html("s10a", &top_nav_page("http://tauri.localhost/")),
        "/s10b.html" => html("s10b", &top_nav_page("http://tauri.example.com/")),
        "/s10c.html" => html("s10c", &top_nav_page("http://tauri.localhost@example.com/")),
        "/sample.pdf" => ("application/pdf", sample_pdf()),
        "/log" => ("text/plain", b"ok".to_vec()),
        _ => return None,
    })
}

fn handle(mut stream: TcpStream) {
    let Ok(clone) = stream.try_clone() else { return };
    let mut reader = BufReader::new(clone);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
    log(format!("[server] {}", short(request_line.trim())));
    let (status, ctype, body) = match route(&path) {
        Some((ctype, body)) => ("200 OK", ctype, body),
        None => ("404 Not Found", "text/plain", b"not found".to_vec()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

// ---------------------------------------------------------------- observers

/// Observers registered after wry's handlers and after the filter block, so the
/// starting observers see the effective `Cancel` value set by earlier handlers.
unsafe fn install_observers(core: &ICoreWebView2) -> windows::core::Result<()> {
    let mut token = 0i64;

    core.add_FrameNavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut uri = windows_core::PWSTR::null();
            let _ = args.Uri(&mut uri);
            let uri = take_pwstr(uri);
            let mut id = 0u64;
            let _ = args.NavigationId(&mut id);
            let mut cancel = windows_core::BOOL::default();
            let _ = args.Cancel(&mut cancel);
            let computed = url::Url::parse(&uri).is_ok_and(|u| is_web_page_url(&u, &origins()));
            remember(id, &uri);
            record("frame", &uri, cancel.as_bool());
            log(format!(
                "[frame-nav-starting] id={id} effective={} is_web_page_url={computed} uri={}",
                if cancel.as_bool() { "CANCELLED" } else { "allowed" },
                short(&uri)
            ));
            Ok(())
        })),
        &mut token,
    )?;

    core.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut uri = windows_core::PWSTR::null();
            let _ = args.Uri(&mut uri);
            let uri = take_pwstr(uri);
            let mut id = 0u64;
            let _ = args.NavigationId(&mut id);
            let mut cancel = windows_core::BOOL::default();
            let _ = args.Cancel(&mut cancel);
            remember(id, &uri);
            record("top", &uri, cancel.as_bool());
            log(format!(
                "[top-nav-starting] id={id} effective={} uri={}",
                if cancel.as_bool() { "CANCELLED" } else { "allowed" },
                short(&uri)
            ));
            Ok(())
        })),
        &mut token,
    )?;

    for kind in ["frame", "top"] {
        let handler = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut id = 0u64;
            let _ = args.NavigationId(&mut id);
            let mut ok = windows_core::BOOL::default();
            let _ = args.IsSuccess(&mut ok);
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
            let _ = args.WebErrorStatus(&mut status);
            log(format!(
                "[{kind}-nav-completed] id={id} success={} web_error_status={}({}) uri={}",
                ok.as_bool(),
                web_error_name(status),
                status.0,
                lookup(id)
            ));
            Ok(())
        }));
        if kind == "frame" {
            core.add_FrameNavigationCompleted(&handler, &mut token)?;
        } else {
            core.add_NavigationCompleted(&handler, &mut token)?;
        }
    }

    if let Ok(core4) = core.cast::<ICoreWebView2_4>() {
        core4.add_FrameCreated(
            &FrameCreatedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let name = args
                    .Frame()
                    .and_then(|frame| {
                        let mut name = windows_core::PWSTR::null();
                        frame.Name(&mut name).map(|_| take_pwstr(name))
                    })
                    .unwrap_or_else(|e| format!("<err {e}>"));
                log(format!("[frame-created] name={name:?}"));
                Ok(())
            })),
            &mut token,
        )?;
    }

    core.add_ProcessFailed(
        &ProcessFailedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            let _ = args.ProcessFailedKind(&mut kind);
            log(format!("[process-failed] kind={}", kind.0));
            Ok(())
        })),
        &mut token,
    )?;

    core.add_WebMessageReceived(
        &WebMessageReceivedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut source = windows_core::PWSTR::null();
            let _ = args.Source(&mut source);
            log(format!("[webmessage-observer] source={}", short(&take_pwstr(source))));
            Ok(())
        })),
        &mut token,
    )?;
    Ok(())
}

unsafe fn capture_preview(core: &ICoreWebView2, path: PathBuf) {
    let Some(stream) = windows::Win32::UI::Shell::SHCreateMemStream(None) else {
        log("[capture] SHCreateMemStream failed");
        return;
    };
    let reader = stream.clone();
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        let mut data = Vec::new();
        let _ = reader.Seek(0, windows::Win32::System::Com::STREAM_SEEK_SET, None);
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let mut read = 0u32;
            let hr = reader.Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut read));
            if hr.is_err() || read == 0 {
                break;
            }
            data.extend_from_slice(&buf[..read as usize]);
        }
        let written = fs::write(&path, &data);
        log(format!(
            "[capture] CapturePreview result={result:?} bytes={} file={} write={written:?}",
            data.len(),
            path.display()
        ));
        Ok(())
    }));
    if let Err(e) = core.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &handler) {
        log(format!("[capture] CapturePreview call failed: {e}"));
    }
}

fn summary(name: &str) {
    let records = RECORDS.lock().unwrap().clone();
    log(format!(
        "SUMMARY run={name} protocol_hits={} navigations={records:?}",
        PROTOCOL_HITS.load(Ordering::SeqCst)
    ));
}

// ---------------------------------------------------------------- main

fn main() {
    START.get_or_init(Instant::now);
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).cloned().unwrap_or_else(|| "s1".into());
    let mode = args.get(2).cloned().unwrap_or_else(|| "filter".into());
    let out_dir = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| "out".into()));
    let name = format!("{scenario}-{mode}");
    fs::create_dir_all(&out_dir).unwrap();
    let _ = LOG.set(Mutex::new(File::create(out_dir.join(format!("{name}.log"))).unwrap()));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(format!("[PANIC] {info}"));
        default_hook(info);
    }));

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle(stream));
        }
    });
    let base = format!("http://127.0.0.1:{port}");

    let start_url = match scenario.as_str() {
        "s9" => "data:text/html,<script>window.chrome.webview.postMessage('x')</script>".to_string(),
        "s9late" => format!(
            "data:text/html,<p>s9late</p><script>setTimeout(()=>{{try{{window.chrome.webview.postMessage('x');\
             fetch('{base}/log?s9late_postMessage_returned',{{mode:'no-cors'}})}}catch(e){{\
             fetch('{base}/log?s9late_postMessage_threw='+encodeURIComponent(String(e)),{{mode:'no-cors'}})}}}},2500)</script>"
        ),
        "s8c" => format!("{base}/sample.pdf"),
        other => format!("{base}/{other}.html"),
    };

    let apply_now = matches!(mode.as_str(), "filter" | "nonav");
    let apply_late = mode == "late";
    let nav_handler = mode != "nonav";
    log(format!(
        "[harness] run={name} start_url={} apply_now={apply_now} apply_late={apply_late} nav_handler={nav_handler} ipc_handler=true",
        short(&start_url)
    ));

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(format!("wincheck {name}"))
        .with_inner_size(LogicalSize::new(980.0, 700.0))
        .with_position(LogicalPosition::new(10.0, 10.0))
        .with_always_on_top(true)
        .build(&event_loop)
        .unwrap();

    let udf = std::env::temp_dir().join("wincheck-udf").join(&name);
    let mut context = WebContext::new(Some(udf));
    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(start_url)
        .with_ipc_handler(|request| {
            log(format!(
                "[ipc-handler] uri={} body={}",
                request.uri(),
                short(request.body())
            ))
        });
    for protocol in ["tauri", "ipc", "asset"] {
        builder = builder.with_custom_protocol(protocol.into(), move |_id, request| {
            PROTOCOL_HITS.fetch_add(1, Ordering::SeqCst);
            log(format!("[protocol:{protocol}] HIT uri={}", request.uri()));
            let body = format!(
                "<html><body style='background:#c00;color:#fff;font:20px sans-serif'>PROTOCOL {protocol} HIT {}</body></html>",
                request.uri()
            );
            http::Response::builder()
                .header("Content-Type", "text/html")
                .body(Cow::Owned(body.into_bytes()))
                .unwrap()
        });
    }
    if nav_handler {
        // Mirrors tauri-runtime-wry 2.11.2's wrapper around `on_navigation`.
        let nav_origins = origins();
        builder = builder.with_navigation_handler(move |url| {
            let allowed = url
                .parse::<url::Url>()
                .map(|u| is_web_page_url(&u, &nav_origins))
                .unwrap_or(true);
            log(format!(
                "[nav-handler] decision={} uri={}",
                if allowed { "allow" } else { "CANCEL" },
                short(&url)
            ));
            allowed
        });
    }

    let webview = match builder.build(&window) {
        Ok(webview) => webview,
        Err(e) => {
            log(format!("[harness] build failed: {e}"));
            std::process::exit(3);
        }
    };
    let platform = Platform(webview.controller());
    if apply_now {
        apply_filter_block(&platform);
    }
    let core = webview.webview();
    if let Err(e) = unsafe { install_observers(&core) } {
        log(format!("[harness] observers failed: {e}"));
    }

    let started = Instant::now();
    let late_at = Duration::from_millis(1000);
    let capture_at = Duration::from_millis(5000);
    let exit_at = Duration::from_millis(8000);
    let mut applied_late = false;
    let mut captured = false;
    let png = out_dir.join(format!("{name}-capture.png"));

    event_loop.run(move |event, _, control_flow| {
        let _keep = (&window, &webview, &context);
        let elapsed = started.elapsed();
        if apply_late && !applied_late && elapsed >= late_at {
            applied_late = true;
            apply_filter_block(&platform);
        }
        if !captured && elapsed >= capture_at {
            captured = true;
            unsafe { capture_preview(&core, png.clone()) };
        }
        if elapsed >= exit_at {
            summary(&name);
            log("[harness] clean exit 0");
            std::process::exit(0);
        }
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            summary(&name);
            std::process::exit(0);
        }
    });
}
