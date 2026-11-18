package app.theone.strictspike;

import android.app.Activity;
import android.os.Bundle;
import android.os.SystemClock;
import android.util.Log;

/** A strict screen drawn by our own renderer: `screen` (notifications, form) names its bundle in the APK's assets. It's
 * fully drawn when the renderer has drawn its first frame on the view's surface. */
public class StrictActivity extends Activity {
    private StrictView view;

    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        long created = SystemClock.uptimeMillis();
        String screen = getIntent().getStringExtra("screen");
        if (screen == null) screen = "notifications";
        view = new StrictView(this, "app:///strict/" + screen + ".js", () -> {
            Log.i("StrictSpike", "strict drawn " + (SystemClock.uptimeMillis() - created) + " ms after onCreate");
            reportFullyDrawn();
        });
        setContentView(view);
    }

    @Override protected void onDestroy() {
        view.destroy();
        super.onDestroy();
    }
}
