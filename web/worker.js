// Worker: menjalankan nes.wasm seperti program native (loop tanpa henti) di thread terpisah.
// Thread utama mengirim {type:'start', wasm: ArrayBuffer, sab: SharedArrayBuffer,
//   audio?: {sab: SharedArrayBuffer, rate}} (ring buffer suara, lihat audio-worklet.js).
// Layout SharedArrayBuffer (Int32Array): [0] tombol, [2] sel tidur (Atomics.wait), [3] minta berhenti.
// Pesan ke thread utama: {type:'frame', pixels}, {type:'fps', fps}, {type:'interp', pc, target}
// (lompatan dinamis yang tidak ada di TOML, dijalankan interpreter), {type:'fault', ...}, {type:'stopped'}.
'use strict';

const W = 256;
const H = 240;
const FRAME_MS = 1000 / 60.0988; // NTSC

class Stop extends Error {}

self.onmessage = async (event) => {
  const msg = event.data;
  if (msg.type !== 'start') return;

  const i32 = new Int32Array(msg.sab);
  let memory = null;
  let fault = null;
  let frames = 0;
  let next = performance.now();
  let statStart = next;
  const seenInterp = new Set();

  // Ring buffer suara (opsional). Latensi dibatasi ~100 ms: sampel berlebih dibuang.
  const ring = msg.audio
    ? { idx: new Int32Array(msg.audio.sab, 0, 2), data: new Float32Array(msg.audio.sab, 8), rate: msg.audio.rate }
    : null;
  const maxFill = ring ? Math.floor(ring.rate * 0.1) : 0;

  const env = {
    buttons: () => Atomics.load(i32, 0),
    present: (ptr) => {
      // Salin layar (memori WASM terus dipakai game), kirim tanpa menyalin lagi (transfer).
      const pixels = memory.buffer.slice(ptr, ptr + W * H * 4);
      postMessage({ type: 'frame', pixels }, [pixels]);
      frames++;

      // Atur kecepatan: tidur sampai waktu frame berikutnya, tanpa membakar CPU.
      next += FRAME_MS;
      const now = performance.now();
      const wait = next - now;
      if (wait > 0.5) Atomics.wait(i32, 2, 0, wait);
      else if (wait < -100) next = now; // tertinggal jauh (tab disembunyikan, dll.): jangan mengebut
      if (now - statStart >= 1000) {
        postMessage({ type: 'fps', fps: (frames * 1000) / (now - statStart) });
        frames = 0;
        statStart = now;
      }
      if (Atomics.load(i32, 3)) throw new Stop();
    },
    audio: (ptr, count) => {
      if (!ring) return;
      const src = new Float32Array(memory.buffer, ptr, count);
      const n = ring.data.length;
      const r = Atomics.load(ring.idx, 0);
      let w = Atomics.load(ring.idx, 1);
      const fill = (w - r + n) % n;
      const k = Math.max(0, Math.min(count, n - 1 - fill, maxFill - fill));
      for (let i = 0; i < k; i++) {
        ring.data[w] = src[i];
        w = (w + 1) % n;
      }
      Atomics.store(ring.idx, 1, w);
    },
    interp: (pc, target) => {
      // Satu pesan per pasangan unik; jumlahnya kecil karena TOML makin lengkap seiring waktu.
      const key = pc * 65536 + target;
      if (seenInterp.has(key)) return;
      seenInterp.add(key);
      postMessage({ type: 'interp', pc, target });
    },
    fault: (pc, target, kind) => {
      fault = { pc, target, kind };
    },
  };

  try {
    const { instance } = await WebAssembly.instantiate(msg.wasm, { env });
    memory = instance.exports.memory;
    if (ring) instance.exports.nes_set_sample_rate(ring.rate);
    instance.exports.nes_run();
  } catch (e) {
    if (fault) postMessage({ type: 'fault', ...fault });
    else if (!(e instanceof Stop)) postMessage({ type: 'error', message: String(e) });
  }
  postMessage({ type: 'stopped' });
};
