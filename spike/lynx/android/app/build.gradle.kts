plugins { id("com.android.application") }

// -Plynx=false builds the same app without Lynx, to measure what Lynx adds to the APK.
val lynx = (findProperty("lynx") as String?) != "false"

android {
    namespace = "app.theone.lynxspike"
    compileSdk = 36
    defaultConfig {
        applicationId = "app.theone.lynxspike"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
        // -Pabi=arm64-v8a builds one ABI, as a phone downloads it from an App Bundle.
        (findProperty("abi") as String?)?.let { ndk { abiFilters += it } }
    }
    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    sourceSets["main"].java.directories.add(if (lynx) "src/lynx/java" else "src/nolynx/java")
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}

dependencies {
    if (lynx) {
        implementation("org.lynxsdk.lynx:lynx:4.2.0")
        implementation("org.lynxsdk.lynx:lynx-jssdk:4.2.0")
        implementation("org.lynxsdk.lynx:lynx-trace:4.2.0")
        implementation("org.lynxsdk.lynx:primjs:4.2.1")
        implementation("org.lynxsdk.lynx:lynx-service-image:4.2.0")
        implementation("com.facebook.fresco:fresco:2.3.0")
        implementation("org.lynxsdk.lynx:lynx-service-log:4.2.0")
        implementation("org.lynxsdk.lynx:xelement:4.2.0")
        implementation("org.lynxsdk.lynx:xelement-input:4.2.0")
        // Lynx brings androidx 1.0.0 libraries that AGP 9 refuses for sharing a namespace.
        implementation("androidx.vectordrawable:vectordrawable-animated:1.2.0")
        implementation("androidx.appcompat:appcompat:1.7.1")
    }
}
