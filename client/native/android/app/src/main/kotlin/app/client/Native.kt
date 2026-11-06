package app.client

/** The app's library (client/native/src/android.rs), which answers what the system starts with no window, by the same
 *  code the window uses: `kind` is "join", "call", "widgets", "open-note" or "register-push", `json` its arguments →
 *  `{ "ok": … }` or `{ "error": "…" }`. It waits, so never on the main thread. */
object Native {
    init {
        System.loadLibrary("one_native")
    }

    @JvmStatic
    external fun call(kind: String, json: String): String
}
