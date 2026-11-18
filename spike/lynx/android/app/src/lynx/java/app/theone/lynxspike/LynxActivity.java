package app.theone.lynxspike;

import android.app.Activity;
import android.os.Bundle;
import android.os.SystemClock;
import android.util.Log;
import android.view.MotionEvent;
import com.lynx.tasm.LynxView;
import com.lynx.tasm.LynxViewBuilder;
import com.lynx.tasm.LynxViewClient;
import com.lynx.tasm.provider.AbsTemplateProvider;
import com.lynx.xelement.XElementBehaviors;
import java.io.IOException;
import java.io.InputStream;
import java.util.Map;

/** The same screen in Lynx: its background thread runs the app, its main thread draws the ops with Element PAPI.
 * Fully drawn when Lynx says the first update's flush (`__lynx_timing_actual_fmp`) is painted. */
public class LynxActivity extends Activity {
    private long created;
    private long touched;

    @Override public boolean dispatchTouchEvent(MotionEvent e) {
        if (e.getAction() == MotionEvent.ACTION_UP) touched = e.getEventTime();
        return super.dispatchTouchEvent(e);
    }

    private void painted() {
        Log.i("LynxSpike", "lynx tap to painted " + (SystemClock.uptimeMillis() - touched) + " ms");
    }
    private boolean drawn;

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        created = SystemClock.uptimeMillis();
        LynxViewBuilder builder = new LynxViewBuilder();
        builder.addBehaviors(new XElementBehaviors().create());
        if (getIntent().getBooleanExtra("threads", true)) builder.setThreadStrategyForRendering(com.lynx.tasm.ThreadStrategyForRendering.MULTI_THREADS);
        builder.setTemplateProvider(new AbsTemplateProvider() {
            @Override public void loadTemplate(String uri, Callback callback) {
                try (InputStream in = getAssets().open(uri)) {
                    callback.onSuccess(in.readAllBytes());
                } catch (IOException e) {
                    callback.onFailed(e.getMessage());
                }
            }
        });
        LynxView view = builder.build(this);
        view.addLynxViewClient(new LynxViewClient() {
            @Override public void onFirstScreen() { log("first screen"); }
            @Override public void onTimingUpdate(Map<String, Object> timing, Map<String, Long> update, String flag) {
                log("timing update " + flag + " " + timing.get("metrics"));
                if (flag != null && flag.startsWith("update-") && touched > 0) painted();
                if (!drawn && flag != null && flag.contains("fmp")) {
                    drawn = true;
                    reportFullyDrawn();
                }
            }
            @Override public void onReceivedError(com.lynx.tasm.LynxError error) { Log.e("LynxSpike", "error " + error); }
        });
        setContentView(view);
        String bundle = getIntent().getStringExtra("bundle");
        view.renderTemplateUrl(bundle != null ? bundle : "notifications.lynx.bundle", "");
    }

    private void log(String what) {
        Log.i("LynxSpike", "lynx " + what + " " + (SystemClock.uptimeMillis() - created) + " ms after onCreate");
    }
}
