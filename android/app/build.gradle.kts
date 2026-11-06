import java.awt.Color as AwtColor
import java.net.URI
import java.awt.RenderingHints
import java.awt.image.BufferedImage
import javax.imageio.ImageIO

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// The custom build this build is: config.toml beside android/, or $CONFIG's.
val repo = rootDir.parentFile
val config = file(System.getenv("CONFIG") ?: repo.resolve("config.toml").path).absoluteFile

/** A top-level field: the [settings] after them are the network's, not the build's. */
fun field(name: String): String = config.readLines().takeWhile { !it.startsWith("[") }
    .firstNotNullOfOrNull { Regex("""^$name\s*=\s*"?([^"]*)"?\s*$""").find(it)?.groupValues?.get(1) } ?: ""

fun art(name: String) = config.parentFile.resolve(field(name))
fun quoted(s: String) = "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

val commit = providers.exec { commandLine("git", "-C", repo.path, "rev-parse", "--short=12", "HEAD"); isIgnoreExitValue = true }
    .standardOutput.asText.get().trim().ifEmpty { "unknown" }
val cli = field("cli")
// What a person calls the network where they read: a workplace's custom build, which its people join with their work
// email (join_domain), is shared, "the PTC App network"; anyone else's is "your One App network".
val yours = field("name").let { if (field("join_domain").isNotEmpty()) it.replaceFirst(Regex("^The "), "the ") else "your " + it.removePrefix("The ") } + " network"

// The launcher icon from the config's, and the logo the app shows; and Phosphor's icons, which apps name, as every
// device draws them.
val generated = layout.buildDirectory.dir("generated/custom-build")
val customBuild = tasks.register("customBuild") {
    val icon = art("icon")
    val logo = art("logo")
    val icons = repo.resolve("vendor/phosphor/icons.json")
    val menubar = art("menubar").let { it.resolveSibling(it.nameWithoutExtension + "@2x." + it.extension) }
    val firebase = field("firebase").ifEmpty { null }?.let { config.parentFile.resolve(it) }
    inputs.files(listOfNotNull(icon, logo, icons, config, menubar, firebase))
    outputs.dir(generated)
    doLast {
        val out = generated.get().asFile
        out.deleteRecursively()
        // The icon is drawn on Apple's Mac grid, an 824-pixel tile in 1024: that tile, its clear corners the colour of
        // its edge, fills an adaptive icon's middle, on that colour.
        val source = ImageIO.read(icon)
        val scale = source.width / 1024.0
        val inset = (100 * scale).toInt()
        val tile = source.getSubimage(inset, inset, source.width - 2 * inset, source.height - 2 * inset)
        val edge = tile.getRGB(tile.width / 2, 4) or (0xFF shl 24)
        for ((dir, px) in listOf("mipmap-mdpi" to 108, "mipmap-hdpi" to 162, "mipmap-xhdpi" to 216, "mipmap-xxhdpi" to 324, "mipmap-xxxhdpi" to 432)) {
            val image = BufferedImage(px, px, BufferedImage.TYPE_INT_ARGB)
            val g = image.createGraphics()
            g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC)
            g.setRenderingHint(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY)
            g.color = AwtColor(edge, true)
            g.fillRect(0, 0, px, px)
            val size = px * 76 / 108
            g.drawImage(tile, (px - size) / 2, (px - size) / 2, size, size, null)
            g.dispose()
            out.resolve("res/$dir").mkdirs()
            ImageIO.write(image, "png", out.resolve("res/$dir/ic_launcher_foreground.png"))
        }
        out.resolve("res/values").mkdirs()
        out.resolve("res/values/launcher.xml").writeText(
            "<resources><color name=\"launcher_background\">#%06X</color></resources>\n".format(edge and 0xFFFFFF)
        )
        out.resolve("res/drawable-nodpi").mkdirs()
        logo.copyTo(out.resolve("res/drawable-nodpi/logo.png"))
        out.resolve("assets").mkdirs()
        icons.copyTo(out.resolve("assets/icons.json"))
        // A notification's small icon is drawn from its shape alone: the menu bar's template, which is just that.
        out.resolve("res/drawable-xhdpi").mkdirs()
        menubar.copyTo(out.resolve("res/drawable-xhdpi/notification.png"))
        // The custom build's Firebase project, which this app gets its pushes through, as Firebase reads it at start:
        // its google-services.json's values for this app, as Google's own Gradle plugin would write them. None, none.
        if (firebase != null) {
            @Suppress("UNCHECKED_CAST")
            val services = groovy.json.JsonSlurper().parse(firebase) as Map<String, Any?>
            val project = services["project_info"] as Map<String, Any?>
            val id = "app.$cli.android"
            @Suppress("UNCHECKED_CAST")
            val app = (services["client"] as List<Map<String, Any?>>).firstOrNull { c ->
                ((c["client_info"] as Map<*, *>)["android_client_info"] as Map<*, *>)["package_name"] == id
            } ?: throw GradleException("$firebase has no Android app $id: add one to its Firebase project, then download its google-services.json again")
            val key = ((app["api_key"] as List<*>).first() as Map<*, *>)["current_key"]
            val values = mapOf(
                "google_app_id" to (app["client_info"] as Map<*, *>)["mobilesdk_app_id"],
                "gcm_defaultSenderId" to project["project_number"],
                "google_api_key" to key,
                "project_id" to project["project_id"],
            )
            out.resolve("res/values/firebase.xml").writeText("<resources>\n" +
                values.entries.joinToString("") { (k, v) -> "    <string name=\"$k\" translatable=\"false\">$v</string>\n" } + "</resources>\n")
        }
    }
}

