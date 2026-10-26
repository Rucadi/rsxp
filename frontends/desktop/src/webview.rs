// SPDX-License-Identifier: GPL-2.0-or-later
//! Native HTML plugin views.
//!
//! The main desktop window remains eframe-owned. On Linux the view is built
//! through GTK (`WebViewBuilder::build_gtk`, the builder form of
//! `WebView::new_gtk`) instead of `WebViewBuilder::new` + `build_as_child`:
//! the latter only works with an Xlib handle and fails with
//! `UnsupportedWindowHandle` on Wayland, because WebKitGTK cannot be
//! reparented into an arbitrary winit surface. The GTK path works on both X11
//! and Wayland, at the cost of hosting the view in its own toplevel window.
//! GTK events are pumped from the egui update loop, as wry recommends for
//! winit hosts. macOS and Windows use a hidden child only as a Wry opener and
//! present the actual page in Wry's native popup window; other desktop platforms keep
//! the embedded child webview. The browser frontend uses a DOM iframe for the
//! equivalent operation.

use eframe::Frame;
use std::sync::{Arc, Mutex};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use wry::NewWindowResponse;
use wry::{WebView, WebViewBuilder};

#[cfg(not(target_os = "linux"))]
use wry::{dpi::LogicalPosition, dpi::LogicalSize, Rect};

#[cfg(target_os = "linux")]
use gtk::prelude::*;

/// Size of the standalone window or popup, and the embedded panel fallback.
const VIEW_SIZE: (f64, f64) = (720.0, 560.0);
/// Position of the embedded panel inside the dashboard window (non-Linux).
#[cfg(not(target_os = "linux"))]
const VIEW_POSITION: (f64, f64) = (320.0, 72.0);

/// Applies the NVIDIA/Wayland WebKitGTK workaround before GTK or WebKit is
/// initialized. WebKitGTK can otherwise submit a DMA-BUF surface with
/// explicit sync enabled but without an acquire point, which makes the
/// compositor disconnect the application with Wayland protocol error 71.
pub fn prepare_runtime() {
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland");
        let nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists()
            || std::path::Path::new("/sys/module/nvidia").exists();
        if wayland && nvidia && std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
            // SAFETY: this runs on the desktop main thread before GTK and
            // WebKit are initialized, and only changes this process's
            // renderer configuration.
            unsafe { std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1") };
            eprintln!("[PluginWebView] enabled NVIDIA Wayland WebKitGTK explicit-sync workaround");
        }
    }
}

pub struct PluginWebView {
    // Retained solely to keep the native child webview alive while the plugin
    // owns the view. Wry drives the child through the host window/event loop.
    #[allow(dead_code)]
    view: WebView,
    // On Linux the webview lives in this GTK toplevel; dropping the last
    // reference closes it. Retention is the only purpose of the field.
    #[cfg(target_os = "linux")]
    #[allow(dead_code)]
    window: Option<gtk::Window>,
    requests: Arc<Mutex<Vec<PluginWebViewRequest>>>,
}

#[cfg(target_os = "linux")]
impl Drop for PluginWebView {
    fn drop(&mut self) {
        // GTK keeps references to toplevel windows independently of the Rust
        // wrapper. Close it explicitly before WebKit is released; otherwise
        // replacing a plugin view can leave the old window visible as a black
        // surface while the new WebView is being initialized.
        if let Some(window) = self.window.take() {
            window.close();
        }
    }
}

#[derive(Debug, Clone)]
pub enum PluginWebViewRequest {
    Api {
        id: u64,
        method: String,
        target: String,
    },
    Asset {
        id: u64,
        category: String,
        name: String,
    },
}

impl PluginWebView {
    pub fn open(frame: &Frame, title: &str, html: &str) -> Result<Self, String> {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_ipc = Arc::clone(&requests);
        let document = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title><script>{}</script></head><body>{}</body></html>",
            escape_html(title),
            native_bridge_script(),
            html
        );
        let builder = WebViewBuilder::new()
            .with_ipc_handler(move |request| {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(request.body()) else {
                    return;
                };
                let Some(kind) = value.get("kind").and_then(serde_json::Value::as_str) else {
                    return;
                };
                let Some(id) = value.get("id").and_then(serde_json::Value::as_u64) else {
                    return;
                };
                let message = match kind {
                    "api" => PluginWebViewRequest::Api {
                        id,
                        method: value
                            .get("method")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("GET")
                            .to_owned(),
                        target: value
                            .get("target")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("/")
                            .to_owned(),
                    },
                    "asset" => PluginWebViewRequest::Asset {
                        id,
                        category: value
                            .get("category")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: value
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    },
                    _ => return,
                };
                if let Ok(mut requests) = requests_for_ipc.lock() {
                    requests.push(message);
                }
            });

