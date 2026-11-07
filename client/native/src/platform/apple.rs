//! What the Mac and the iPhone share: their bundle's Info.plist.

/// A string in the bundle's Info.plist (`AppGroup`, `KeychainGroup`), if it has one.
pub fn info(key: &str) -> Option<String> {
    use core_foundation::string::CFString;
    let info = core_foundation::bundle::CFBundle::main_bundle().info_dictionary();
    let value = info.find(CFString::new(key))?;
    value.downcast::<CFString>().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// Whether the bundle's Info.plist says `key` is true (`Push`, in a build with pushes).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn info_true(key: &str) -> bool {
    use core_foundation::string::CFString;
    let info = core_foundation::bundle::CFBundle::main_bundle().info_dictionary();
    let value = info.find(CFString::new(key));
    value.and_then(|v| v.downcast::<core_foundation::boolean::CFBoolean>()).is_some_and(bool::from)
}
