package app.client

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

// What the node sends, as the app reads it (tests/contracts.rs checks these against what it does send). A field
// with a default may be left out; one without must be there.

val json = Json { ignoreUnknownKeys = true; explicitNulls = false; coerceInputValues = true; isLenient = true }

@Serializable
data class Config(
    val name: String,
    val cli: String,
    val accent: String,
    /** The app the App Store opens: the install's `store` setting. */
    val store: String? = null,
)

/** `GET /v1/state`: who this is, and the install. */
@Serializable
data class NodeState(
    val config: Config,
    @SerialName("me") val machine: Machine,
    @SerialName("server") val host: Machine? = null,
    val user: Person? = null,
    val install: Install? = null,
    @SerialName("vault_requests") val vaultRequests: Int? = null,
    @SerialName("secret_requests") val secretRequests: Int? = null,
    /** How this person arranged each page, by scope. */
    val arrangements: Map<String, Arrangement>? = null,
) {
    val waitingRequests get() = vaultRequests ?: secretRequests ?: 0
}

@Serializable
data class Arrangement(
    val items: List<ArrangementItem> = emptyList(),
    val hidden: List<String>? = null,
)

/** An app's icon on a page, or with `widget` a copy of one of its widgets, with its id, size and options. */
@Serializable
data class ArrangementItem(
    val app: String,
    val widget: String? = null,
    val id: String? = null,
    val size: String? = null,
    val options: Map<String, JsonElement>? = null,
)

@Serializable
data class Machine(
    val key: String,
    val name: String,
)

@Serializable
data class Person(
    val id: String,
    val name: String,
    val role: String,
) {
    val admin get() = role == "admin"
}

@Serializable
data class Install(
    val name: String,
    val users: List<Person>,
    val groups: List<GroupInfo>,
    val servers: List<Server>,
    val clients: List<ClientInfo>,
) {
    fun user(id: String) = users.firstOrNull { it.id == id }
    fun group(id: String) = groups.firstOrNull { it.id == id }
}

@Serializable
data class GroupInfo(
    val id: String,
    val name: String,
    val members: List<GroupMember>,
    val invited: List<GroupMember>? = null,
) {
    fun role(of: String) = members.firstOrNull { it.user == of }?.role
    val admins get() = members.filter { it.role == "admin" }.map { it.user }
}

@Serializable
data class GroupMember(
    val user: String,
    val role: String,
)

@Serializable
data class Server(
    val key: String,
    val name: String,
    val user: String,
    val online: Boolean? = null,
    /** `user:<id>`, `group:<id>` or `install`. */
    val owners: List<String>,
    val users: List<String>,
    val privileged: Boolean? = null,
    @SerialName("trusted_by") val trustedBy: List<String>? = null,
    val build: String? = null,
    val load: Load? = null,
)

@Serializable
data class Load(
    val memory: LoadSpace? = null,
    /** Percent busy, oldest first. */
    val cpu: List<Int>? = null,
) {
    /** "Memory 12.2 of 32 GB · CPU 12%". */
    val summary: String?
        get() {
            val parts = mutableListOf<String>()
            memory?.takeIf { it.total > 0 }?.let { parts += "Memory %.1f of %.1f GB".format(it.used / 1_073_741_824, it.total / 1_073_741_824) }
            cpu?.lastOrNull()?.let { parts += "CPU $it%" }
            return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
        }
}

@Serializable
data class LoadSpace(
    val used: Double,
    val total: Double,
)

@Serializable
data class ClientInfo(
    val key: String,
    val name: String,
    val user: String,
    /** When it last asked, in milliseconds. */
    val asked: Double? = null,
)

