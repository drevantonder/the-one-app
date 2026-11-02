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

// The white label this build is: config.toml beside android/, or $CONFIG's.
val repo = rootDir.parentFile
val config = file(System.getenv("CONFIG") ?: repo.resolve("config.toml").path).absoluteFile

/** A top-level field: the [settings] after them are the install's, not the build's. */
fun field(name: String): String = config.readLines().takeWhile { !it.startsWith("[") }
    .firstNotNullOfOrNull { Regex("""^$name\s*=\s*"?([^"]*)"?\s*$""").find(it)?.groupValues?.get(1) } ?: ""

fun art(name: String) = config.parentFile.resolve(field(name))
fun quoted(s: String) = "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

val commit = providers.exec { commandLine("git", "-C", repo.path, "rev-parse", "--short=12", "HEAD"); isIgnoreExitValue = true }
    .standardOutput.asText.get().trim().ifEmpty { "unknown" }
val cli = field("cli")

// The launcher icon from the config's, and the logo the app shows; and Phosphor's icons, which apps name, as every
// device draws them.
val generated = layout.buildDirectory.dir("generated/white-label")
val whiteLabel = tasks.register("whiteLabel") {
    val icon = art("icon")
    val logo = art("logo")
    val icons = repo.resolve("vendor/phosphor/icons.json")
    inputs.files(icon, logo, icons, config)
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
    }
}

// Links to the install's address open the app (App Links), at the addresses it's built for: the white label's own, and
// $LINKS's (comma-separated), for one that asks where. Android takes them only from the manifest, and opens the app
// only where the address lists it (kernel set android), so they're in a manifest of their own, merged into the app's.
val links = (listOf(field("address")) + (System.getenv("LINKS") ?: "").split(","))
    .map { it.trim().trimEnd('/') }.filter { it.startsWith("https://") }.map { URI(it).host }.distinct()
val linksManifest = layout.buildDirectory.file("generated/links/AndroidManifest.xml").get().asFile.also { file ->
    // The web client's own places (kernel/web/web.js's route()), an invite and a Vault request; never `/`, which is
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
        buildConfigField("String", "ACCENT", quoted(field("accent")))
        // A white label with one install signs in there; otherwise the first screen asks where.
        buildConfigField("String", "ADDRESS", quoted(field("address")))
        buildConfigField("String", "COMMIT", quoted(commit))
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

tasks.named("preBuild") { dependsOn(whiteLabel) }

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
}
