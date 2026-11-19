// Ours: the text input channel as the native host has it, in place of Flutter's method channel and its JSON codec.
// TextInputPlugin's TextInputClient calls go out to the window's Rust through it (src/platform/windows/text_input.cc),
// and TextInput's come in to its methods, decoded there.

#ifndef FLUTTER_SHELL_PLATFORM_WINDOWS_TEXT_INPUT_CHANNEL_H_
#define FLUTTER_SHELL_PLATFORM_WINDOWS_TEXT_INPUT_CHANNEL_H_

extern "C" {

struct TextInputChannel {
  void* context;
  // TextInputClient.updateEditingState: the text as UTF-8, its offsets UTF-16 units, -1 where there's none.
  void (*update_editing_state)(void* context,
                               int client,
                               const char* text,
                               int selection_base,
                               int selection_extent,
                               int composing_base,
                               int composing_extent);
  // TextInputClient.performAction.
  void (*perform_action)(void* context, int client, const char* action);
};

}  // extern "C"

#endif  // FLUTTER_SHELL_PLATFORM_WINDOWS_TEXT_INPUT_CHANNEL_H_
