# The NDK's compilers and linkers for Cargo's Android targets (QuickJS is C): `. ./env.sh` before `cargo build`.
NDK=$HOME/Android/Sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/bin
export CC_x86_64_linux_android=$NDK/x86_64-linux-android26-clang AR_x86_64_linux_android=$NDK/llvm-ar
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER=$NDK/x86_64-linux-android26-clang
export CC_aarch64_linux_android=$NDK/aarch64-linux-android26-clang AR_aarch64_linux_android=$NDK/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/aarch64-linux-android26-clang
# rquickjs ships no bindings for Android: bindgen makes them (its `bindgen` feature), against the NDK's headers.
export LIBCLANG_PATH=/usr/lib/llvm-18/lib
SYSROOT=$HOME/Android/Sdk/ndk/28.2.13676358/toolchains/llvm/prebuilt/linux-x86_64/sysroot
export BINDGEN_EXTRA_CLANG_ARGS_x86_64_linux_android="--sysroot=$SYSROOT --target=x86_64-linux-android26"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$SYSROOT --target=aarch64-linux-android26"
