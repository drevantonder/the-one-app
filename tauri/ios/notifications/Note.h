// The notification extension's Rust (tauri/note): a pushed note opened with the device key → JSON.
char *one_open_note(const unsigned char *secret, const char *pushed);
void one_free(char *s);
