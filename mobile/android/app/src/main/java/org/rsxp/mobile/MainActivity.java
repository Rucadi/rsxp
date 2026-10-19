// SPDX-License-Identifier: GPL-2.0-or-later
package org.rsxp.mobile;
import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.DocumentsContract;
import android.system.Os;
import android.webkit.*;
import android.graphics.Bitmap;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.view.View;
import java.io.*;
import java.nio.file.Files;
import org.json.JSONObject;

public final class MainActivity extends Activity {
    static { System.loadLibrary("rsxp_jni"); }
    private static final int REQUEST_STORAGE = 40;
    private static final int REQUEST_TREE = 41;
    private static native boolean nativeConfigure(String path,String runtime);
    private static native String nativeRequest(String request);
    private static native int[] nativeFrame();
    private ImageView gameImage;
    private Bitmap bitmap;
    private WebView web;
    private File library;
    private boolean importing;
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        library = new File(getFilesDir(), "Games");
        File runtime=new File(getFilesDir(),"runtime");
        File marker=new File(runtime,".rsxp-runtime");
        try {
            if(!marker.isFile()) {
                deleteTree(runtime);
                copyAssets("runtime",runtime);
                if(!marker.createNewFile()) throw new IOException("Cannot mark the runtime as installed");
            }
        } catch(IOException error) {throw new IllegalStateException(error);}
        if (!nativeConfigure(library.getAbsolutePath(),runtime.getAbsolutePath())) throw new IllegalStateException("Cannot configure library");
        web = new WebView(this);
        web.getSettings().setJavaScriptEnabled(true);
        web.getSettings().setDomStorageEnabled(true);
        web.getSettings().setAllowFileAccess(false);
        web.getSettings().setAllowContentAccess(false);
        web.setWebViewClient(new WebViewClient() {
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                return !trusted(request.getUrl());
            }
            @Override public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                if (!trusted(request.getUrl())) return new WebResourceResponse("text/plain", "UTF-8", new ByteArrayInputStream(new byte[0]));
                String path = request.getUrl().getPath();
                if (path == null || path.contains("..")) return null;
                String mime = path.endsWith(".js") ? "text/javascript" : path.endsWith(".css") ? "text/css" : path.endsWith(".html") ? "text/html" : "application/octet-stream";
                try { return new WebResourceResponse(mime, "UTF-8", getAssets().open("ui" + path)); }
                catch (IOException error) { return new WebResourceResponse("text/plain", "UTF-8", 404, "Not found", java.util.Collections.emptyMap(), new ByteArrayInputStream(new byte[0])); }
            }
        });
        web.setWebChromeClient(new WebChromeClient() {
            @Override public boolean onJsPrompt(WebView view, String url, String message, String defaultValue, JsPromptResult result) {
                if (!trusted(Uri.parse(url)) || !message.startsWith("rsxp:")) { result.cancel(); return true; }
                String request = message.substring(5);
                try {
                    JSONObject value=new JSONObject(request);
                    String op=value.optString("op");
                    if(op.equals("viewport")) {
                        float scale=getResources().getDisplayMetrics().density;
                        FrameLayout.LayoutParams layout=new FrameLayout.LayoutParams((int)(value.getDouble("width")*scale),(int)(value.getDouble("height")*scale));
                        layout.leftMargin=(int)(value.getDouble("x")*scale);layout.topMargin=(int)(value.getDouble("y")*scale);gameImage.setLayoutParams(layout);
                        result.confirm("{\"ok\":null}");return true;
                    }
                    if(op.equals("frameVisibility")) {
                        gameImage.setVisibility(value.optBoolean("visible",true)?View.VISIBLE:View.GONE);
                        result.confirm("{\"ok\":null}");return true;
                    }
                    if (op.equals("import")) {
                        chooseGameFolder();
                        result.confirm("{\"ok\":null}");
                    } else {
                        String response=nativeRequest(request);
                        if(op.equals("step")) presentFrame();
                        if(op.equals("stop")) gameImage.setVisibility(View.GONE);
                        result.confirm(response);
                    }
                } catch (Exception error) { result.confirm("{\"error\":" + JSONObject.quote(error.toString()) + "}"); }
                return true;
            }
        });
        FrameLayout container=new FrameLayout(this);
        container.addView(web,new FrameLayout.LayoutParams(-1,-1));
        gameImage=new ImageView(this);gameImage.setScaleType(ImageView.ScaleType.FIT_CENTER);gameImage.setVisibility(View.GONE);
        container.addView(gameImage,new FrameLayout.LayoutParams(1,1));
        setContentView(container);
        String url="https://rsxp.local/index.html";
        String game=getIntent()==null?null:getIntent().getStringExtra("game");
        if(game!=null&&!game.isEmpty()) url+="?game="+Uri.encode(game);
        else if(getIntent()!=null&&getIntent().getBooleanExtra("demo",false)) url+="?demo=1";
        web.loadUrl(url);
    }
    private boolean trusted(Uri uri) { return "https".equals(uri.getScheme()) && "rsxp.local".equals(uri.getHost()); }
    private void presentFrame() {
        int[] frame=nativeFrame();if(frame==null)return;
        int width=frame[0],height=frame[1];
        if(bitmap==null||bitmap.getWidth()!=width||bitmap.getHeight()!=height) bitmap=Bitmap.createBitmap(width,height,Bitmap.Config.ARGB_8888);
        bitmap.setPixels(frame,2,width,0,0,width,height);gameImage.setImageBitmap(bitmap);gameImage.setVisibility(View.VISIBLE);
    }
    private void copyAssets(String source,File destination) throws IOException {
        String[] children=getAssets().list(source);
        if(children!=null&&children.length>0) {if(!destination.mkdirs()&&!destination.isDirectory()) throw new IOException("Cannot create runtime directory");for(String child:children)copyAssets(source+"/"+child,new File(destination,child));}
        else try(InputStream input=getAssets().open(source)){Files.copy(input,destination.toPath());}
    }
    private void chooseGameFolder() {
        if (importing) return;
        if (Build.VERSION.SDK_INT >= 23
                && checkSelfPermission(Manifest.permission.READ_EXTERNAL_STORAGE)
                != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.READ_EXTERNAL_STORAGE},
                    REQUEST_STORAGE);
            return;
        }
        openTreePicker();
    }
    private void openTreePicker() {
        if (importing) return;
        importing = true;
        startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), REQUEST_TREE);
    }
    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request != REQUEST_STORAGE) return;
        if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED) openTreePicker();
        else imported("Storage access is required to run a game in place");
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request,result,data);
        if (request != REQUEST_TREE) return;
        if (result != RESULT_OK || data == null || data.getData() == null) { imported(null); return; }
        Uri tree = data.getData();
        try {
            getContentResolver().takePersistableUriPermission(
                    tree, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (Exception ignored) {
            // The selected filesystem path is still usable for this session.
        }
        new Thread(() -> linkTree(tree), "rsxp-link").start();
    }
    /**
     * Links the selected game folder into the library without copying it. The
     * folder must expose a real filesystem path (Documents, Downloads or a
     * storage volume); cloud-only document providers cannot be used in place.
     */
    private void linkTree(Uri tree) {
        String error = null;
        try {
            File source = treePath(tree);
            if (source == null || !source.isDirectory()) {
                throw new IOException("This folder provider has no filesystem path. "
                        + "Choose a folder under Documents, Downloads or a storage volume.");
            }
            if (!source.canRead()) {
                throw new IOException("Cannot read " + source.getAbsolutePath()
                        + ". Allow storage access and choose the folder again.");
            }
            String name = source.getName();
            if (!safeName(name) || "0".equals(name)) {
                throw new IOException("The selected folder has an invalid name");
            }
            File link = new File(library, name);
            if (link.exists() || Files.isSymbolicLink(link.toPath())) {
                throw new IOException("A game named \"" + name + "\" is already in the library");
            }
            Os.symlink(source.getAbsolutePath(), link.getAbsolutePath());
        } catch (Exception failure) {
            error = failure.getMessage() == null ? failure.toString() : failure.getMessage();
        }
        String message = error;
        runOnUiThread(() -> imported(message));
    }
    private File treePath(Uri tree) {
        String id = DocumentsContract.getTreeDocumentId(tree);
        if (id == null) return null;
        int colon = id.indexOf(':');
        if (colon < 0) return null;
        String volume = id.substring(0, colon);
        String relative = id.substring(colon + 1);
        if ("raw".equalsIgnoreCase(volume)) return new File(relative);
        relative = relative.replaceFirst("^/+", "");
        File base;
        if ("primary".equalsIgnoreCase(volume)) {
            base = Environment.getExternalStorageDirectory();
        } else if ("home".equalsIgnoreCase(volume)) {
            base = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOCUMENTS);
        } else if ("downloads".equalsIgnoreCase(volume)) {
            base = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS);
        } else {
            base = new File("/storage/" + volume);
        }
        return new File(base, relative);
    }
    private static boolean safeName(String name) { return name != null && !name.isEmpty() && !name.startsWith(".") && !name.contains("/") && !name.contains("\\"); }
    private void deleteTree(File path) { File[] children=path.listFiles(); if(children!=null) for(File child:children) deleteTree(child); path.delete(); }
    private void imported(String error) { importing=false; web.evaluateJavascript("globalThis.RSXPMobileImported?.(" + (error == null ? "null" : JSONObject.quote(error)) + ")",null); }
    @Override protected void onPause() { super.onPause(); web.evaluateJavascript("window.dispatchEvent(new Event('blur'))",null); web.onPause(); }
    @Override protected void onResume() { super.onResume(); if(web!=null) web.onResume(); }
    @Override protected void onDestroy() { web.destroy(); super.onDestroy(); }
}
