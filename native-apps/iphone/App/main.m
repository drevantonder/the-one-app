// The app starts in its Rust (src/lib.rs `run`, which tauri::mobile_entry_point calls start_app).
void start_app(void);

int main(int argc, char *argv[]) {
    start_app();
    return 0;
}
