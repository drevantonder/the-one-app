package app.theone.client

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/** The app's window: the web view, kept clear of the system's bars and of the keyboard, with the window showing behind them. */
class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    ViewCompat.setOnApplyWindowInsetsListener(findViewById(android.R.id.content)) { view, insets ->
      val kept = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      val clear = insets.getInsets(kept)
      view.setPadding(clear.left, clear.top, clear.right, clear.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
