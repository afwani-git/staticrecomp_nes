// Tes web tanpa browser: worker.js dijalankan di worker_threads dengan SharedArrayBuffer nyata.
// Pemakaian: node web_smoke.mjs <nes.wasm>   (nestest dengan menu; Start menjalankan semua tes)
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { Worker } from 'node:worker_threads';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const here = path.dirname(fileURLToPath(import.meta.url));
const webDir = path.join(here, '..', '..', '..', 'web');
const fail = (m) => { console.error('GAGAL:', m); process.exit(1); };
// Pengaman: tes ini tidak boleh pernah menggantung.
setTimeout(() => fail('melewati batas waktu 30 detik'), 30000).unref();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// 1. Skrip inline di index.html harus valid secara sintaks.
const html = readFileSync(path.join(webDir, 'index.html'), 'utf8');
const script = /<script>([\s\S]*)<\/script>/.exec(html)?.[1];
if (!script) fail('tidak ada <script> di index.html');
try { new vm.Script(script); } catch (e) { fail(`sintaks index.html: ${e.message}`); }

// 2. Worker: kecepatan, input lewat SharedArrayBuffer, dan berhenti atas permintaan.
const sab = new SharedArrayBuffer(16);
const i32 = new Int32Array(sab);
// Ring buffer suara tanpa pembaca: harus terisi sampai batas latensi (~100 ms) lalu berhenti.
const AUDIO_RATE = 48000;
const audioSab = new SharedArrayBuffer(8 + 4 * 16384);
const audioIdx = new Int32Array(audioSab, 0, 2);
const worker = new Worker(path.join(here, 'node_worker_shim.mjs'), {
  workerData: { workerPath: path.join(webDir, 'worker.js') },
});
const hashes = [];
const times = [];
let fps = null, stoppedAt = null, error = null;
worker.on('message', (m) => {
  if (m.type === 'frame') {
    if (m.pixels.byteLength !== 256 * 240 * 4) error = `ukuran frame ${m.pixels.byteLength}`;
    times.push(performance.now());
    hashes.push(createHash('sha256').update(Buffer.from(m.pixels)).digest('hex').slice(0, 12));
  } else if (m.type === 'fps') fps = m.fps;
  else if (m.type === 'stopped') stoppedAt = performance.now();
  else if (m.type === 'fault' || m.type === 'error') error = JSON.stringify(m);
});
worker.on('error', (e) => { error = String(e); });
const wasm = readFileSync(process.argv[2]);
const t0 = performance.now();
worker.postMessage({
  type: 'start',
  wasm: wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength),
  sab,
  audio: { sab: audioSab, rate: AUDIO_RATE },
});

await sleep(700);
const menuFrame = hashes.at(-1);
Atomics.store(i32, 0, 1 << 3); // tahan Start
await sleep(200);
Atomics.store(i32, 0, 0);
await sleep(2400);
const elapsed = (performance.now() - t0) / 1000;
const frames = hashes.length;

const stopRequested = performance.now();
Atomics.store(i32, 3, 1);
await sleep(300);
await worker.terminate();

if (error) fail(error);
const rate = frames / elapsed;
const distinct = new Set(hashes).size;
if (rate < 50 || rate > 70) fail(`kecepatan ${rate.toFixed(1)} fps di luar 50-70`);
if (!fps || fps < 50 || fps > 70) fail(`pesan fps tidak wajar: ${fps}`);
if (hashes.at(-1) === menuFrame) fail('layar tidak berubah setelah Start ditekan');
if (distinct < 3) fail(`hanya ${distinct} layar berbeda`);
if (stoppedAt === null) fail('worker tidak berhenti saat diminta');
const audioFill = Atomics.load(audioIdx, 1) - Atomics.load(audioIdx, 0);
if (audioFill !== Math.floor(AUDIO_RATE * 0.1) - 1 && audioFill !== Math.floor(AUDIO_RATE * 0.1))
  fail(`ring suara terisi ${audioFill} sampel, seharusnya ~${AUDIO_RATE * 0.1}`);
if (stoppedAt - stopRequested > 250) fail(`worker berhenti terlalu lama: ${(stoppedAt - stopRequested).toFixed(0)} ms`);

console.log(`OK fps=${rate.toFixed(1)} frames=${frames} layar_berbeda=${distinct} suara=${audioFill} berhenti=${(stoppedAt - stopRequested).toFixed(0)}ms`);
