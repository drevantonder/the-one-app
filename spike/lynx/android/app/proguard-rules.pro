# Lynx: its classes are reached from C++ and by generated setters (lynxjs.org's integration guide).
-keep class com.lynx.** { *; }
-keep class com.facebook.** { *; }
-keepclasseswithmembers,includedescriptorclasses class * { native <methods>; }
-dontwarn **
