package app.theone.strictspike;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;

/** The app's own screen, as the One App's shell is: what opens a screen in a WebView or with our own renderer. */
public class MainActivity extends Activity {
    @Override protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(48, 160, 48, 48);
        layout.addView(button("Notifications in WebView", WebActivity.class, "notifications"));
        layout.addView(button("Notifications, own renderer", StrictActivity.class, "notifications"));
        layout.addView(button("Form in WebView", WebActivity.class, "form"));
        layout.addView(button("Form, own renderer", StrictActivity.class, "form"));
        setContentView(layout);
    }

    private Button button(String label, Class<?> activity, String screen) {
        Button b = new Button(this);
        b.setText(label);
        b.setOnClickListener(v -> startActivity(new Intent(this, activity).putExtra("screen", screen)));
        return b;
    }
}
