// The key names from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/mac/events.rs,
// `parse_keystroke`) at commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on crates.io), named here as the
// DOM's `KeyboardEvent.key` names them, which the strict protocol's `keydown` and the inspector speak, and read with
// objc2's AppKit bindings. What a key types is what AppKit says it types (`characters`), not GPUI's own layout
// lookup: the strict host has no key bindings to match.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use objc2_app_kit::{
    NSDeleteFunctionKey, NSDownArrowFunctionKey, NSEndFunctionKey, NSEvent, NSEventModifierFlags, NSF1FunctionKey, NSF35FunctionKey,
    NSHelpFunctionKey, NSHomeFunctionKey, NSLeftArrowFunctionKey, NSPageDownFunctionKey, NSPageUpFunctionKey, NSRightArrowFunctionKey,
    NSUpArrowFunctionKey,
};

use crate::inspect::Key;

const BACKSPACE_KEY: u16 = 0x7f;
const ENTER_KEY: u16 = 0x0d;
const NUMPAD_ENTER_KEY: u16 = 0x03;
const ESCAPE_KEY: u16 = 0x1b;
const TAB_KEY: u16 = 0x09;
const SHIFT_TAB_KEY: u16 = 0x19;

/// The key `event` is, with the modifiers held: a named key (Enter, ArrowLeft), else the character it types, or with
/// ⌘ or Control the key's own character, as the DOM has it (⌘A's `a`).
pub fn key(event: &NSEvent) -> Key {
    let flags = event.modifierFlags();
    let held = |flag: NSEventModifierFlags| flags.contains(flag);
    let (shift, alt, ctrl, meta) =
        (held(NSEventModifierFlags::Shift), held(NSEventModifierFlags::Option), held(NSEventModifierFlags::Control), held(NSEventModifierFlags::Command));
    let ignoring = event.charactersIgnoringModifiers().map(|characters| characters.to_string()).unwrap_or_default();
    let first = ignoring.encode_utf16().next();
    #[allow(non_upper_case_globals)]
    let named = match first.map(u32::from) {
        Some(code) if code == u32::from(ENTER_KEY) || code == u32::from(NUMPAD_ENTER_KEY) => "Enter",
        Some(code) if code == u32::from(TAB_KEY) || code == u32::from(SHIFT_TAB_KEY) => "Tab",
        Some(code) if code == u32::from(BACKSPACE_KEY) => "Backspace",
        Some(code) if code == u32::from(ESCAPE_KEY) => "Escape",
        Some(NSUpArrowFunctionKey) => "ArrowUp",
        Some(NSDownArrowFunctionKey) => "ArrowDown",
        Some(NSLeftArrowFunctionKey) => "ArrowLeft",
        Some(NSRightArrowFunctionKey) => "ArrowRight",
        Some(NSPageUpFunctionKey) => "PageUp",
        Some(NSPageDownFunctionKey) => "PageDown",
        Some(NSHomeFunctionKey) => "Home",
        Some(NSEndFunctionKey) => "End",
        Some(NSDeleteFunctionKey) => "Delete",
        // Observed Insert==NSHelpFunctionKey not NSInsertFunctionKey.
        Some(NSHelpFunctionKey) => "Insert",
        _ => "",
    };
    let key = match first.map(u32::from) {
        _ if !named.is_empty() => named.to_owned(),
        Some(code @ NSF1FunctionKey..=NSF35FunctionKey) => format!("F{}", code - NSF1FunctionKey + 1),
        // A key with ⌘ or Control types nothing of its own: the key's own character, as the DOM has it.
        _ if ctrl || meta => ignoring,
        _ => event.characters().map(|characters| characters.to_string()).unwrap_or_default(),
    };
    Key { key, shift, alt, ctrl, meta }
}
