# kotlinx.serialization finds each class's serializer by its companion.
-keepclassmembers @kotlinx.serialization.Serializable class ** { *** Companion; }
-keepclasseswithmembers class ** { kotlinx.serialization.KSerializer serializer(...); }
# What the app's library calls by name (src/android.rs), and Tauri's plugin.
-keep class app.client.Host { *; }
-keep class app.client.Native { *; }
-keep class app.client.OnePlugin { *; }
