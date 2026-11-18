// The key names from Zed's GPUI (github.com/zed-industries/zed, crates/gpui_linux/src/linux/platform.rs,
// `keystroke_from_xkb`, `get_xkb_compose_state` and `keystroke_underlying_dead_key`, and x11/client.rs,
// `xkb_state_for_key_event`) at commit
// 1043f4a86ab911a381d9bfe45cf6b36d3cdcc89a, named here as the DOM's `KeyboardEvent.key` names them, which the
// renderer's `keydown` and the inspector speak.
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
        Keysym::Multi_key => "Compose",
        sym if (0xfe50..=0xfe93).contains(&sym.raw()) => "Dead",
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

/// The compose table for the locale, as libxkbcommon finds it (`LC_ALL`, `LC_CTYPE`, `LANG`, then `C`'s), which dead
/// keys and the Compose key type accented letters and symbols by, as GTK's and GPUI's do without an input method.
pub fn compose(context: &xkb::Context) -> Option<xkb::compose::State> {
    let locales = ["LC_ALL", "LC_CTYPE", "LANG"].into_iter().filter_map(|name| std::env::var_os(name).filter(|locale| !locale.is_empty()));
    locales.chain([std::ffi::OsString::from("C")]).find_map(|locale| {
        let table = xkb::compose::Table::new_from_locale(context, &locale, xkb::compose::COMPILE_NO_FLAGS).ok()?;
        Some(xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS))
    })
}

/// What a dead key shows while it waits for the letter it accents, as the DOM's dead keys are drawn
/// (developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_key_values#dead_keycodes_for_linux).
pub fn dead_key(keysym: Keysym) -> Option<String> {
    match keysym {
        Keysym::dead_grave => Some("`".to_owned()),
        Keysym::dead_acute => Some("´".to_owned()),
        Keysym::dead_circumflex => Some("^".to_owned()),
        Keysym::dead_tilde => Some("~".to_owned()),
        Keysym::dead_macron => Some("¯".to_owned()),
        Keysym::dead_breve => Some("˘".to_owned()),
        Keysym::dead_abovedot => Some("˙".to_owned()),
        Keysym::dead_diaeresis => Some("¨".to_owned()),
        Keysym::dead_abovering => Some("˚".to_owned()),
        Keysym::dead_doubleacute => Some("˝".to_owned()),
        Keysym::dead_caron => Some("ˇ".to_owned()),
        Keysym::dead_cedilla => Some("¸".to_owned()),
        Keysym::dead_ogonek => Some("˛".to_owned()),
        Keysym::dead_iota => Some("ͅ".to_owned()),
        Keysym::dead_voiced_sound => Some("゙".to_owned()),
        Keysym::dead_semivoiced_sound => Some("゚".to_owned()),
        Keysym::dead_belowdot => Some("̣̣".to_owned()),
        Keysym::dead_hook => Some("̡".to_owned()),
        Keysym::dead_horn => Some("̛".to_owned()),
        Keysym::dead_stroke => Some("̶̶".to_owned()),
        Keysym::dead_abovecomma => Some("̓̓".to_owned()),
        Keysym::dead_abovereversedcomma => Some("ʽ".to_owned()),
        Keysym::dead_doublegrave => Some("̏".to_owned()),
        Keysym::dead_belowring => Some("˳".to_owned()),
        Keysym::dead_belowmacron => Some("̱".to_owned()),
        Keysym::dead_belowcircumflex => Some("ꞈ".to_owned()),
        Keysym::dead_belowtilde => Some("̰".to_owned()),
        Keysym::dead_belowbreve => Some("̮".to_owned()),
        Keysym::dead_belowdiaeresis => Some("̤".to_owned()),
        Keysym::dead_invertedbreve => Some("̯".to_owned()),
        Keysym::dead_belowcomma => Some("̦".to_owned()),
        Keysym::dead_currency => None,
        Keysym::dead_lowline => None,
        Keysym::dead_aboveverticalline => None,
        Keysym::dead_belowverticalline => None,
        Keysym::dead_longsolidusoverlay => None,
        Keysym::dead_a => None,
        Keysym::dead_A => None,
        Keysym::dead_e => None,
        Keysym::dead_E => None,
        Keysym::dead_i => None,
        Keysym::dead_I => None,
        Keysym::dead_o => None,
        Keysym::dead_O => None,
        Keysym::dead_u => None,
        Keysym::dead_U => None,
        Keysym::dead_small_schwa => Some("ə".to_owned()),
        Keysym::dead_capital_schwa => Some("Ə".to_owned()),
        Keysym::dead_greek => None,
        _ => None,
    }
}
