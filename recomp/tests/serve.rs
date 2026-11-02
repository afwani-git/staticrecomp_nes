//! `recomp serve`: header COOP/COEP wajib ada (tanpa itu SharedArrayBuffer tidak tersedia di
//! browser), tipe konten benar, dan path di luar folder tidak bisa dibaca.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Jalankan server pada port acak dan tunggu sampai ia melapor siap.
fn start(dir: &std::path::Path) -> (Server, u16) {
    let port = 20000 + (std::process::id() % 20000) as u16;
    let mut child = Command::new(env!("CARGO_BIN_EXE_recomp"))
        .args(["serve", "--port", &port.to_string(), "--dir"])
        .arg(dir)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready).unwrap();
    assert!(ready.contains("Melayani"), "server tidak melapor siap: {ready:?}");
    (Server(child), port)
}

fn get(port: u16, path: &str) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    (head.to_string(), body.to_string())
}

#[test]
fn header_isolasi_tipe_konten_dan_batas_folder() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("serve_dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(dir.join("web/index.html"), "<p>halo</p>").unwrap();
    std::fs::write(dir.join("web/worker.js"), "1").unwrap();
    std::fs::write(dir.join("rahasia.txt"), "jangan bocor").unwrap();
    let (_server, port) = start(&dir.join("web"));

    let (head, body) = get(port, "/index.html");
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("Cross-Origin-Opener-Policy: same-origin"), "{head}");
    assert!(head.contains("Cross-Origin-Embedder-Policy: require-corp"), "{head}");
    assert!(head.contains("text/html"), "{head}");
    assert_eq!(body, "<p>halo</p>");

    let (head, _) = get(port, "/");
    assert!(head.starts_with("HTTP/1.1 200"), "/ harus melayani index.html: {head}");

    let (head, _) = get(port, "/worker.js");
    assert!(head.contains("javascript"), "worker.js harus dilayani sebagai JavaScript: {head}");

    // Keluar dari folder yang dilayani harus ditolak, termasuk lewat persen-encoding.
    for path in ["/../rahasia.txt", "/%2e%2e/rahasia.txt", "/a/../../rahasia.txt", "/tidak-ada"] {
        let (head, body) = get(port, path);
        assert!(head.starts_with("HTTP/1.1 404"), "{path}: {head}");
        assert!(!body.contains("jangan bocor"), "{path} membocorkan isi berkas");
    }
}
