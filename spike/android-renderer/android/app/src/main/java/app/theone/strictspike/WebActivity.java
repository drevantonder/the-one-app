package app.theone.strictspike;

import android.app.Activity;
import android.os.Bundle;
import android.os.SystemClock;
import android.util.Log;
import android.view.MotionEvent;
import android.webkit.JavascriptInterface;
import android.webkit.WebView;

/** Today's way: the screen in an Android WebView (what Tauri's wry uses on Android), its page drawing the ops to the
 * DOM. `drawn()` is called once its first update is applied; it's fully drawn when the WebView says that state is on
 * screen (`postVisualStateCallback`). */
public class WebActivity extends Activity {
    private long created;
    private long touched;

    @Override public boolean dispatchTouchEvent(MotionEvent e) {
        if (e.getAction() == MotionEvent.ACTION_UP) touched = e.getEventTime();
        return super.dispatchTouchEvent(e);
    }

    private void painted() {
        Log.i("StrictSpike", "web tap to painted " + (SystemClock.uptimeMillis() - touched) + " ms");
    }
    private WebView web;

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        created = SystemClock.uptimeMillis();
        web = new WebView(this);
        web.getSettings().setJavaScriptEnabled(true);
        web.getSettings().setAllowFileAccess(true);
        web.addJavascriptInterface(this, "Host");
        setContentView(web);
        String screen = getIntent().getStringExtra("screen");
        web.loadUrl("file:///android_asset/web/index.html" + ("form".equals(screen) ? "#form" : ""));
    }

    @JavascriptInterface public void updated() {
        runOnUiThread(() -> web.postVisualStateCallback(1, new WebView.VisualStateCallback() {
            @Override public void onComplete(long id) { if (touched > 0) painted(); }
        }));
    }

    @JavascriptInterface public void drawn() {
        runOnUiThread(() -> web.postVisualStateCallback(0, new WebView.VisualStateCallback() {
            @Override public void onComplete(long id) {
                Log.i("StrictSpike", "webview drawn " + (SystemClock.uptimeMillis() - created) + " ms after onCreate");
                reportFullyDrawn();
            }
        }));
    }
}