        #[cfg(target_os = "linux")]
        {
            let _ = frame;
            Self::open_linux(title, builder.with_html(document), requests)
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let window = frame
                .winit_window()
                .ok_or_else(|| "the desktop window is not available".to_owned())?;
            Self::open_native_popup(window, builder, &document, requests)
        }
        #[cfg(not(target_os = "linux"))]
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let window = frame
                .winit_window()
                .ok_or_else(|| "the desktop window is not available".to_owned())?;
            let view = builder
                .with_html(document)
                .with_bounds(bounds(VIEW_POSITION, VIEW_SIZE))
                .build_as_child(window)
                .map_err(|error| format!("could not open plugin webview: {error}"))?;
            Ok(Self { view, requests })
        }
    }

    /// Wry can create a real native popup for `window.open` on macOS and Windows.
    /// Keep a hidden child webview as the opener so the popup shares the same
    /// configuration and IPC handler, then write the plugin document into the
    /// popup. The popup is owned by Wry and gets normal title-bar controls and
    /// resize behavior.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn open_native_popup(
        window: &winit::window::Window,
        builder: WebViewBuilder<'_>,
        document: &str,
        requests: Arc<Mutex<Vec<PluginWebViewRequest>>>,
    ) -> Result<Self, String> {
        let launcher = format!(
            "<!doctype html><html><head><script>{}</script></head><body><script>\n                const popup = window.open('about:blank', '_blank', 'width={},height={},resizable=yes');\n                if (popup) {{\n                    popup.document.open();\n                    popup.document.write({});\n                    popup.document.close();\n                }}\n            </script></body></html>",
            native_bridge_script(),
            VIEW_SIZE.0 as i32,
            VIEW_SIZE.1 as i32,
            javascript_string(document),
        );
        let view = builder
            .with_html(launcher)
            .with_visible(false)
            .with_new_window_req_handler(|_, _| NewWindowResponse::Allow)
            .build_as_child(window)
            .map_err(|error| format!("could not open plugin webview: {error}"))?;
        Ok(Self { view, requests })
    }

    /// Builds the webview inside an expanding GTK `Box`, which wry supports on
    /// both X11 and Wayland (unlike `build_as_child`, which requires an Xlib
    /// handle). The box makes the WebKit widget follow the toplevel allocation
    /// when the user resizes the webview window.
    #[cfg(target_os = "linux")]
    fn open_linux(
        title: &str,
        builder: WebViewBuilder<'_>,
        requests: Arc<Mutex<Vec<PluginWebViewRequest>>>,
    ) -> Result<Self, String> {
        use wry::WebViewBuilderExtUnix;

        gtk::init()
            .map_err(|error| format!("could not initialize GTK for plugin webview: {error}"))?;
        let toplevel = gtk::Window::new(gtk::WindowType::Toplevel);
        toplevel.set_title(title);
        toplevel.set_default_size(VIEW_SIZE.0 as i32, VIEW_SIZE.1 as i32);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        toplevel.add(&container);
        container.show_all();
        toplevel.show_all();
        let view = builder
            .build_gtk(&container)
            .map_err(|error| format!("could not open plugin webview: {error}"))?;
        Ok(Self {
            view,
            window: Some(toplevel),
            requests,
        })
    }

    pub fn take_requests(&self) -> Vec<PluginWebViewRequest> {
        self.requests
            .lock()
            .map(|mut requests| std::mem::take(&mut *requests))
            .unwrap_or_default()
    }

    pub fn respond(&self, id: u64, status: u16, body: &str) -> Result<(), String> {
        let message = serde_json::json!({"id": id, "status": status, "body": body});
        let script = format!(
            "window.__fanplay_plugin_response({});",
            serde_json::to_string(&message).map_err(|error| error.to_string())?
        );
        self.view
            .evaluate_script(&script)
            .map_err(|error| format!("could not respond to plugin webview: {error}"))
    }

    pub fn respond_asset(&self, id: u64, data_url: Option<&str>) -> Result<(), String> {
        let body = serde_json::json!({"src": data_url});
        self.respond(id, 200, &body.to_string())
    }

    /// Keeps child webviews aligned with the dashboard after its window is
    /// resized. GTK views use their expanding container instead.
    pub fn resize(&self, frame: &Frame) {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let _ = frame;
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        if let Some(window) = frame.winit_window() {
            let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
            let width = (logical_size.width - VIEW_POSITION.0).max(1.0);
            let height = (logical_size.height - VIEW_POSITION.1).max(1.0);
            let _ = self.view.set_bounds(bounds(VIEW_POSITION, (width, height)));
        }
    }

    pub fn pump(&self) {
        #[cfg(target_os = "linux")]
        for _ in 0..64 {
            if !gtk::events_pending() {
                break;
            }
            gtk::main_iteration_do(false);
        }
    }
}

