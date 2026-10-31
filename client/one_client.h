// The client library's C interface (client/src/lib.rs). Every call returns a JSON string,
// {"ok": ...} or {"error": "..."}, which the caller frees with one_free. Calls block: make them off the main thread.
#include <stdint.h>

char *one_start(const char *dir, const char *name);
char *one_join(const char *code, const char *name);
char *one_request(const char *method, const char *path, const char *body);
char *one_next_event(uint64_t timeout_ms);
void one_free(char *s);
