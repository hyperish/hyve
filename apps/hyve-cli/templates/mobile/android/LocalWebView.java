package org.hyperish.serge.webview;

import android.app.Activity;
import android.graphics.Bitmap;
import android.net.Uri;
import android.os.Message;
import android.view.Gravity;
import android.view.ViewGroup;
import android.webkit.*;
import android.widget.FrameLayout;
import androidx.webkit.WebMessageCompat;
import androidx.webkit.WebViewCompat;
import androidx.webkit.WebViewFeature;
import java.io.ByteArrayInputStream;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicBoolean;

/** A NativeActivity child bound to one immutable bundle and one local HTTPS origin. */
public final class LocalWebView {
    private static final class Asset {
        final String type;
        final byte[] bytes;
        Asset(String type, byte[] bytes) { this.type = type; this.bytes = bytes; }
    }
    private final Activity activity;
    private final String origin, page, bootstrap, prefix;
    private final Map<String, String> headers = new HashMap<>();
    private final Map<String, Asset> files = new HashMap<>();
    private final BlockingQueue<String> events = new ArrayBlockingQueue<>(128);
    private final AtomicBoolean closed = new AtomicBoolean();
    private WebView view;

    public LocalWebView(Activity activity, String origin, String bootstrap, String prefix, String csp) {
        this.activity = activity;
        this.origin = origin;
        this.page = origin + "/index.html";
        this.bootstrap = bootstrap;
        this.prefix = prefix;
        headers.put("Content-Security-Policy", csp);
        headers.put("X-Content-Type-Options", "nosniff");
        headers.put("Cache-Control", "no-store");
    }

    // Called only during construction, before publishing the map to the WebView's IO thread.
    public void asset(String path, String type, byte[] bytes) { files.put(path, new Asset(type, bytes)); }

    private WebResourceResponse response(WebResourceRequest request) {
        Uri url = request.getUrl();
        Asset file = null;
        if (request.getMethod().equals("GET") && url.getQuery() == null && url.getFragment() == null
            && origin.equals(url.getScheme() + "://" + url.getEncodedAuthority())) {
            file = files.get(url.getEncodedPath());
        }
        if (file == null) return new WebResourceResponse("text/plain", "utf-8", 404, "Not Found",
            headers, new ByteArrayInputStream(new byte[0]));
        return new WebResourceResponse(file.type, null, 200, "OK", headers, new ByteArrayInputStream(file.bytes));
    }

    private void post(String event) {
        if (!closed.get() && !events.offer(event)) close();
    }

    private static FrameLayout.LayoutParams bounds(int x, int y, int width, int height) {
        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(width, height, Gravity.TOP | Gravity.LEFT);
        params.leftMargin = x;
        params.topMargin = y;
        return params;
    }

    public void attach(int x, int y, int w, int h) throws Exception {
        FutureTask<Void> create = new FutureTask<>(() -> {
            if (closed.get() || activity.isDestroyed()) throw new IllegalStateException("activity closed");
            if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)
                || !WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER))
                throw new UnsupportedOperationException("WebView requires a document-start script and a scoped message listener");
            view = new WebView(activity);
            WebSettings settings = view.getSettings();
            settings.setJavaScriptEnabled(true);
            settings.setDomStorageEnabled(true);
            settings.setAllowFileAccess(false);
            settings.setAllowContentAccess(false);
            settings.setGeolocationEnabled(false);
            settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
            settings.setSupportMultipleWindows(true);
            view.setDownloadListener((url, ua, disposition, type, len) -> {});
            view.setWebChromeClient(new WebChromeClient() {
                @Override public void onPermissionRequest(PermissionRequest request) { request.deny(); }
                @Override public boolean onCreateWindow(WebView v, boolean dialog, boolean gesture, Message result) { return false; }
            });
            view.setWebViewClient(new WebViewClient() {
                @Override public WebResourceResponse shouldInterceptRequest(WebView v, WebResourceRequest req) { return response(req); }
                @Override public boolean shouldOverrideUrlLoading(WebView v, WebResourceRequest req) {
                    return !req.isForMainFrame() || !page.equals(req.getUrl().toString());
                }
                @Override public void onPageStarted(WebView v, String url, Bitmap icon) { if (page.equals(url)) post("loading"); }
                @Override public void onPageFinished(WebView v, String url) { if (page.equals(url)) post("open"); }
                @Override public boolean onRenderProcessGone(WebView v, RenderProcessGoneDetail detail) {
                    close();
                    return true;
                }
            });
            Set<String> origins = Collections.singleton(origin);
            WebViewCompat.addWebMessageListener(view, "__sergeAndroid", origins, (v, message, sender, mainFrame, reply) -> {
                if (!mainFrame || !origin.equals(sender.toString()) || !page.equals(v.getUrl())
                    || message.getType() != WebMessageCompat.TYPE_STRING) return;
                String data = message.getData();
                if (data != null && data.length() <= 87384 + prefix.length() && data.startsWith(prefix)) {
                    post("message:" + data.substring(prefix.length()));
                }
            });
            WebViewCompat.addDocumentStartJavaScript(view, bootstrap, origins);
            activity.addContentView(view, bounds(x, y, w, h));
            view.loadUrl(page);
            return null;
        });
        activity.runOnUiThread(create);
        try {
            create.get(15, TimeUnit.SECONDS);
        } catch (Exception e) {
            create.cancel(false);
            close();
            throw e;
        }
    }

    // The Rust worker releases its slot reference before blocking here.
    public String next() throws InterruptedException { return events.take(); }
    public void resize(int x, int y, int w, int h) {
        activity.runOnUiThread(() -> { if (!closed.get()) view.setLayoutParams(bounds(x, y, w, h)); });
    }
    public void evaluate(String script) {
        activity.runOnUiThread(() -> { if (!closed.get() && page.equals(view.getUrl())) view.evaluateJavascript(script, null); });
    }
    public void reload() { activity.runOnUiThread(() -> { if (!closed.get()) view.loadUrl(page); }); }

    public void close() {
        if (!closed.compareAndSet(false, true)) return;
        events.clear();
        events.offer("closed");
        activity.runOnUiThread(() -> {
            if (view == null) return;
            view.stopLoading();
            if (view.getParent() instanceof ViewGroup) ((ViewGroup)view.getParent()).removeView(view);
            view.destroy();
            view = null;
        });
    }
}
