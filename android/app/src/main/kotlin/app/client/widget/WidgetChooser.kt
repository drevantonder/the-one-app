package app.client.widget

import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.client.AppTile
import app.client.Appearance
import app.client.BuildConfig
import app.client.Glyphs
import app.client.LocalPalette
import app.client.Symbol
import app.client.Theme
import app.client.TileLook
import app.client.Type
import app.client.Unavailable

/** Which of your pages' widgets a home-screen widget shows: asked when it's placed, and when it's touched and held. */
class WidgetChooser : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge(SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT))
        super.onCreate(savedInstanceState)
        val widgetId = intent.extras?.getInt(AppWidgetManager.EXTRA_APPWIDGET_ID, AppWidgetManager.INVALID_APPWIDGET_ID) ?: AppWidgetManager.INVALID_APPWIDGET_ID
        val done = Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, widgetId)
        setResult(RESULT_CANCELED, done)
        if (widgetId == AppWidgetManager.INVALID_APPWIDGET_ID) return finish()
        HomeWidgets.start(this)
        Glyphs.load(this)
        setContent {
            val shown by HomeWidgets.shown.collectAsState()
            LaunchedEffect(Unit) { HomeWidgets.refresh() }
            Theme(shown.kept?.accent ?: BuildConfig.ACCENT, Appearance.Automatic) {
                Chooser(shown, widgetId) { w ->
                    HomeWidgets.choose(widgetId, w.id, w.title)
                    setResult(RESULT_OK, done)
                    finish()
                }
            }
        }
    }
}

@Composable
private fun Chooser(shown: HomeWidgets.Shown, widgetId: Int, choose: (KeptWidget) -> Unit) {
    val p = LocalPalette.current
    val widgets = shown.kept?.widgets.orEmpty()
    val chosen = shown.choices[widgetId] ?: widgets.firstOrNull()?.id
    Column(Modifier.fillMaxSize().background(p.background).windowInsetsPadding(WindowInsets.safeDrawing)) {
        Column(Modifier.padding(start = 20.dp, end = 20.dp, top = 20.dp, bottom = 8.dp)) {
            Text("Choose a Widget", style = Type.largeTitle, color = p.label)
            Spacer(Modifier.height(4.dp))
            Text("Pick which of your pages’ widgets this shows.", style = Type.subheadline, color = p.secondary)
        }
        if (!shown.signedIn) return@Column Unavailable("Not Signed In", "Sign in to ${BuildConfig.NAME} to see your pages’ widgets.")
        if (widgets.isEmpty()) return@Column Unavailable("No Widgets Yet", "Open ${BuildConfig.NAME} to see your pages, and their widgets show up here.")
        val pages = widgets.groupBy { it.scope }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            for ((_, here) in pages) {
                item {
                    Text(here.first().page, style = Type.footnote, color = p.secondary, modifier = Modifier.padding(start = 16.dp, top = 14.dp, bottom = 2.dp))
                }
                item {
                    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(p.card)) {
                        here.forEachIndexed { i, w ->
                            if (i > 0) HorizontalDivider(Modifier.padding(start = 60.dp), color = p.separator, thickness = 0.5.dp)
                            Row(
                                Modifier.fillMaxWidth().clickable { choose(w) }.padding(horizontal = 16.dp, vertical = 13.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Box(Modifier.padding(end = 14.dp)) { AppTile(TileLook(w.icon, w.app, w.color), 30.dp, w.title) }
                                Text(w.title, Modifier.weight(1f), style = Type.body, color = p.label, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                if (w.id == chosen) Symbol("check", p.accent, 18.dp)
                            }
                        }
                    }
                }
            }
        }
    }
}
