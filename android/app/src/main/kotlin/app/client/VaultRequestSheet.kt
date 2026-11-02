package app.client

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AddCircle
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/** A request for an item an app may use: who asks and why, then the person's items that have the fields it wants, to
 *  pick one, or a new one, whose values go to the node and nowhere else. The asker never sees a secret. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun VaultRequestSheet(store: Store, sheet: VaultSheet) {
    val p = LocalPalette.current
    var request by remember(sheet) { mutableStateOf<VaultRequest?>(null) }
    var error by remember(sheet) { mutableStateOf<String?>(null) }
    var picked by remember(sheet) { mutableStateOf<String?>(null) }
    var title by remember(sheet) { mutableStateOf("") }
    val values = remember(sheet) { mutableStateMapOf<String, String>() }
    var busy by remember(sheet) { mutableStateOf(false) }
    val close = { if (store.sheet == sheet) store.sheet = null; sheet.done?.invoke() }

    LaunchedEffect(sheet) {
        try {
            val r = store.vaultRequests(sheet.scope, sheet.id).firstOrNull()
            if (r == null) error = "It isn’t for you, or it’s from another install."
            else {
                request = r
                picked = r.matches?.firstOrNull()?.id
                title = r.title ?: ""
            }
        } catch (e: Exception) {
            error = e.message
        }
        // Answered somewhere else (another device, an agent's CLI): the sheet goes.
        while (request?.state == "waiting") {
            delay(2100)
            if (busy) continue
            val now = runCatching { store.vaultRequests(sheet.scope, sheet.id).firstOrNull() }.getOrNull()
            if (now != null && now.state != "waiting") close()
        }
    }

    fun answer(declined: Boolean) {
        val r = request ?: return
        busy = true
        error = null
        val body = mutableMapOf<String, Any?>("scope" to r.scope, "id" to r.id)
        when {
            declined -> body["declined"] = true
            picked != null -> body["item"] = picked
            else -> {
                if (title.isNotBlank()) body["title"] = title.trim()
                body["fields"] = JsonObject(values.mapValues { JsonPrimitive(it.value) })
            }
        }
        store.launch {
            try {
                store.post("/v1/vault/answer", obj(*body.toList().toTypedArray()))
                values.clear()
                close()
                runCatching { store.refreshState() }
            } catch (e: Exception) {
                error = e.message
                busy = false
            }
        }
    }

    ModalBottomSheet({ close() }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = p.background) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = 32.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
            val r = request
            val waiting = r?.state == "waiting"
            val ready = picked != null || r?.wanted?.all { !values[it.name].isNullOrEmpty() } == true
            Box(Modifier.fillMaxWidth()) {
                TextButton({ close() }, Modifier.align(Alignment.CenterStart)) { Text(if (waiting) "Cancel" else "Done", color = p.accent) }
                Text("Vault", style = Type.headline, color = p.label, modifier = Modifier.align(Alignment.Center))
                if (waiting) TextButton({ answer(false) }, Modifier.align(Alignment.CenterEnd), enabled = ready && !busy) {
                    Text("Allow", color = if (ready && !busy) p.accent else p.tertiary, fontWeight = FontWeight.SemiBold)
                }
            }
            if (r == null) {
                if (error != null) Unavailable("Can’t Find This Request", error!!)
                else Box(Modifier.fillMaxWidth().padding(40.dp), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = p.secondary) }
                return@Column
            }
            val app = store.app(r.app)
            Section(footer = if (r.agent != null) "${r.asker} · ${r.scopeName}" else r.scopeName) {
                ListRow(icon = { AppTile(app, r.app, 44.dp) }, title = app?.let { "${it.label} wants ${r.label}" } ?: r.label, subtitle = r.why?.ifEmpty { null })
            }
            if (!waiting) {
                Section { ListRow(title = when (r.state) { "set" -> "Allowed."; "declined" -> "Declined."; else -> "This request expired. Ask ${r.asker} to ask again." }) }
                return@Column
            }
            Section {
                r.matches.orEmpty().forEachIndexed { i, m ->
                    Divided(i == 0)
                    ListRow(icon = { AppTile(TileLook(null, m.title), 30.dp, m.title) }, title = m.title, subtitle = m.type?.ifEmpty { null },
                        end = { if (picked == m.id) Icon(Icons.Rounded.Check, null, tint = p.accent) }) { picked = m.id }
                }
                Divided(r.matches.isNullOrEmpty())
                ListRow(icon = { Icon(Icons.Rounded.AddCircle, null, Modifier.size(30.dp), tint = p.accent) }, title = "New Item",
                    end = { if (picked == null) Icon(Icons.Rounded.Check, null, tint = p.accent) }) { picked = null }
            }
            if (picked == null) {
                Section(footer = r.hosts.takeIf { it.isNotEmpty() }?.let { "Sent only to ${it.joinToString(", ")}" }) {
                    Field("Title", title, secret = false) { title = it }
                    for (f in r.wanted) {
                        Divided(false)
                        Field(fieldLabel(f.name), values[f.name] ?: "", f.secret) { values[f.name] = it }
                    }
                }
            }
            Section { ListRow(title = "Decline", color = p.red, onClick = { if (!busy) answer(true) }) }
            error?.let { Text(it, color = p.red, style = Type.subheadline, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth()) }
        }
    }
}

/** A field to type into, a secret's hidden. */
@Composable
private fun Field(label: String, value: String, secret: Boolean, change: (String) -> Unit) {
    val p = LocalPalette.current
    TextField(
        value, change, Modifier.fillMaxWidth(), label = { Text(label) }, singleLine = true,
        visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(keyboardType = if (secret) KeyboardType.Password else KeyboardType.Text, autoCorrectEnabled = false),
        colors = TextFieldDefaults.colors(focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
            focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent, focusedLabelColor = p.accent, cursorColor = p.accent),
    )
}

/** A field's name as a person reads it: "api_key" → "API key". */
fun fieldLabel(name: String): String {
    val words = name.replace(Regex("[_-]+"), " ").trim().split(Regex("\\s+"))
    return words.mapIndexed { i, w ->
        if (Regex("^(api|url|id|ssh|pem|aws|s3|ip)$", RegexOption.IGNORE_CASE).matches(w)) w.uppercase()
        else if (i > 0) w.lowercase() else w.lowercase().replaceFirstChar { it.uppercase() }
    }.joinToString(" ")
}
