// SPDX-License-Identifier: GPL-2.0-or-later
//! cargo-mobile2 shell: one tao window and one system WebView on every target.
use crate::{engine, platform};
use serde_json::{json, Value};
use std::{cell::RefCell, rc::Rc};
use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::WindowBuilder,
};
use wry::{
    http::{header, Request, Response},
    WebView, WebViewBuilder,
};

const UI: &str = include_str!(concat!(env!("OUT_DIR"), "/ui.html"));

/// IPC arrives on a WebView thread on Android. All engine and MRI work must
/// run on the event-loop thread, so requests are forwarded as user events.
enum AppEvent {
    Ipc { id: Value, request: Value },
}

#[cfg(target_os = "android")]
fn init_logging() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("RSXP"),
    );
}

#[cfg(not(target_os = "android"))]
fn init_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn stop_unwind<F: FnOnce() -> T, T>(f: F) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("attempt to unwind out of Rust: {error:?}");
            std::process::abort()
        }
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn _start_app() {
    stop_unwind(run);
}

/// Entry point used by the cargo-mobile2 iOS project and Android `WryActivity`.
#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn start_app() {
    _start_app();
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn start_app() {
    // These identifiers mirror `org.rsxp.mobile` and the `Rust` object that
    // cargo-mobile2's wry template generates.
    tao::android_binding!(
        org_rsxp,
        mobile,
        WryActivity,
        wry::android_setup,
        _start_app
    );
    wry::android_binding!(org_rsxp, mobile);
}

fn launch_url() -> String {
    let mut url = String::from("rsxp://app/index.html");
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.iter().any(|argument| argument == "--demo") {
        url.push_str("?demo=1");
    } else if let Some(index) = arguments.iter().position(|argument| argument == "--game") {
        if let Some(game) = arguments.get(index + 1) {
            let encoded: String = game
                .bytes()
                .map(|byte| {
                    if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                        (byte as char).to_string()
                    } else {
                        format!("%{byte:02X}")
                    }
                })
                .collect();
            url.push_str("?game=");
            url.push_str(&encoded);
        }
    }
    url
}

fn build_webview(
    event_loop: &EventLoopWindowTarget<AppEvent>,
    proxy: EventLoopProxy<AppEvent>,
) -> anyhow::Result<(tao::window::Window, Rc<RefCell<Option<WebView>>>)> {
    let window = WindowBuilder::new().with_title("RSXP").build(event_loop)?;

    let webview: Rc<RefCell<Option<WebView>>> = Rc::new(RefCell::new(None));
    let builder = WebViewBuilder::new()
        .with_url(launch_url())
        .with_custom_protocol("rsxp".into(), move |_id, request: Request<Vec<u8>>| {
            let path = request.uri().path().trim_start_matches('/');
            if path.is_empty() || path == "index.html" {
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                    .body(UI.as_bytes().into())
                    .unwrap_or_default()
            } else {
                Response::builder()
                    .status(404)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(b"Not found".as_slice().into())
                    .unwrap_or_default()
            }
        })
        .with_ipc_handler(move |message: Request<String>| {
            let payload: Value = serde_json::from_str(message.body()).unwrap_or(Value::Null);
            let id = payload.get("id").cloned().unwrap_or(Value::Null);
            let request = payload.get("request").cloned().unwrap_or(Value::Null);
            let _ = proxy.send_event(AppEvent::Ipc { id, request });
        });
    *webview.borrow_mut() = Some(builder.build(&window)?);
    Ok((window, webview))
}

pub fn run() {
    init_logging();
    match platform::library_dir()
        .and_then(|library| platform::ruby_runtime().map(|runtime| (library, runtime)))
    {
        Ok((library, runtime)) => {
            if let Err(error) = engine::configure(library, Some(runtime)) {
                eprintln!("RSXP configuration failed: {error}");
            }
        }
        Err(error) => eprintln!("RSXP configuration failed: {error}"),
    }

    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let mut window = None;
    let mut webview: Option<Rc<RefCell<Option<WebView>>>> = None;
    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => match build_webview(event_loop, proxy.clone()) {
                Ok((new_window, new_webview)) => {
                    window = Some(new_window);
                    webview = Some(new_webview);
                }
                Err(error) => eprintln!("could not create the RSXP window: {error}"),
            },
            Event::UserEvent(AppEvent::Ipc { id, request }) => {
                let response = engine::request(request);
                let response = serde_json::to_string(&response)
                    .unwrap_or_else(|_| json!({"error": "response encoding failed"}).to_string());
                let script = format!("globalThis.__rsxpResponse({id}, {response});");
                if let Some(slot) = webview.as_ref() {
                    if let Some(webview) = slot.borrow().as_ref() {
                        let _ = webview.evaluate_script(&script);
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested { .. },
                ..
            } => {
                webview.take();
                window.take();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
