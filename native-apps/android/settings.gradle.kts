pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "android"
include(":app")
// Tauri's own Android library and its plugins', which the Rust build lists from what the app depends on.
apply(from = "tauri.settings.gradle")
