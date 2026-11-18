// The key names from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_linux/src/linux/platform.rs,
// `keystroke_from_xkb`, and x11/client.rs, `xkb_state_for_key_event`) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, named here as the DOM's `KeyboardEvent.key` names them, which the strict
// protocol's `keydown` and the inspector speak.
// Copyright 2022 - 2025 Zed Industries, Inc. Licensed under the Apache License, Version 2.0
// (clients/app/strict/LICENSE-APACHE); this file is changed from the original.

use x11rb::protocol::xproto;
use xkbcommon::xkb::{self, Keycode, Keysym, State};

use crate::inspect::Key;

/// The key `keycode` is, with the modifiers held, as xkbcommon's state has them.
pub fn key(state: &State, keycode: Keycode) -> Key {
    let sym = state.key_get_one_sym(keycode);
    let typed = state.key_get_utf8(keycode);
    let named = match sym {
        Keysym::Return | Keysym::KP_Enter => "Enter",
        Keysym::Tab | Keysym::ISO_Left_Tab => "Tab",
        Keysym::BackSpace => "Backspace",
        Keysym::Delete | Keysym::KP_Delete => "Delete",
        Keysym::Escape => "Escape",
        Keysym::Left | Keysym::KP_Left => "ArrowLeft",
        Keysym::Right | Keysym::KP_Right => "ArrowRight",
        Keysym::Up | Keysym::KP_Up => "ArrowUp",
        Keysym::Down | Keysym::KP_Down => "ArrowDown",
        Keysym::Home | Keysym::KP_Home => "Home",
        Keysym::End | Keysym::KP_End => "End",
        Keysym::Prior | Keysym::KP_Prior => "PageUp",
        Keysym::Next | Keysym::KP_Next => "PageDown",
        Keysym::Insert => "Insert",
        Keysym::Shift_L | Keysym::Shift_R => "Shift",
        Keysym::Control_L | Keysym::Control_R => "Control",
        Keysym::Alt_L | Keysym::Alt_R => "Alt",
        Keysym::Super_L | Keysym::Super_R => "Meta",
        _ => "",
    };
    let key = if !named.is_empty() {
        named.to_owned()
    } else if (0xffbe..=0xffe0).contains(&sym.raw()) {
        format!("F{}", sym.raw() - 0xffbe + 1)
    } else if !typed.is_empty() && !typed.chars().any(char::is_control) {
        typed
    } else {
        // A key with a modifier that types nothing of its own (Ctrl-W): the key's own character, as the DOM has it.
        let lower = xkb::keysym_to_utf32(sym);
        char::from_u32(lower).filter(|c| !c.is_control() && *c != '\0').map_or_else(|| "Unidentified".to_owned(), |c| c.to_string())
    };
    let held = |name: &str| state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
    Key { key, shift: held(xkb::MOD_NAME_SHIFT), alt: held(xkb::MOD_NAME_ALT), ctrl: held(xkb::MOD_NAME_CTRL), meta: held(xkb::MOD_NAME_LOGO) }
}

/// The keyboard's state as a key event says it was when it was pressed: its modifiers from the event, the rest from
/// `xkb`. An input method hands a key back after its modifiers may have come up, as Shift has by the time "A" comes
/// back.
pub fn state_of(xkb: &State, event_state: xproto::KeyButMask) -> State {
    let keymap = xkb.get_keymap();
    let mut key_event_state = State::new(&keymap);
    let latched_modifiers = xkb.serialize_mods(xkb::STATE_MODS_LATCHED);
    let locked_modifiers = xkb.serialize_mods(xkb::STATE_MODS_LOCKED);
    let active_modifier_mask: xkb::ModMask = u16::from(
        event_state
            & (xproto::KeyButMask::SHIFT
                | xproto::KeyButMask::LOCK
                | xproto::KeyButMask::CONTROL
                | xproto::KeyButMask::MOD1
                | xproto::KeyButMask::MOD2
                | xproto::KeyButMask::MOD3
                | xproto::KeyButMask::MOD4
                | xproto::KeyButMask::MOD5),
    )
    .into();
    let depressed_modifiers = active_modifier_mask & !(latched_modifiers | locked_modifiers);
    key_event_state.update_mask(
        depressed_modifiers,
        latched_modifiers,
        locked_modifiers,
        xkb.serialize_layout(xkb::STATE_LAYOUT_DEPRESSED),
        xkb.serialize_layout(xkb::STATE_LAYOUT_LATCHED),
        xkb.serialize_layout(xkb::STATE_LAYOUT_LOCKED),
    );
    key_event_state
}
