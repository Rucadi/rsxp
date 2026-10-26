// fanplay:name = Dynamic Plugin Demo
// fanplay:capabilities = ui.webview
// fanplay:panel = demo,Dynamic Plugin
// A single Rhai file can be imported or pasted without a companion manifest.
// The filename supplies the plugin ID when imported from disk.

fn on_activate() {
    state_set("clicks", 0);
}

fn build_ui() {
    let clicks = state_get("clicks");
    [
        ui_label("dynamic-demo.description", "Loaded dynamically from one Rhai file."),
        ui_button("dynamic-demo.increment", "Click me"),
        ui_button("dynamic-demo.webview", "Open HTML GUI"),
        ui_label("dynamic-demo.count", "Clicks: " + clicks.to_string())
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "dynamic-demo.increment" {
        state_set("clicks", state_get("clicks") + 1);
    }
    if widget_id == "dynamic-demo.webview" {
        let html = "<!doctype html><meta charset='utf-8'><title>Dynamic Plugin</title>"
            + "<style>body{font:16px sans-serif;padding:2rem;background:#20252b;color:#f4f7fa}button{padding:.6rem 1rem}</style>"
            + "<h1>Dynamic Plugin</h1><p>This GUI is supplied entirely by Rhai.</p>"
            + "<button onclick=\"fetch('/v1/game/status').then(r=>r.text()).then(t=>document.querySelector('pre').textContent=t).catch(e=>document.querySelector('pre').textContent=e)\">Query game API</button><pre></pre>";
        webview_open("dynamic-demo", "Dynamic Plugin GUI", html);
    }
}

fn on_operation_result(_request_id, _result) { }
fn on_shutdown() { }
