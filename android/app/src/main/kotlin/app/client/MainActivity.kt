package app.client

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val store get() = (application as ClientApp).store

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge(SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT))
        super.onCreate(savedInstanceState)
        val store = store
        if (store.phase != Store.Phase.SignedOut) store.run()
        setContent {
            Theme(store.config?.accent ?: BuildConfig.ACCENT, store.appearance) {
                Root(store)
            }
        }
        if (savedInstanceState == null) intent?.data?.let(::open)
    }

    private var started = false

    override fun onStart() {
        super.onStart()
        if (started) store.resumed()
        started = true
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.data?.let(::open)
    }

    /** `<cli>://signed-in/?code=…`: back from signing in; `<cli>://open/…`: a home-screen widget, tapped; a link to the
     *  install's address (Links), or the same as `<cli>://<path>`. */
    private fun open(uri: Uri) {
        if (uri.scheme == "https" || uri.scheme == "http") return Links.open(this, store, uri)
        if (uri.scheme != BuildConfig.CLI) return
        if (uri.host == "open") return app.client.widget.HomeWidgets.open(store, uri)
        if (uri.host != "signed-in") return Links.own(store, uri)
        welcomeError = null
        store.signInFinished(uri) { welcomeError = it }
    }

    companion object {
        var welcomeError by mutableStateOf<String?>(null)
    }
}

/** The app's process: one store, however many times the activity comes and goes. */
class ClientApp : android.app.Application() {
    val store by lazy { Store(this, SignIn(this), getSharedPreferences("app", MODE_PRIVATE)) }

    override fun onCreate() {
        super.onCreate()
        ScreenCache.dir = java.io.File(cacheDir, "screens")
        Glyphs.loadSoon(this)
        app.client.widget.HomeWidgets.start(this)
    }
}

@Composable
private fun Root(store: Store) {
    val p = LocalPalette.current
    // The status and gesture bars in this app's light or dark, which Settings may choose apart from the phone's.
    val view = androidx.compose.ui.platform.LocalView.current
    androidx.compose.runtime.SideEffect {
        val window = (view.context as? android.app.Activity)?.window ?: return@SideEffect
        androidx.core.view.WindowCompat.getInsetsController(window, view).apply {
            isAppearanceLightStatusBars = !p.dark
            isAppearanceLightNavigationBars = !p.dark
        }
    }
    Box(Modifier.fillMaxSize().background(p.background)) {
        if (store.phase == Store.Phase.SignedOut) Welcome(store) else Shell(store)
    }
}

/** Signing in: where the install is (unless this build knows), or an invite link to it, then the passkey. */
@Composable
private fun Welcome(store: Store) {
    val p = LocalPalette.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    var address by remember { mutableStateOf(store.signIn.address ?: "") }
    var busy by remember { mutableStateOf(false) }
    var invited by remember { mutableStateOf(false) }
    val fixed = BuildConfig.ADDRESS.isNotEmpty()
    val error = MainActivity.welcomeError
    val name = BuildConfig.NAME

    fun signIn() {
        keyboard?.hide()
        val given = address.trim()
        // An invite link: its address, and its page, where the person makes their passkey first.
        val invite = Regex("^(https?://[^/]+)/invite#.+").find(given)
        val at = invite?.groupValues?.get(1) ?: normal(given)
        if (at == null) {
            MainActivity.welcomeError = "That isn’t an address. It looks like one.example.com."
            return
        }
        store.signIn.address = at
        MainActivity.welcomeError = null
        if (invite != null) {
            invited = true
            address = at
            SignIn.openInTab(context, Uri.parse(given))
            return
        }
        busy = true
        scope.launch {
            try {
                store.signIn.start(context as android.app.Activity)
            } catch (e: NodeError) {
                MainActivity.welcomeError = e.message
            } catch (e: Exception) {
                MainActivity.welcomeError = "Can’t reach that address. Check it, and that you’re online."
            }
            busy = false
        }
    }

    // An invite link this app was opened with, as if it were pasted.
    LaunchedEffect(Links.invite) {
        val link = Links.invite ?: return@LaunchedEffect
        Links.invite = null
        address = link
        signIn()
    }

    Column(
        Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding().padding(horizontal = 28.dp, vertical = 20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.weight(1f))
        Image(painterResource(R.drawable.logo), null, Modifier.size(116.dp).clip(RoundedCornerShape(26.dp)))
        Spacer(Modifier.height(28.dp))
        Text(name, fontSize = 34.sp, lineHeight = 40.sp, fontWeight = FontWeight.Bold, color = p.label, textAlign = TextAlign.Center)
        if (invited) {
            Spacer(Modifier.height(12.dp))
            Text("Made your passkey? Sign in with it.", style = Type.body, color = p.secondary, textAlign = TextAlign.Center)
        }
        if (!fixed) {
            Spacer(Modifier.height(32.dp))
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text("${BuildConfig.YOURS.replaceFirstChar { it.uppercase() }}’s address, or an invite link", style = Type.footnote, color = p.secondary, modifier = Modifier.padding(horizontal = 16.dp))
                TextField(
                    address, { address = it }, Modifier.fillMaxWidth(), singleLine = true, placeholder = { Text("one.example.com", color = p.tertiary) },
                    shape = RoundedCornerShape(16.dp),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go, autoCorrectEnabled = false),
                    keyboardActions = KeyboardActions(onGo = { signIn() }),
                    colors = TextFieldDefaults.colors(focusedContainerColor = p.card, unfocusedContainerColor = p.card, focusedIndicatorColor = Color.Transparent,
                        unfocusedIndicatorColor = Color.Transparent, cursorColor = p.accent),
                )
            }
        }
        error?.let {
            Spacer(Modifier.height(16.dp))
            Text(it, style = Type.subheadline, color = p.red, textAlign = TextAlign.Center)
        }
        Spacer(Modifier.weight(1.4f))
        Prominent(if (invited || address.isEmpty() || !address.contains("/invite#")) "Sign In" else "Open Invite", Modifier.fillMaxWidth().height(56.dp), enabled = !busy && (fixed || address.isNotBlank())) { signIn() }
        Box(Modifier.height(48.dp), contentAlignment = Alignment.Center) {
            if (busy) CircularProgressIndicator(Modifier.size(20.dp), color = p.secondary, strokeWidth = 2.dp)
            else if (invited) TextButton({ invited = false }) { Text("Use Another Address", color = p.accent) }
        }
    }
}

/** An address as the app keeps it: https unless it's this device's own. → nil if it isn't one. */
private fun normal(given: String): String? {
    val text = given.trimEnd('/')
    if (text.isEmpty() || text.contains(' ')) return null
    val url = if (text.contains("://")) text else if (text.startsWith("localhost") || text.startsWith("127.0.0.1")) "http://$text" else "https://$text"
    val uri = Uri.parse(url)
    if (uri.host.isNullOrEmpty() || uri.scheme !in listOf("http", "https")) return null
    if (uri.scheme == "http" && uri.host !in listOf("localhost", "127.0.0.1")) return null
    return "${uri.scheme}://${uri.authority}"
}
