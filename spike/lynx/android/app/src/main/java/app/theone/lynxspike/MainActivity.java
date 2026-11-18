package app.theone.lynxspike;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;

/** The app's own screen, as the One App's shell is: what opens a screen in a WebView or in Lynx. */
public class MainActivity extends Activity {
    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(48, 160, 48, 48);
        layout.addView(button("Open in WebView", WebActivity.class));
        layout.addView(button("Open in Lynx", LynxActivity.class));
        setContentView(layout);
    }

    private Button button(String label, Class<?> activity) {
        Button b = new Button(this);
        b.setText(label);
        b.setOnClickListener(v -> startActivity(new Intent(this, activity)));
        return b;
    }
}
