// SPDX-License-Identifier: GPL-2.0-or-later
import UIKit
import WebKit
import QuartzCore
import UniformTypeIdentifiers

/// A view backed directly by `CAMetalLayer`, so wgpu can present without a
/// CPU framebuffer readback.
final class MetalGameView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }
    var metalLayer: CAMetalLayer { layer as! CAMetalLayer }
}

final class BundleScheme: NSObject, WKURLSchemeHandler {
    let root: URL
    init(root: URL) { self.root = root.resolvingSymlinksInPath() }
    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard let requestURL = task.request.url, requestURL.host == "app" else {
            task.didFailWithError(URLError(.badURL)); return
        }
        let path = requestURL.path == "/" ? "index.html" : String(requestURL.path.dropFirst())
        let file = root.appendingPathComponent(path).resolvingSymlinksInPath()
        guard file.path.hasPrefix(root.path + "/"), let data = try? Data(contentsOf: file) else {
            task.didFailWithError(URLError(.fileDoesNotExist)); return
        }
        let mime = ["html":"text/html", "js":"text/javascript", "css":"text/css"][file.pathExtension] ?? "application/octet-stream"
        task.didReceive(HTTPURLResponse(url: requestURL, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": mime, "Content-Length": String(data.count), "Access-Control-Allow-Origin": "*"])!)
        task.didReceive(data); task.didFinish()
    }
    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

final class GameViewController: UIViewController, WKUIDelegate, WKNavigationDelegate, UIDocumentPickerDelegate {
    private var webView: WKWebView!
    private let gameImage = UIImageView()
    private let gameView = MetalGameView()
    private let library = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Games", isDirectory: true)
    private var importing = false
    private var frameVisible = true
    private var usingGPU = false
    private var surfaceAttached = false
    private var gameViewport = CGRect.zero
    override func viewDidLoad() {
        super.viewDidLoad()
        let runtime = Bundle.main.resourceURL!.appendingPathComponent("runtime").path
        let success = library.path.withCString { path in runtime.withCString { rsxp_mobile_configure(path,$0) } }
        guard success == 1 else { fatalError("Could not configure game library") }
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(BundleScheme(root: Bundle.main.resourceURL!.appendingPathComponent("ui")), forURLScheme: "rsxp")
        configuration.allowsInlineMediaPlayback = true
        configuration.mediaTypesRequiringUserActionForPlayback = []
        webView = WKWebView(frame: .zero, configuration: configuration)
        webView.uiDelegate = self; webView.navigationDelegate = self
        webView.scrollView.bounces = false
        webView.scrollView.contentInsetAdjustmentBehavior = .never
        webView.isOpaque = false; webView.backgroundColor = UIColor(red: 16/255, green: 24/255, blue: 32/255, alpha: 1)
        let container=UIView()
        webView.translatesAutoresizingMaskIntoConstraints=false
        container.addSubview(webView)
        NSLayoutConstraint.activate([webView.topAnchor.constraint(equalTo:container.topAnchor),webView.bottomAnchor.constraint(equalTo:container.bottomAnchor),webView.leadingAnchor.constraint(equalTo:container.leadingAnchor),webView.trailingAnchor.constraint(equalTo:container.trailingAnchor)])
        gameImage.contentMode = .scaleAspectFit
        gameImage.layer.magnificationFilter = .nearest
        gameImage.layer.minificationFilter = .nearest
        gameImage.isUserInteractionEnabled=false
        gameImage.isHidden=true
        container.addSubview(gameImage)
        gameView.isUserInteractionEnabled=false
        gameView.isHidden=true
        gameView.metalLayer.framebufferOnly = true
        gameView.metalLayer.isOpaque = true
        gameView.layer.zPosition = 10
        container.addSubview(gameView)
        view=container
        container.backgroundColor = UIColor(red: 16/255, green: 24/255, blue: 32/255, alpha: 1)
        var interfaceURL = "rsxp://app/index.html"
        // Repeatable launch hook for Simulator smoke tests, including Release builds.
        if ProcessInfo.processInfo.arguments.contains("--demo") { interfaceURL += "?demo=1" }
        let arguments = ProcessInfo.processInfo.arguments
        if let index = arguments.firstIndex(of: "--game"), arguments.indices.contains(index + 1) {
            var components = URLComponents(string: "rsxp://app/index.html")!
            components.queryItems = [URLQueryItem(name: "game", value: arguments[index + 1])]
            interfaceURL = components.string!
        }
        webView.load(URLRequest(url: URL(string: interfaceURL)!))
    }
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        updateSurface()
        applySafeAreaInsets()
    }
    /// WebKit can report `env(safe-area-inset-*)` as zero (notably in portrait
    /// and shortly after launch). Pass the UIKit values to CSS so the in-game
    /// menu and controls stay clear of the status bar and home indicator.
    private var appliedInsets = UIEdgeInsets.zero
    private func applySafeAreaInsets(force: Bool = false) {
        guard webView != nil else { return }
        let insets = view.safeAreaInsets
        guard force || insets != appliedInsets else { return }
        appliedInsets = insets
        let script = """
        const s=document.documentElement.style;
        s.setProperty('--safe-top','\(insets.top)px');
        s.setProperty('--safe-right','\(insets.right)px');
        s.setProperty('--safe-bottom','\(insets.bottom)px');
        s.setProperty('--safe-left','\(insets.left)px');
        """
        webView.evaluateJavaScript(script, completionHandler: nil)
    }
    /// Attaches the Metal layer once the view has real bounds, then keeps the
    /// drawable size in physical pixels during rotation and layout changes.
    private func updateSurface() {
        let rect = gameViewport.isEmpty ? view.bounds : gameViewport
        guard !rect.isEmpty else { return }
        if gameViewport.isEmpty { gameView.frame = rect }
        let scale = view.window?.screen.scale ?? UIScreen.main.scale
        let width = UInt32(max(1, (rect.width * scale).rounded()))
        let height = UInt32(max(1, (rect.height * scale).rounded()))
        if !surfaceAttached {
            surfaceAttached = rsxp_mobile_attach_surface(Unmanaged.passUnretained(gameView.metalLayer).toOpaque(), width, height) == 1
        } else {
            _ = rsxp_mobile_resize_surface(width, height)
        }
        gameView.metalLayer.contentsScale = scale
        gameView.metalLayer.drawableSize = CGSize(width: Int(width), height: Int(height))
    }
    private func updateGameVisibility() {
        gameImage.isHidden = !frameVisible || usingGPU
        gameView.isHidden = !frameVisible || !usingGPU
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        // The interface may load after the first layout pass.
        applySafeAreaInsets(force: true)
    }
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        // Only trusted, bundled UI can reach the synchronous native bridge.
        let trusted = action.request.url?.scheme == "rsxp" && action.request.url?.host == "app"
        decisionHandler(trusted ? .allow : .cancel)
    }
    func webView(_ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String, defaultText: String?, initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (String?) -> Void) {
        guard frame.isMainFrame, frame.request.url?.scheme == "rsxp", prompt.hasPrefix("rsxp:") else { completionHandler(nil); return }
        let request = String(prompt.dropFirst(5))
        let value=(try? JSONSerialization.jsonObject(with:Data(request.utf8))) as? [String:Any]
        let op = value?["op"] as? String
        // `init` attaches the drawable before the Rust runtime is created.
        if op == "init" { updateSurface() }
        if op == "viewport" {
            gameViewport=CGRect(x:(value?["x"] as? NSNumber)?.doubleValue ?? 0,y:(value?["y"] as? NSNumber)?.doubleValue ?? 0,width:(value?["width"] as? NSNumber)?.doubleValue ?? 0,height:(value?["height"] as? NSNumber)?.doubleValue ?? 0)
            gameImage.frame=gameViewport
            gameView.frame=gameViewport
            updateSurface()
            if usingGPU { updateGameVisibility() }
            completionHandler("{\"ok\":null}");return
        }
        // Native frames sit above the WebView, so interface overlays (game menu,
        // errors) must hide the image while they are visible.
        if op == "frameVisibility" {
            frameVisible = value?["visible"] as? Bool ?? true
            updateGameVisibility()
            completionHandler("{\"ok\":null}");return
        }
        if let data = request.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any], value["op"] as? String == "import" {
            if !importing {
                importing = true
                DispatchQueue.main.async {
                    let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder], asCopy: false)
                    picker.delegate = self; picker.allowsMultipleSelection = false
                    self.present(picker, animated: true)
                }
            }
            completionHandler("{\"ok\":null}"); return
        }
        let pointer = request.withCString { rsxp_mobile_request($0) }
        guard let pointer = pointer else { completionHandler("{\"error\":\"Native response unavailable\"}"); return }
        let result = String(cString: pointer); rsxp_mobile_free(pointer)
        var nativeError: String?
        if let data = result.data(using: .utf8),
           let response = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
           let error = response["error"] as? String {
            nativeError = error
            NSLog("RSXP native error: %@", error)
        }
        if op == "init" {
            // The Rust runtime attaches its presenter during init.
            usingGPU = nativeError == nil && rsxp_mobile_gpu_active() == 1
            if usingGPU { gameImage.isHidden = true }
        }
        if op == "stop" {
            usingGPU = false
            gameView.isHidden = true
            gameImage.isHidden = true
        }
        if op == "step" {
            if usingGPU { updateGameVisibility() } else { presentFrame() }
        }
        completionHandler(result)
    }
    private func presentFrame() {
        if usingGPU { return }
        var width:UInt32=0,height:UInt32=0,length=0
        guard let bytes=rsxp_mobile_frame(&width,&height,&length),width>0,height>0 else{return}
        let data=Data(bytes:bytes,count:length)
        guard let provider=CGDataProvider(data:data as CFData),let image=CGImage(width:Int(width),height:Int(height),bitsPerComponent:8,bitsPerPixel:32,bytesPerRow:Int(width)*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGBitmapInfo(rawValue:CGImageAlphaInfo.last.rawValue),provider:provider,decode:nil,shouldInterpolate:false,intent:.defaultIntent) else{return}
        gameImage.image=UIImage(cgImage:image)
        updateGameVisibility()
    }
    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { imported(nil) }
    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        guard let source = urls.first else { imported(nil); return }
        DispatchQueue.global(qos: .userInitiated).async {
            let scoped = source.startAccessingSecurityScopedResource()
            defer { if scoped { source.stopAccessingSecurityScopedResource() } }
            var error: String?
            let temporary = self.library.appendingPathComponent(".import-" + UUID().uuidString)
            do {
                let name = source.lastPathComponent
                guard !name.hasPrefix("."), !name.isEmpty else { throw CocoaError(.fileReadInvalidFileName) }
                let destination = self.library.appendingPathComponent(name)
                guard !FileManager.default.fileExists(atPath: destination.path) else { throw CocoaError(.fileWriteFileExists) }
                try FileManager.default.copyItem(at: source, to: temporary)
                try FileManager.default.moveItem(at: temporary, to: destination)
            } catch let failure { error = failure.localizedDescription; try? FileManager.default.removeItem(at: temporary) }
            DispatchQueue.main.async { self.imported(error) }
        }
    }
    private func imported(_ error: String?) {
        importing = false
        let argument = String(data: try! JSONSerialization.data(withJSONObject: [error as Any? ?? NSNull()]), encoding: .utf8)!
        webView.evaluateJavaScript("globalThis.RSXPMobileImported?.(\(argument)[0])", completionHandler: nil)
    }
}
