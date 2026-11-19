// The key names from Zed's GPUI (github.com/zed-industries/zed, crates/gpui/src/platform/windows/events.rs,
// `parse_normal_key` and `current_modifiers`) at commit 69e2130295c2649963eb639fc70b4f2ee8ea1624 (gpui 0.2.2 on
// crates.io), named here as the DOM's `KeyboardEvent.key` names them, which the renderer's `keydown` and the
// inspector speak. What a key types is WM_CHAR's (`window`); this names the keys that type nothing, and a letter or
// digit held with Ctrl or Alt, which types nothing either.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/renderer/LICENSE-APACHE); this file is changed from the original.

use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::inspect::Key;

/// Key `vkey` pressed, if it's one that types nothing by itself: a named key, or a letter or digit with Ctrl or Alt.
pub fn key(vkey: VIRTUAL_KEY) -> Option<Key> {
    let modifiers = modifiers();
    let named = match vkey {
        VK_BACK => "Backspace",
        VK_RETURN => "Enter",
        VK_TAB => "Tab",
        VK_UP => "ArrowUp",
        VK_DOWN => "ArrowDown",
        VK_RIGHT => "ArrowRight",
        VK_LEFT => "ArrowLeft",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_ESCAPE => "Escape",
        VK_INSERT => "Insert",
        VK_DELETE => "Delete",
        VK_APPS => "ContextMenu",
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => "Shift",
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => "Control",
        VK_MENU | VK_LMENU | VK_RMENU => "Alt",
        VK_LWIN | VK_RWIN => "Meta",
        _ => "",
    };
    let key = if !named.is_empty() {
        named.to_owned()
    } else if (VK_F1.0..=VK_F24.0).contains(&vkey.0) {
        format!("F{}", vkey.0 - VK_F1.0 + 1)
    } else if (modifiers.ctrl || modifiers.alt) && matches!(u8::try_from(vkey.0), Ok(b'A'..=b'Z' | b'0'..=b'9' | b' ')) {
        // A letter, digit or space with Ctrl or Alt: the key's own character, as the DOM has it.
        char::from(vkey.0 as u8).to_ascii_lowercase().to_string()
    } else {
        return None;
    };
    Some(Key { key, ..modifiers })
}

/// What `typed` types, with the modifiers held.
pub fn typed(typed: String) -> Key {
    Key { key: typed, ..modifiers() }
}

fn is_virtual_key_pressed(vkey: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vkey.0 as i32) < 0 }
}

/// The modifiers held, with AltGr (Windows' right Alt and left Ctrl together) neither.
fn modifiers() -> Key {
    let altgr = is_virtual_key_pressed(VK_RMENU) && is_virtual_key_pressed(VK_LCONTROL);
    Key {
        key: String::new(),
        ctrl: is_virtual_key_pressed(VK_CONTROL) && !altgr,
        alt: is_virtual_key_pressed(VK_MENU) && !altgr,
        shift: is_virtual_key_pressed(VK_SHIFT),
        meta: is_virtual_key_pressed(VK_LWIN) || is_virtual_key_pressed(VK_RWIN),
    }
}
