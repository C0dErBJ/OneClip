fn main() {
    let _ = prost_build::compile_protos(&["protos/clip_frame.proto"], &["src/"]);
    tauri_build::build()
}
