# kotlinx.serialization finds each class's serializer by its companion.
-keepclassmembers @kotlinx.serialization.Serializable class ** { *** Companion; }
-keepclasseswithmembers class ** { kotlinx.serialization.KSerializer serializer(...); }
# The client library's bindings, which JNA calls by name.
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.** { *; }
-keep class uniffi.** { *; }
# JNA names desktop Java (AWT), which Android has none of and the bindings never use.
-dontwarn java.awt.**
