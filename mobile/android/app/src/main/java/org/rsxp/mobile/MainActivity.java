// SPDX-License-Identifier: GPL-2.0-or-later
package org.rsxp.mobile;
import android.app.Activity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.DocumentsContract;
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
        try { if(!runtime.isDirectory()) copyAssets("runtime",runtime); } catch(IOException error) {throw new IllegalStateException(error);}
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
                        if (!importing) {
                            importing = true;
                            startActivityForResult(new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), 1);
                        }
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
        web.loadUrl("https://rsxp.local/index.html");
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
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request,result,data);
        if (request != 1) return;
        if (result != RESULT_OK || data == null || data.getData() == null) { imported(null); return; }
        Uri tree = data.getData();
        new Thread(() -> {
            File temporary = new File(library, ".import-" + java.util.UUID.randomUUID());
            try {
                Uri root = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
                String name;
                try (Cursor cursor = getContentResolver().query(root, new String[]{DocumentsContract.Document.COLUMN_DISPLAY_NAME}, null,null,null)) {
                    if (cursor == null || !cursor.moveToFirst()) throw new IOException("Folder unavailable");
                    name = cursor.getString(0);
                }
                if (!safeName(name)) throw new IOException("Invalid game folder name");
                File destination = new File(library,name);
                if (destination.exists()) throw new IOException("A game with this name is already imported");
                copyFolder(tree, DocumentsContract.getDocumentId(root), temporary);
                if (!temporary.renameTo(destination)) throw new IOException("Could not finish import");
                runOnUiThread(() -> imported(null));
            } catch (Exception error) {
                deleteTree(temporary);
                runOnUiThread(() -> imported(error.toString()));
            }
        }, "rsxp-import").start();
    }
    private static boolean safeName(String name) { return name != null && !name.isEmpty() && !name.startsWith(".") && !name.contains("/") && !name.contains("\\"); }
    private void copyFolder(Uri tree, String id, File directory) throws IOException {
        if (!directory.mkdirs()) throw new IOException("Cannot create import directory");
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree,id);
        try (Cursor cursor = getContentResolver().query(children,new String[]{DocumentsContract.Document.COLUMN_DOCUMENT_ID,DocumentsContract.Document.COLUMN_DISPLAY_NAME,DocumentsContract.Document.COLUMN_MIME_TYPE},null,null,null)) {
            if (cursor == null) throw new IOException("Cannot read game folder");
            while (cursor.moveToNext()) {
                String childId=cursor.getString(0), name=cursor.getString(1), mime=cursor.getString(2);
                if (!safeName(name)) continue;
                File destination=new File(directory,name);
                if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mime)) copyFolder(tree,childId,destination);
                else try (InputStream input=getContentResolver().openInputStream(DocumentsContract.buildDocumentUriUsingTree(tree,childId))) {
                    if (input == null) throw new IOException("Cannot read " + name);
                    Files.copy(input,destination.toPath());
                }
            }
        }
    }
    private void deleteTree(File path) { File[] children=path.listFiles(); if(children!=null) for(File child:children) deleteTree(child); path.delete(); }
    private void imported(String error) { importing=false; web.evaluateJavascript("globalThis.RSXPMobileImported?.(" + (error == null ? "null" : JSONObject.quote(error)) + ")",null); }
    @Override protected void onPause() { super.onPause(); web.evaluateJavascript("window.dispatchEvent(new Event('blur'))",null); web.onPause(); }
    @Override protected void onResume() { super.onResume(); if(web!=null) web.onResume(); }
    @Override protected void onDestroy() { web.destroy(); super.onDestroy(); }
}
