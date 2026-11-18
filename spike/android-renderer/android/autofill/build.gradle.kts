plugins { id("com.android.application") }

// A test autofill service, as a password manager is one: it offers a username and a password for any login form, and
// logs what a form asks it to save. `adb shell settings put secure autofill_service app.theone.testautofill/.TestAutofillService`
android {
    namespace = "app.theone.testautofill"
    compileSdk = 36
    defaultConfig {
        applicationId = "app.theone.testautofill"
        minSdk = 30
        targetSdk = 36
        versionCode = 1
        versionName = "0.1"
    }
    buildTypes { release { signingConfig = signingConfigs.getByName("debug") } }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