/** An app as `GET /v1/apps` describes it: only what the app needs to show it. */
@Serializable
data class AppInfo(
    val name: String,
    val title: String? = null,
    val description: String? = null,
    val icon: String? = null,
    val color: String? = null,
    /** Its own picture, as a data URL; `image_kind` "icon" when it's a finished app icon, else a logo. */
    val image: String? = null,
    @SerialName("image_kind") val imageKind: String? = null,
    val owner: String? = null,
    val screens: List<AppScreenInfo>? = null,
    val ops: List<AppOp>? = null,
    val widgets: List<AppWidget>? = null,
    /** The scopes it's installed in that this person may use: `user:<id>`, `group:<id>`, `global`. */
    val installed: List<String>? = null,
    val deploys: Int? = null,
    val vault: Map<String, VaultNeed>? = null,
) {
    val label get() = title ?: name
    fun has(screen: String) = screens?.any { it.name == screen } == true
    fun hasOp(op: String) = ops?.any { it.name == op } == true
    /** Only apps with a `main` screen get an icon; the rest still give widgets and tools. */
    val hasScreens get() = has("main")
    /** Nothing to open and no widget: it goes in a page's Services. */
    val isService get() = !hasScreens && widgets.isNullOrEmpty()
    /** Its pages: this person's own scope is the page `user`. */
    val scopes get() = installed.orEmpty().map { if (it.startsWith("user:")) "user" else it }
    val keepsAccounts get() = vault?.values?.any { it.accounts == true } == true
}

@Serializable
data class AppScreenInfo(
    val name: String,
    val title: String? = null,
)

@Serializable
data class AppOp(
    val name: String,
)

@Serializable
data class AppWidget(
    val name: String,
)

@Serializable
data class VaultNeed(
    val title: String? = null,
    val accounts: Boolean? = null,
)

@Serializable
data class NodeEvent(
    val type: String,
    val apps: List<String>? = null,
)

/** An item of the Vault as `GET /v1/vault` lists it: a secret field never with its value. */
@Serializable
data class VaultItem(
    val id: String,
    val scope: String,
    val title: String,
    val type: String? = null,
    val apps: List<String> = emptyList(),
    @SerialName("wanted_by") val wantedBy: List<String>? = null,
)

/** A request for an item an app may use: the person picks one of theirs or makes one. */
@Serializable
data class VaultRequest(
    val id: String,
    val scope: String,
    /** The scope's owner: a person's name, a group's, or the install's. */
    @SerialName("scope_name") val scopeName: String,
    val app: String,
    val title: String? = null,
    val fields: List<VaultField>? = null,
    /** The one secret it asks for, as nodes before the Vault asked. */
    val name: String? = null,
    val hosts: List<String>,
    val why: String? = null,
    val by: String,
    val agent: String? = null,
    /** `waiting`, `set`, `declined` or `expired`. */
    val state: String,
    val matches: List<VaultMatch>? = null,
) {
    val label get() = title ?: name ?: ""
    val asker get() = agent?.ifEmpty { null } ?: by
    val wanted get() = fields ?: name?.let { listOf(VaultField(it, true)) } ?: emptyList()
}

@Serializable
data class VaultField(
    val name: String,
    val secret: Boolean,
)

@Serializable
data class VaultMatch(
    val id: String,
    val title: String,
    val type: String? = null,
)

/** A widget as `GET /v1/widgets` gives it: its tree at each size it draws. */
@Serializable
data class WidgetInfo(
    val app: String,
    val widget: String,
    val title: String? = null,
    /** The size a page starts it at: `small`, `medium`, `large` or `xl`. */
    val size: String? = null,
    val sizes: Map<String, WidgetNode>? = null,
    /** Why its render failed, if it did. */
    val error: String? = null,
    val root: WidgetNode,
    /** Each copy this person put on a page with options of their own, by its id. */
    val placed: Map<String, WidgetPlaced>? = null,
) {
    /** The sizes it draws, smallest first. */
    val supported get() = WIDGET_SIZES.filter { sizes == null || sizes.containsKey(it) }
    fun tree(size: String) = sizes?.get(size) ?: root
}

@Serializable
data class WidgetPlaced(
    val sizes: Map<String, WidgetNode>? = null,
    val error: String? = null,
)

val WIDGET_SIZES = listOf("small", "medium", "large", "xl")

/** A node of a widget's tree, `t` saying what it is (WidgetTree.kt draws each). Decoded leniently: a field of the
 *  wrong shape is left out, and so is a child that isn't a node. */
