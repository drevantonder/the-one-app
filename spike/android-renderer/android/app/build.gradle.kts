plugins { id("com.android.application") }

android {
    namespace = "app.theone.strictspike"
    compileSdk = 36
    defaultConfig {
        applicationId = "app.theone.strictspike"
        minSdk = 30
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
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