fn native_bridge_script() -> &'static str {
    r#"
      (() => {
        // On macOS the visible page is a native popup created by window.open,
        // while the hidden opener owns Wry's IPC handler and the response
        // evaluator. Delegate popup calls to that opener so request replies
        // resolve in the same JavaScript context that created them.
        let openerBridge = null;
        try {
          if (window.opener && window.opener.FANPLAY && window.opener.FANPLAY.__native) {
            openerBridge = window.opener.FANPLAY;
          }
        } catch (_) {}
        if (openerBridge) {
          window.FANPLAY = window.FANPLAY || {};
          window.FANPLAY.api = (...args) => openerBridge.api(...args);
          window.FANPLAY.http = window.FANPLAY.api;
          window.FANPLAY.asset = (...args) => openerBridge.asset(...args);
          const nativeFetch = window.fetch.bind(window);
          window.fetch = (input, init = {}) => {
            const raw = typeof input === 'string' ? input : input.url;
            if (!raw.startsWith('/v1/')) return nativeFetch(input, init);
            const method = String(init.method || (typeof input === 'string' ? 'GET' : input.method) || 'GET').toUpperCase();
            return window.FANPLAY.api(method, raw, init.body || '');
          };
          return;
        }

        const pending = new Map();
        let next = 1;
        window.FANPLAY = window.FANPLAY || {};
        window.FANPLAY.__native = true;
        window.FANPLAY.api = (method, target, body = '') => new Promise((resolve, reject) => {
          const id = next++;
          pending.set(id, { kind: 'api', resolve, reject });
          window.ipc.postMessage(JSON.stringify({ kind: 'api', id, method, target, body }));
        });
        window.FANPLAY.http = window.FANPLAY.api;
        window.FANPLAY.asset = (category, name) => new Promise((resolve, reject) => {
          const id = next++;
          pending.set(id, { kind: 'asset', resolve, reject });
          window.ipc.postMessage(JSON.stringify({ kind: 'asset', id, category, name }));
        });
        window.__fanplay_plugin_response = message => {
          const request = pending.get(message.id);
          if (!request) return;
          pending.delete(message.id);
          if (request.kind === 'asset') {
            try { request.resolve(JSON.parse(message.body || '{}').src || null); }
            catch (_) { request.resolve(null); }
            return;
          }
          request.resolve(new Response(message.body || '', {
            status: message.status || 500,
            headers: { 'content-type': 'application/json' },
          }));
        };
        const nativeFetch = window.fetch.bind(window);
        window.fetch = (input, init = {}) => {
          const raw = typeof input === 'string' ? input : input.url;
          if (!raw.startsWith('/v1/')) return nativeFetch(input, init);
          const method = String(init.method || (typeof input === 'string' ? 'GET' : input.method) || 'GET').toUpperCase();
          return window.FANPLAY.api(method, raw, init.body || '');
        };
      })();
    "#
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn javascript_string(value: &str) -> String {
    // JSON is valid JavaScript string syntax. Escape HTML-sensitive characters
    // as well so a plugin containing `</script>` cannot terminate the launcher
    // script before the popup receives its document.
    serde_json::to_string(value)
        .expect("serializing a Rust string to JSON cannot fail")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

#[cfg(not(target_os = "linux"))]
fn bounds(position: (f64, f64), size: (f64, f64)) -> Rect {
    Rect {
        position: LogicalPosition::new(position.0, position.1).into(),
        size: LogicalSize::new(size.0, size.1).into(),
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
