//! Server dev kecil untuk halaman web.
//!
//! SharedArrayBuffer (dipakai untuk tombol dan pengaturan 60 fps di worker) hanya tersedia bila
//! halaman "cross-origin isolated", yang butuh header COOP/COEP. Server statis biasa tidak
//! mengirimnya, jadi perintah ini menambahkannya. Hanya GET/HEAD, satu permintaan per koneksi.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

const SECURITY_HEADERS: &str = "Cross-Origin-Opener-Policy: same-origin\r\n\
     Cross-Origin-Embedder-Policy: require-corp\r\n\
     Cache-Control: no-store\r\n\
     Connection: close\r\n";

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Ubah path URL menjadi path di bawah `root`; None bila mencoba keluar dari `root`.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let url_path = url_path.split(['?', '#']).next().unwrap_or("");
    let mut rel = PathBuf::new();
    for part in percent_decode(url_path).split('/') {
        match Path::new(part).components().next() {
            None | Some(Component::CurDir) => {}
            Some(Component::Normal(p)) if Path::new(part).components().count() == 1 => rel.push(p),
            _ => return None, // "..", akar, atau awalan drive
        }
    }
    let mut full = root.join(rel);
    if full.is_dir() {
        full.push("index.html");
    }
    Some(full)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8], head_only: bool) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{SECURITY_HEADERS}\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    if !head_only {
        let _ = stream.write_all(body);
    }
}

fn handle(mut stream: TcpStream, root: &Path) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else { return };
    let head_only = method == "HEAD";
    if method != "GET" && !head_only {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain; charset=utf-8", b"metode tidak didukung", false);
    }
    let Some(path) = resolve(root, target) else {
        return respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "tidak ditemukan".as_bytes(), head_only);
    };
    match std::fs::read(&path) {
        Ok(body) => respond(&mut stream, "200 OK", content_type(&path), &body, head_only),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "tidak ditemukan".as_bytes(), head_only)
        }
        Err(_) => respond(&mut stream, "500 Internal Server Error", "text/plain; charset=utf-8", "galat server".as_bytes(), head_only),
    }
}

pub fn run(host: &str, port: u16, dir: &Path) -> Result<()> {
    let root = std::fs::canonicalize(dir).with_context(|| format!("folder tidak ditemukan: {}", dir.display()))?;
    let listener = TcpListener::bind((host, port)).with_context(|| format!("gagal membuka {host}:{port}"))?;
    println!("Melayani {} di http://{host}:{port} (Ctrl+C untuk berhenti)", root.display());
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        std::thread::spawn(move || handle(stream, &root));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_menolak_keluar_dari_root() {
        let root = Path::new("/srv/web");
        assert_eq!(resolve(root, "/worker.js"), Some(PathBuf::from("/srv/web/worker.js")));
        assert_eq!(resolve(root, "/a/b.js?x=1"), Some(PathBuf::from("/srv/web/a/b.js")));
        assert_eq!(resolve(root, "/../Cargo.toml"), None);
        assert_eq!(resolve(root, "/%2e%2e/Cargo.toml"), None);
        assert_eq!(resolve(root, "/a/../../etc/passwd"), None);
    }

    #[test]
    fn percent_decode_dasar() {
        assert_eq!(percent_decode("/a%20b"), "/a b");
        assert_eq!(percent_decode("/%zz"), "/%zz");
        assert_eq!(percent_decode("/akhir%2"), "/akhir%2");
    }
}