// Links to the network's address open the app (App Links), at the addresses it's built for: the custom build's own, and
// $LINKS's (comma-separated), for one that asks where. Android takes them only from the manifest, and opens the app
// only where the address lists it (cloudflare set android), so they're in a manifest of their own, merged into the app's.
val links = (listOf(field("address")) + (System.getenv("LINKS") ?: "").split(","))
    .map { it.trim().trimEnd('/') }.filter { it.startsWith("https://") }.map { URI(it).host }.distinct()
val linksManifest = layout.buildDirectory.file("generated/links/AndroidManifest.xml").get().asFile.also { file ->
    // The web client's own places (cloudflare/web/web.js's route()), an invite and a Vault request; never `/`, which is
    // the web client itself.
    val paths = listOf("path" to "/invite", "path" to "/vault", "path" to "/settings", "pathPrefix" to "/settings/",
        "pathPrefix" to "/page/", "pathPrefix" to "/services/", "pathPrefix" to "/app/")
    val data = (links.map { "android:host=\"$it\"" } + paths.map { (k, v) -> "android:$k=\"$v\"" }).joinToString("\n") { "                <data $it />" }
    val filter = if (links.isEmpty()) "" else """
        <activity android:name="app.client.MainActivity" android:exported="true">
            <intent-filter android:autoVerify="true">
                <action android:name="android.intent.action.VIEW" />
                <category android:name="android.intent.category.DEFAULT" />
                <category android:name="android.intent.category.BROWSABLE" />
                <data android:scheme="https" />
$data
            </intent-filter>
        </activity>"""
    val text = """<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application>$filter
    </application>
</manifest>
"""
    if (!file.exists() || file.readText() != text) file.apply { parentFile.mkdirs() }.writeText(text)
}

android {
    namespace = "app.client"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.$cli.android"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = commit
        buildConfigField("String", "NAME", quoted(field("name")))
        buildConfigField("String", "CLI", quoted(cli))
        buildConfigField("String", "YOURS", quoted(yours))
        // A custom build with one network signs in there; otherwise the first screen asks where.
        buildConfigField("String", "ADDRESS", quoted(field("address")))
        manifestPlaceholders["scheme"] = cli
        resValue("string", "app_name", field("name"))
    }

    signingConfigs {
        // Signed with the debug key until the app has a release key of its own.
        getByName("debug")
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    sourceSets["debug"].manifest.srcFile(linksManifest)
    sourceSets["release"].manifest.srcFile(linksManifest)
    sourceSets["main"].res.srcDir(generated.map { it.dir("res") })
    sourceSets["main"].assets.srcDir(generated.map { it.dir("assets") })
    // The client library and its Kotlin, which build.sh makes (client/ffi).
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("generated/client/jniLibs"))
    sourceSets["main"].kotlin.srcDir(layout.buildDirectory.dir("generated/client/kotlin"))

    buildFeatures {
        compose = true
        buildConfig = true
        resValues = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

tasks.named("preBuild") { dependsOn(customBuild) }

dependencies {
    val compose = platform("androidx.compose:compose-bom:2025.10.01")
    implementation(compose)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.activity:activity-compose:1.11.0")
    implementation("androidx.browser:browser:1.9.0")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
    implementation("androidx.glance:glance-appwidget:1.2.0")
    implementation("androidx.work:work-runtime:2.12.0")
    implementation("com.google.android.gms:play-services-mlkit-document-scanner:16.0.0")
    // Pushes (FCM): Google's own client is the only way an app gets one on Android, woken as its notes arrive.
    implementation("com.google.firebase:firebase-messaging:26.0.1")
    // The client library's Kotlin (uniffi's) calls into it through JNA.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
}