@Serializable(with = WidgetNodeSerializer::class)
data class WidgetNode(
    val t: String,
    val gap: Double? = null,
    val c: List<WidgetNode>? = null,
    val text: String? = null,
    val size: String? = null,
    val weight: String? = null,
    val tone: String? = null,
    val lines: Int? = null,
    val at: Double? = null,
    val style: String? = null,
    val value: JsonElement? = null,
    val max: Double? = null,
    val label: String? = null,
    val unit: String? = null,
    val values: List<Double>? = null,
    val labels: List<String>? = null,
    val kind: String? = null,
    val title: String? = null,
    val subtitle: String? = null,
    val trailing: String? = null,
    val icon: String? = null,
    val name: String? = null,
    val url: String? = null,
    val w: Double? = null,
    val h: Double? = null,
    val round: Boolean? = null,
    val empty: String? = null,
    val link: JsonElement? = null,
    val op: String? = null,
    val input: JsonElement? = null,
    val on: Boolean? = null,
    val next: Double? = null,
    val grow: Boolean? = null,
    val align: String? = null,
    val dim: Boolean? = null,
    val badge: String? = null,
    val header: Boolean? = null,
    val path: String? = null,
)

object WidgetNodeSerializer : KSerializer<WidgetNode> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor("WidgetNode")

    override fun serialize(encoder: Encoder, value: WidgetNode) {
        (encoder as JsonEncoder).encodeJsonElement(JsonObject(mapOf("t" to JsonPrimitive(value.t))))
    }

    override fun deserialize(decoder: Decoder): WidgetNode = node((decoder as JsonDecoder).decodeJsonElement())
        ?: throw kotlinx.serialization.SerializationException("not a widget node")

    fun node(e: JsonElement): WidgetNode? {
        val o = e as? JsonObject ?: return null
        val t = o.str("t") ?: return null
        fun any(k: String) = o[k]?.takeIf { it !is JsonNull }
        return WidgetNode(
            t = t, gap = o.num("gap"), c = (o["c"] as? JsonArray)?.mapNotNull(::node), text = o.str("text"), size = o.str("size"),
            weight = o.str("weight"), tone = o.str("tone"), lines = o.num("lines")?.toInt(), at = o.num("at"), style = o.str("style"),
            value = any("value"), max = o.num("max"), label = o.str("label"), unit = o.str("unit"),
            values = (o["values"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.doubleOrNull },
            labels = (o["labels"] as? JsonArray)?.mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.content },
            kind = o.str("kind"), title = o.str("title"), subtitle = o.str("subtitle"), trailing = o.str("trailing"), icon = o.str("icon"),
            name = o.str("name"), url = o.str("url"), w = o.num("w"), h = o.num("h"), round = o.bool("round"), empty = o.str("empty"),
            link = any("link"), op = o.str("op"), input = any("input"), on = o.bool("on"), next = o.num("next"), grow = o.bool("grow"),
            align = o.str("align"), dim = o.bool("dim"), badge = o.str("badge"), header = o.bool("header"), path = o.str("path"),
        )
    }

    private fun JsonObject.str(k: String) = (this[k] as? JsonPrimitive)?.takeIf { it.isString }?.content
    private fun JsonObject.num(k: String) = (this[k] as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull
    private fun JsonObject.bool(k: String) = (this[k] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull
}

/** A value as text: a string as it is, a number without a needless ".0". */
val JsonElement?.text: String?
    get() {
        val p = this as? JsonPrimitive ?: return null
        if (p is JsonNull) return null
        if (p.isString) return p.content
        p.longOrNull?.let { return it.toString() }
        p.doubleOrNull?.let { d -> return if (d == Math.floor(d) && !d.isInfinite()) d.toLong().toString() else "%.2f".format(d).trimEnd('0').trimEnd('.') }
        p.booleanOrNull?.let { return it.toString() }
        return p.content
    }

val JsonElement?.number: Double? get() = (this as? JsonPrimitive)?.takeIf { it !is JsonNull }?.let { it.doubleOrNull ?: it.intOrNull?.toDouble() }

/** What went wrong, in words a person reads: an app's JavaScript error's first line. */
class NodeError(raw: String, val signedOut: Boolean = false) : Exception(clean(raw)) {
    companion object {
        fun clean(raw: String): String {
            var line = raw.lineSequence().firstOrNull() ?: raw
            if (line.startsWith("Error: ")) line = line.removePrefix("Error: ")
            return line.replaceFirstChar { it.uppercase() }
        }
    }
}
